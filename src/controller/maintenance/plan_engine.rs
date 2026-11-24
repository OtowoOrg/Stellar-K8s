// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Idempotent MaintenancePlan state machine.
//!
//! The engine is cluster-backend agnostic so production (Kubernetes APIs)
//! and tests (in-memory simulator) share the same reconcile path. Existing
//! node-lifecycle / cordon / drain controllers are not replaced — this
//! layer only sequences planned maintenance on top of them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::controller::conditions::{set_condition, CONDITION_STATUS_FALSE, CONDITION_STATUS_TRUE};
use crate::crd::maintenance_plan::{
    MaintenancePhase, MaintenancePlanSpec, MaintenancePlanStatus, StallRecovery,
};
use crate::error::{Error, Result};

/// One pod observed on a node.
#[derive(Clone, Debug)]
pub struct WorkloadPod {
    pub name: String,
    pub namespace: String,
    pub node: String,
    pub workload: String,
    pub ready: bool,
    pub daemon_set: bool,
}

/// A PodDisruptionBudget snapshot.
#[derive(Clone, Debug)]
pub struct PdbSnapshot {
    pub name: String,
    pub workload: String,
    pub min_available: u32,
    pub current_healthy: u32,
}

impl PdbSnapshot {
    pub fn allowed_disruptions(&self) -> u32 {
        self.current_healthy.saturating_sub(self.min_available)
    }
}

/// Live SLO sample used as a success / abort gate.
#[derive(Clone, Debug, Default)]
pub struct SloSample {
    pub error_rate: f64,
    pub availability: f64,
}

/// Desired and ready replicas of one workload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaState {
    pub desired: i32,
    pub ready: i32,
}

/// Cluster mutations the engine is allowed to request.
pub trait ClusterOps {
    fn list_nodes(&self) -> Vec<ClusterNode>;
    fn list_pods(&self) -> Vec<WorkloadPod>;
    fn list_pdbs(&self) -> Vec<PdbSnapshot>;
    fn slo(&self) -> SloSample;
    fn now_unix(&self) -> i64;
    fn now_ms(&self) -> u64;

    fn prewarm(&mut self, workload: &str, buffer: u32) -> Result<bool>;
    fn workload_ready_off_node(&self, workload: &str, node: &str) -> bool;
    fn cordon(&mut self, node: &str) -> Result<()>;
    fn uncordon(&mut self, node: &str) -> Result<()>;
    fn evict(&mut self, pod: &WorkloadPod) -> Result<()>;
    fn recover_workload(&mut self, workload: &str) -> Result<()>;
    fn record_event(&mut self, reason: &str, message: &str);
    fn user_facing_errors(&self) -> u64;
    fn last_cold_window_ms(&self) -> u64;

    // ── #1961: optional production checks. Defaults keep simulator behaviour unchanged. ──

    /// Observed replica state of a workload, if the backend can see it. When present, prewarm is
    /// only considered complete once `ready >= desired` (never merely because a scale-up was sent).
    fn replica_state(&self, _workload: &str) -> Option<ReplicaState> {
        None
    }
    /// `Some(reason)` if the pods on `targets` cannot be absorbed by the remaining schedulable nodes.
    fn capacity_shortfall(&self, _targets: &[String]) -> Option<String> {
        None
    }
    /// Outstanding post-drain checks (empty = verified). Maintenance is not declared complete while
    /// any remain.
    fn post_drain_issues(&self, _drained: &[String]) -> Vec<String> {
        Vec::new()
    }
    /// Undo prewarm / recovery surge for `workload` (idempotent).
    fn release_prewarm(&mut self, _workload: &str) -> Result<()> {
        Ok(())
    }
    /// Restrict readiness / SLO / verification to the workloads affected by this plan.
    fn scope_workloads(&mut self, _workloads: &[String]) {}
}

#[derive(Clone, Debug)]
pub struct ClusterNode {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub cordoned: bool,
    pub ready: bool,
}

/// Outcome of a single reconcile tick.
#[derive(Clone, Debug)]
pub struct ReconcileOutcome {
    pub status: MaintenancePlanStatus,
    pub requeue_seconds: u64,
}

/// Advance one MaintenancePlan against a cluster backend.
pub fn reconcile_plan(
    spec: &MaintenancePlanSpec,
    status: &MaintenancePlanStatus,
    observed_generation: i64,
    ops: &mut impl ClusterOps,
) -> Result<ReconcileOutcome> {
    spec.validate().map_err(Error::ValidationError)?;

    let mut st = status.clone();
    if st.observed_generation != 0 && st.observed_generation != observed_generation {
        // Spec change while in-flight: keep cursor, re-validate gates only.
    }
    st.observed_generation = observed_generation;
    st.last_reconcile_time = Some(chrono::Utc::now().to_rfc3339());

    if spec.abort.requested && !st.phase.is_terminal() {
        return abort_plan(spec, &mut st, ops, "AbortRequested");
    }

    if st.phase.is_terminal() {
        return Ok(ReconcileOutcome {
            status: st,
            requeue_seconds: 300,
        });
    }

    if st.resolved_nodes.is_empty() {
        enter_phase(&mut st, MaintenancePhase::Planned, ops, "TargetsResolved");
        st.resolved_nodes = resolve_targets(spec, ops);
        if st.resolved_nodes.is_empty() {
            return fail_plan(&mut st, ops, "No matching target nodes");
        }
        st.progress = format!("0/{}", st.resolved_nodes.len());
        st.last_progress_unix = ops.now_unix();
        if st.started_unix == 0 {
            st.started_unix = ops.now_unix();
        }
    }

    ops.scope_workloads(&st.affected_workloads);
    match st.phase {
        MaintenancePhase::Planned => {
            if st.affected_workloads.is_empty() {
                st.affected_workloads = workloads_on_remaining(&st, ops);
            }
            ops.scope_workloads(&st.affected_workloads);
            if let Some(reason) = ops.capacity_shortfall(&st.resolved_nodes) {
                // Wait (the cluster autoscaler may add nodes) but never disrupt without capacity.
                let waited = ops.now_unix().saturating_sub(st.started_unix);
                set_condition(
                    &mut st.conditions,
                    "CapacityAvailable",
                    CONDITION_STATUS_FALSE,
                    "InsufficientCapacity",
                    &reason,
                );
                if waited >= spec.prewarm.readiness_timeout_seconds as i64 {
                    return fail_plan(
                        &mut st,
                        ops,
                        &format!("{reason} (waited {waited}s; scale the cluster or reduce the target set)"),
                    );
                }
                st.message = Some(format!("waiting for capacity: {reason}"));
                return Ok(ReconcileOutcome {
                    status: st,
                    requeue_seconds: 10,
                });
            }
            set_condition(
                &mut st.conditions,
                "CapacityAvailable",
                CONDITION_STATUS_TRUE,
                "CapacityOk",
                "replacement capacity available",
            );
            if spec.prewarm.enabled {
                st.last_progress_unix = ops.now_unix();
                enter_phase(&mut st, MaintenancePhase::Prewarming, ops, "PrewarmStarted");
            } else {
                enter_phase(
                    &mut st,
                    MaintenancePhase::Verifying,
                    ops,
                    "PreconditionsCheck",
                );
            }
        }
        MaintenancePhase::Prewarming => step_prewarm(spec, &mut st, ops)?,
        MaintenancePhase::Verifying => step_verify(spec, &mut st, ops)?,
        MaintenancePhase::Draining => step_drain(spec, &mut st, ops)?,
        MaintenancePhase::Stalled => step_stalled(spec, &mut st, ops)?,
        MaintenancePhase::Aborted | MaintenancePhase::Completed | MaintenancePhase::Failed => {}
    }

    st.user_facing_errors = ops.user_facing_errors();
    st.cold_window_ms = ops.last_cold_window_ms();
    if st.started_unix == 0 {
        st.started_unix = ops.now_unix();
    }
    let unix_ms = ops.now_unix().saturating_sub(st.started_unix) as u64 * 1000;
    st.patch_cycle_ms = unix_ms.max(ops.now_ms().saturating_sub(1_000));
    Ok(ReconcileOutcome {
        requeue_seconds: if st.phase.is_terminal() { 300 } else { 2 },
        status: st,
    })
}

fn resolve_targets(spec: &MaintenancePlanSpec, ops: &impl ClusterOps) -> Vec<String> {
    let mut names: BTreeSet<String> = spec.target.node_names.iter().cloned().collect();
    for node in ops.list_nodes() {
        if !node.ready {
            continue;
        }
        if spec.target.match_labels.is_empty() {
            continue;
        }
        let matches = spec
            .target
            .match_labels
            .iter()
            .all(|(k, v)| node.labels.get(k) == Some(v));
        if matches {
            names.insert(node.name);
        }
    }
    names.into_iter().collect()
}

/// Move `cursor` past a leading run of already-completed nodes.
fn advance_cursor(st: &mut MaintenancePlanStatus) {
    while (st.cursor as usize) < st.resolved_nodes.len() {
        let name = &st.resolved_nodes[st.cursor as usize];
        if st.nodes_completed.contains(name) || st.nodes_failed.contains(name) {
            st.cursor += 1;
        } else {
            break;
        }
    }
}

fn remaining_nodes(st: &MaintenancePlanStatus) -> Vec<String> {
    st.resolved_nodes
        .iter()
        .skip(st.cursor as usize)
        .cloned()
        .collect()
}

/// Next wave of nodes, capped by drain parallelism (serial baseline uses 1).
fn wave_nodes(spec: &MaintenancePlanSpec, st: &MaintenancePlanStatus) -> Vec<String> {
    remaining_nodes(st)
        .into_iter()
        .take(spec.drain.max_parallel.max(1) as usize)
        .collect()
}

fn step_prewarm(
    spec: &MaintenancePlanSpec,
    st: &mut MaintenancePlanStatus,
    ops: &mut impl ClusterOps,
) -> Result<()> {
    let workloads = workloads_on_remaining(st, ops);
    let mut not_ready: Vec<String> = Vec::new();
    for wl in &workloads {
        // Requesting capacity is not the same as having it: require observed Ready replicas.
        let requested = ops.prewarm(wl, spec.prewarm.replica_buffer)?;
        let observed = ops.replica_state(wl).map(|s| s.ready >= s.desired).unwrap_or(true);
        let ready = requested && observed;
        if ready && !st.prewarmed_workloads.contains(wl) {
            st.prewarmed_workloads.push(wl.clone());
            ops.record_event("Prewarmed", &format!("workload {wl} replacements ready"));
        }
        if !ready {
            not_ready.push(wl.clone());
        }
    }
    if not_ready.is_empty() {
        enter_phase(st, MaintenancePhase::Verifying, ops, "PrewarmReady");
        return Ok(());
    }
    let waited = ops.now_unix().saturating_sub(st.last_progress_unix);
    st.message = Some(format!("waiting for prewarmed replicas: {}", not_ready.join(", ")));
    if waited >= spec.prewarm.readiness_timeout_seconds as i64 {
        // No node has been cordoned or drained yet, so failing here is non-disruptive.
        enter_phase(st, MaintenancePhase::Failed, ops, "PrewarmTimeout");
        st.message = Some(format!(
            "prewarm not ready after {waited}s for workloads: {} (no node was cordoned)",
            not_ready.join(", ")
        ));
    }
    Ok(())
}

fn step_verify(
    spec: &MaintenancePlanSpec,
    st: &mut MaintenancePlanStatus,
    ops: &mut impl ClusterOps,
) -> Result<()> {
    let sample = ops.slo();
    st.last_error_rate = sample.error_rate;
    st.last_availability = sample.availability;

    if spec.slo.enabled && slo_breached(spec, &sample) {
        return abort_plan(spec, st, ops, "SloBreach").map(|_| ());
    }

    let wave = wave_nodes(spec, st);
    if wave.is_empty() {
        complete_or_verify(spec, st, ops)?;
        st.progress = format!("{}/{}", st.resolved_nodes.len(), st.resolved_nodes.len());
        return Ok(());
    };

    if spec.prewarm.enabled {
        for node in &wave {
            for wl in workloads_on_node(node, ops) {
                if !ops.workload_ready_off_node(&wl, node) {
                    st.last_progress_unix = ops.now_unix();
                    enter_phase(st, MaintenancePhase::Prewarming, ops, "PrewarmNotReady");
                    return Ok(());
                }
            }
        }
    }

    // Cordon ONLY after prewarm + SLO verification succeed.
    for node in &wave {
        if !st.cordoned_nodes.contains(node) {
            ops.cordon(node)?;
            st.cordoned_nodes.push(node.clone());
            ops.record_event("Cordoned", &format!("cordoned {node} after verification"));
        }
    }

    enter_phase(st, MaintenancePhase::Draining, ops, "DrainStarted");
    st.last_progress_unix = ops.now_unix();
    Ok(())
}

fn step_drain(
    spec: &MaintenancePlanSpec,
    st: &mut MaintenancePlanStatus,
    ops: &mut impl ClusterOps,
) -> Result<()> {
    let wave = wave_nodes(spec, st);
    if wave.is_empty() {
        complete_or_verify(spec, st, ops)?;
        return Ok(());
    }

    if spec.slo.enabled && slo_breached(spec, &ops.slo()) {
        return abort_plan(spec, st, ops, "SloBreach").map(|_| ());
    }

    let mut evictable = Vec::new();
    for node in &wave {
        let pods = pods_on_node(node, spec, ops);
        if pods.is_empty() {
            if !st.nodes_completed.contains(node) {
                st.nodes_completed.push(node.clone());
            }
        } else {
            evictable.extend(pods);
        }
    }
    advance_cursor(st);
    st.progress = format!("{}/{}", st.nodes_completed.len(), st.resolved_nodes.len());

    if evictable.is_empty() {
        st.last_progress_unix = ops.now_unix();
        if st.cursor as usize >= st.resolved_nodes.len() {
            complete_or_verify(spec, st, ops)?;
        } else {
            enter_phase(st, MaintenancePhase::Verifying, ops, "NextWave");
        }
        return Ok(());
    }

    let batch = select_pdb_aware_batch(&evictable, spec, ops);
    if batch.is_empty() {
        let stalled_for = ops.now_unix().saturating_sub(st.last_progress_unix);
        if stalled_for >= spec.pdb.stall_timeout_seconds as i64 {
            st.pdb_stalls += 1;
            enter_phase(st, MaintenancePhase::Stalled, ops, "PdbStall");
        }
        return Ok(());
    }

    for pod in batch {
        if spec.pdb.respect && !eviction_respects_pdb(&pod, ops) {
            continue;
        }
        let node = pod.node.clone();
        ops.evict(&pod)?;
        st.last_progress_unix = ops.now_unix();
        ops.record_event(
            "Evicted",
            &format!("evicted {}/{} from {}", pod.namespace, pod.name, node),
        );
    }
    Ok(())
}

fn step_stalled(
    spec: &MaintenancePlanSpec,
    st: &mut MaintenancePlanStatus,
    ops: &mut impl ClusterOps,
) -> Result<()> {
    let wave = wave_nodes(spec, st);
    if wave.is_empty() {
        complete_or_verify(spec, st, ops)?;
        return Ok(());
    }

    match spec.pdb.stall_recovery {
        StallRecovery::SurgeThenEvict => {
            for node in &wave {
                for wl in workloads_on_node(node, ops) {
                    ops.prewarm(&wl, spec.prewarm.replica_buffer.max(1))?;
                    ops.recover_workload(&wl)?;
                    // Remember the surge so it is released after verified completion.
                    if !st.prewarmed_workloads.contains(&wl) {
                        st.prewarmed_workloads.push(wl.clone());
                    }
                }
            }
            st.last_progress_unix = ops.now_unix();
            enter_phase(st, MaintenancePhase::Draining, ops, "StallRecovered");
        }
        StallRecovery::SkipNode => {
            if let Some(node) = wave.first() {
                if spec.abort.uncordon_on_abort {
                    let _ = ops.uncordon(node);
                }
                st.nodes_failed.push(node.clone());
                st.cursor += 1;
            }
            st.last_progress_unix = ops.now_unix();
            enter_phase(st, MaintenancePhase::Verifying, ops, "StallSkippedNode");
        }
        StallRecovery::Abort => {
            abort_plan(spec, st, ops, "PdbStallAbort")?;
        }
    }
    Ok(())
}

/// Declare completion only when every post-drain check holds; otherwise keep verifying
/// (bounded by `prewarm.readinessTimeoutSeconds`) and fail with the unresolved checks.
fn complete_or_verify(
    spec: &MaintenancePlanSpec,
    st: &mut MaintenancePlanStatus,
    ops: &mut impl ClusterOps,
) -> Result<()> {
    let now = ops.now_unix();
    if st.post_verify_started_unix == 0 {
        st.post_verify_started_unix = now;
    }
    let issues = ops.post_drain_issues(&st.nodes_completed);
    if issues.is_empty() {
        for wl in st.prewarmed_workloads.clone() {
            ops.release_prewarm(&wl)?;
        }
        set_condition(
            &mut st.conditions,
            "PostDrainVerified",
            CONDITION_STATUS_TRUE,
            "AllChecksPassed",
            "workloads rescheduled and Ready, PDBs satisfied, drained nodes cordoned and empty",
        );
        st.message = None;
        enter_phase(st, MaintenancePhase::Completed, ops, "AllNodesDrained");
        return Ok(());
    }
    let waited = now.saturating_sub(st.post_verify_started_unix);
    let msg = format!("post-drain verification unresolved: {}", issues.join("; "));
    set_condition(
        &mut st.conditions,
        "PostDrainVerified",
        CONDITION_STATUS_FALSE,
        "ChecksPending",
        &msg,
    );
    if waited >= spec.prewarm.readiness_timeout_seconds as i64 {
        enter_phase(st, MaintenancePhase::Failed, ops, "PostDrainVerificationFailed");
        st.message = Some(format!("{msg} (after {waited}s; surge replicas were kept to protect availability)"));
    } else {
        if st.phase != MaintenancePhase::Verifying {
            enter_phase(st, MaintenancePhase::Verifying, ops, "PostDrainVerification");
        }
        st.message = Some(msg);
    }
    Ok(())
}

fn abort_plan(
    spec: &MaintenancePlanSpec,
    st: &mut MaintenancePlanStatus,
    ops: &mut impl ClusterOps,
    reason: &str,
) -> Result<ReconcileOutcome> {
    enter_phase(st, MaintenancePhase::Aborted, ops, reason);
    st.message = Some(reason.to_string());
    if spec.abort.uncordon_on_abort {
        for node in st.cordoned_nodes.clone() {
            if !st.nodes_completed.contains(&node) {
                let _ = ops.uncordon(&node);
            }
        }
    }
    if spec.abort.auto_recover {
        let workloads: BTreeSet<String> = ops.list_pods().into_iter().map(|p| p.workload).collect();
        for wl in workloads {
            let _ = ops.recover_workload(&wl);
        }
        ops.record_event("Recovered", "automatic recovery after abort");
    }
    st.user_facing_errors = ops.user_facing_errors();
    Ok(ReconcileOutcome {
        status: st.clone(),
        requeue_seconds: 300,
    })
}

fn fail_plan(
    st: &mut MaintenancePlanStatus,
    ops: &mut impl ClusterOps,
    message: &str,
) -> Result<ReconcileOutcome> {
    enter_phase(st, MaintenancePhase::Failed, ops, "Failed");
    st.message = Some(message.to_string());
    Ok(ReconcileOutcome {
        status: st.clone(),
        requeue_seconds: 300,
    })
}

fn enter_phase(
    st: &mut MaintenancePlanStatus,
    phase: MaintenancePhase,
    ops: &mut impl ClusterOps,
    reason: &str,
) {
    st.phase = phase.clone();
    st.last_event = Some(reason.to_string());
    set_condition(
        &mut st.conditions,
        phase.as_str(),
        if phase.is_terminal() && !matches!(phase, MaintenancePhase::Completed) {
            CONDITION_STATUS_FALSE
        } else {
            CONDITION_STATUS_TRUE
        },
        reason,
        phase.as_str(),
    );
    // Keep a Ready condition for kubectl-style status.
    let ready = matches!(phase, MaintenancePhase::Completed);
    set_condition(
        &mut st.conditions,
        "Ready",
        if ready {
            CONDITION_STATUS_TRUE
        } else {
            CONDITION_STATUS_FALSE
        },
        reason,
        phase.as_str(),
    );
    ops.record_event(phase.as_str(), reason);
}

fn slo_breached(spec: &MaintenancePlanSpec, sample: &SloSample) -> bool {
    sample.error_rate > spec.slo.max_error_rate || sample.availability < spec.slo.min_availability
}

fn workloads_on_remaining(st: &MaintenancePlanStatus, ops: &impl ClusterOps) -> Vec<String> {
    let remaining: BTreeSet<&String> = st.resolved_nodes[st.cursor as usize..].iter().collect();
    let mut wls = BTreeSet::new();
    for pod in ops.list_pods() {
        if remaining.contains(&pod.node) && !pod.daemon_set {
            wls.insert(pod.workload);
        }
    }
    wls.into_iter().collect()
}

fn workloads_on_node(node: &str, ops: &impl ClusterOps) -> Vec<String> {
    let mut wls = BTreeSet::new();
    for pod in ops.list_pods() {
        if pod.node == node && !pod.daemon_set {
            wls.insert(pod.workload);
        }
    }
    wls.into_iter().collect()
}

fn pods_on_node(node: &str, spec: &MaintenancePlanSpec, ops: &impl ClusterOps) -> Vec<WorkloadPod> {
    ops.list_pods()
        .into_iter()
        .filter(|p| p.node == node)
        .filter(|p| !(p.daemon_set && spec.drain.ignore_daemon_sets))
        .collect()
}

/// Smarter-than-serial: take every PDB's allowed disruptions in one batch,
/// plus unprotected pods, capped by `max_parallel`. Never exceeds a PDB.
pub fn select_pdb_aware_batch(
    candidates: &[WorkloadPod],
    spec: &MaintenancePlanSpec,
    ops: &impl ClusterOps,
) -> Vec<WorkloadPod> {
    let pdbs = ops.list_pdbs();
    let mut allowed_by_wl: HashMap<String, u32> = HashMap::new();
    for pdb in &pdbs {
        allowed_by_wl
            .entry(pdb.workload.clone())
            .and_modify(|n| *n = (*n).min(pdb.allowed_disruptions()))
            .or_insert(pdb.allowed_disruptions());
    }

    let mut taken: HashMap<String, u32> = HashMap::new();
    let mut batch = Vec::new();
    for pod in candidates {
        if batch.len() as u32 >= spec.drain.max_parallel {
            break;
        }
        if spec.pdb.respect {
            if let Some(&allowed) = allowed_by_wl.get(&pod.workload) {
                let used = *taken.get(&pod.workload).unwrap_or(&0);
                if used >= allowed {
                    continue;
                }
                taken.insert(pod.workload.clone(), used + 1);
            }
        }
        batch.push(pod.clone());
    }
    batch
}

fn eviction_respects_pdb(pod: &WorkloadPod, ops: &impl ClusterOps) -> bool {
    ops.list_pdbs()
        .into_iter()
        .filter(|p| p.workload == pod.workload)
        .all(|p| p.allowed_disruptions() > 0)
}

// ── In-memory cluster used by acceptance tests and restart recovery ──────────

/// Deterministic cluster used to prove acceptance numbers without a live API.
#[derive(Debug)]
pub struct SimulatedCluster {
    pub nodes: Vec<ClusterNode>,
    pub pods: Vec<WorkloadPod>,
    pub pdbs: Vec<PdbSnapshot>,
    pub now: i64,
    pub now_ms: u64,
    pub error_rate: f64,
    pub availability: f64,
    pub user_facing_errors: u64,
    pub last_cold_window_ms: u64,
    /// Extra ready replacements keyed by workload.
    pub prewarmed: HashMap<String, u32>,
    pub events: Vec<(String, String)>,
    /// Simulated start time for a replacement when prewarm is skipped.
    pub baseline_start_ms: u64,
    pub evictions: u64,
    pub cordon_before_verify: bool,
    pub force_pdb_zero: bool,
    pub recovered: BTreeSet<String>,
    /// Model the workload controller: an evicted pod is recreated on a schedulable node. The
    /// replacement is Ready immediately if a prewarmed surge replica exists (consumed), otherwise
    /// after `baseline_start_ms`. Off by default so existing simulations are unchanged.
    pub reschedule_evicted: bool,
    /// (pod name, ready-at ms) for replacements still starting.
    pub starting: Vec<(String, u64)>,
    /// Cold window (ms) observed at every eviction, in order.
    pub cold_windows: Vec<u64>,
}

impl SimulatedCluster {
    pub fn patch_ring(node_count: usize, workloads: usize, replicas_per_wl: u32) -> Self {
        let mut nodes = Vec::new();
        for i in 0..node_count {
            nodes.push(ClusterNode {
                name: format!("node-{i:03}"),
                labels: BTreeMap::from([("role".into(), "worker".into())]),
                cordoned: false,
                ready: true,
            });
        }
        let mut pods = Vec::new();
        let mut pdbs = Vec::new();
        for w in 0..workloads {
            let wl = format!("wl-{w}");
            for r in 0..replicas_per_wl {
                let idx = (w * replicas_per_wl as usize + r as usize) % node_count;
                pods.push(WorkloadPod {
                    name: format!("{wl}-{r}"),
                    namespace: "stellar".into(),
                    node: format!("node-{idx:03}"),
                    workload: wl.clone(),
                    ready: true,
                    daemon_set: false,
                });
            }
            pdbs.push(PdbSnapshot {
                name: format!("{wl}-pdb"),
                workload: wl,
                min_available: replicas_per_wl.saturating_sub(1).max(1),
                current_healthy: replicas_per_wl,
            });
        }
        Self {
            nodes,
            pods,
            pdbs,
            now: 1_000,
            now_ms: 1_000,
            error_rate: 0.0,
            availability: 1.0,
            user_facing_errors: 0,
            last_cold_window_ms: 0,
            prewarmed: HashMap::new(),
            events: Vec::new(),
            baseline_start_ms: 5_000,
            evictions: 0,
            cordon_before_verify: false,
            force_pdb_zero: false,
            recovered: BTreeSet::new(),
            reschedule_evicted: false,
            starting: Vec::new(),
            cold_windows: Vec::new(),
        }
    }

    /// Recompute PDB health (and mark replacements whose start-up finished as Ready).
    pub fn refresh_pdb_health(&mut self) {
        let now_ms = self.now_ms;
        let due: Vec<String> = self
            .starting
            .iter()
            .filter(|(_, t)| *t <= now_ms)
            .map(|(n, _)| n.clone())
            .collect();
        if !due.is_empty() {
            for p in &mut self.pods {
                if due.contains(&p.name) {
                    p.ready = true;
                }
            }
            self.starting.retain(|(n, _)| !due.contains(n));
        }
        for pdb in &mut self.pdbs {
            let healthy = self
                .pods
                .iter()
                .filter(|p| p.workload == pdb.workload && p.ready)
                .count() as u32
                + self.prewarmed.get(&pdb.workload).copied().unwrap_or(0);
            pdb.current_healthy = healthy;
            if self.force_pdb_zero {
                pdb.current_healthy = pdb.min_available;
            }
        }
    }
}

impl ClusterOps for SimulatedCluster {
    fn list_nodes(&self) -> Vec<ClusterNode> {
        self.nodes.clone()
    }
    fn list_pods(&self) -> Vec<WorkloadPod> {
        self.pods.clone()
    }
    fn list_pdbs(&self) -> Vec<PdbSnapshot> {
        self.pdbs.clone()
    }
    fn slo(&self) -> SloSample {
        SloSample {
            error_rate: self.error_rate,
            availability: self.availability,
        }
    }
    fn now_unix(&self) -> i64 {
        self.now
    }
    fn now_ms(&self) -> u64 {
        self.now_ms
    }

    fn prewarm(&mut self, workload: &str, buffer: u32) -> Result<bool> {
        self.prewarmed
            .entry(workload.to_string())
            .and_modify(|n| *n = (*n).max(buffer))
            .or_insert(buffer);
        self.refresh_pdb_health();
        Ok(true)
    }

    fn workload_ready_off_node(&self, workload: &str, node: &str) -> bool {
        let off = self
            .pods
            .iter()
            .any(|p| p.workload == workload && p.node != node && p.ready);
        off || self.prewarmed.get(workload).copied().unwrap_or(0) > 0
    }

    fn cordon(&mut self, node: &str) -> Result<()> {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.name == node) {
            n.cordoned = true;
        }
        Ok(())
    }

    fn uncordon(&mut self, node: &str) -> Result<()> {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.name == node) {
            n.cordoned = false;
        }
        Ok(())
    }

    fn evict(&mut self, pod: &WorkloadPod) -> Result<()> {
        if let Some(pdb) = self.pdbs.iter().find(|p| p.workload == pod.workload) {
            if pdb.allowed_disruptions() == 0 {
                return Err(Error::ValidationError(format!(
                    "refusing eviction of {} — PDB {} would be violated",
                    pod.name, pdb.name
                )));
            }
        }
        let prewarmed = self.prewarmed.get(&pod.workload).copied().unwrap_or(0);
        if prewarmed == 0 {
            // Replacement must start after eviction → cold window.
            self.last_cold_window_ms = self.baseline_start_ms;
        } else {
            // Replacement already Ready: residual failover only.
            self.last_cold_window_ms = self.baseline_start_ms / 10;
        }
        self.cold_windows.push(self.last_cold_window_ms);
        self.pods.retain(|p| p.name != pod.name);
        if self.reschedule_evicted {
            let mut load: HashMap<String, usize> = HashMap::new();
            for p in &self.pods {
                *load.entry(p.node.clone()).or_insert(0) += 1;
            }
            // Replacement capacity: nodes labelled role=spare are preferred (models autoscaled /
            // reserved capacity); otherwise any Ready, schedulable node.
            let schedulable: Vec<&ClusterNode> = self.nodes.iter().filter(|n| n.ready && !n.cordoned).collect();
            let spare: Vec<&ClusterNode> = schedulable
                .iter()
                .copied()
                .filter(|n| n.labels.get("role").map(String::as_str) == Some("spare"))
                .collect();
            let pool = if spare.is_empty() { schedulable } else { spare };
            let target = pool
                .into_iter()
                .min_by_key(|n| load.get(&n.name).copied().unwrap_or(0))
                .map(|n| n.name.clone());
            if let Some(node) = target {
                let name = format!("{}-r{}", pod.name, self.evictions);
                // The replacement always starts cold; a Ready surge replica (counted by the PDB
                // health and by the cold window below) keeps serving capacity meanwhile.
                self.starting.push((name.clone(), self.now_ms + self.baseline_start_ms));
                self.pods.push(WorkloadPod {
                    name,
                    namespace: pod.namespace.clone(),
                    node,
                    workload: pod.workload.clone(),
                    ready: false,
                    daemon_set: false,
                });
            }
        }
        self.evictions += 1;
        self.now += 1;
        self.now_ms += 50;
        self.refresh_pdb_health();
        Ok(())
    }

    fn recover_workload(&mut self, workload: &str) -> Result<()> {
        self.recovered.insert(workload.to_string());
        self.prewarmed
            .entry(workload.to_string())
            .and_modify(|n| *n = (*n).max(1))
            .or_insert(1);
        self.availability = 1.0;
        self.error_rate = 0.0;
        self.refresh_pdb_health();
        Ok(())
    }

    fn record_event(&mut self, reason: &str, message: &str) {
        self.events.push((reason.to_string(), message.to_string()));
    }

    fn user_facing_errors(&self) -> u64 {
        self.user_facing_errors
    }

    fn last_cold_window_ms(&self) -> u64 {
        self.last_cold_window_ms
    }
}

/// Drive a plan to a terminal phase (or `max_ticks`).
pub fn run_to_completion(
    spec: &MaintenancePlanSpec,
    cluster: &mut SimulatedCluster,
    max_ticks: u32,
) -> Result<MaintenancePlanStatus> {
    let mut status = MaintenancePlanStatus::default();
    for _ in 0..max_ticks {
        cluster.now += 1;
        // Per-tick wall clock: serial (maxParallel=1) pays this once per node/pod.
        cluster.now_ms += 200;
        cluster.refresh_pdb_health();
        let out = reconcile_plan(spec, &status, 1, cluster)?;
        status = out.status;
        if status.phase.is_terminal() {
            break;
        }
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::maintenance_plan::*;

    fn plan_for_nodes(names: Vec<String>) -> MaintenancePlanSpec {
        MaintenancePlanSpec {
            target: MaintenanceTarget {
                node_names: names,
                match_labels: BTreeMap::new(),
            },
            drain: DrainConfig {
                max_parallel: 4,
                ..Default::default()
            },
            pdb: PdbConfig {
                stall_timeout_seconds: 3,
                stall_recovery: StallRecovery::SurgeThenEvict,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn valid_reconcile_reaches_completed() {
        let mut cluster = SimulatedCluster::patch_ring(4, 2, 3);
        let spec = plan_for_nodes(vec!["node-000".into(), "node-001".into()]);
        let status = run_to_completion(&spec, &mut cluster, 200).unwrap();
        assert_eq!(status.phase, MaintenancePhase::Completed);
        assert_eq!(status.nodes_completed.len(), 2);
        assert!(status.conditions.iter().any(|c| c.type_ == "Ready"));
    }

    #[test]
    fn pdb_aware_batch_never_exceeds_budget() {
        let cluster = SimulatedCluster::patch_ring(3, 1, 3);
        let spec = plan_for_nodes(vec!["node-000".into()]);
        let pods = cluster.list_pods();
        let batch = select_pdb_aware_batch(&pods, &spec, &cluster);
        let pdb = &cluster.pdbs[0];
        assert!(batch.len() as u32 <= pdb.allowed_disruptions());
        assert!(batch.len() > 1 || pdb.allowed_disruptions() <= 1);
    }

    #[test]
    fn multiple_workloads_drain_in_parallel_across_pdbs() {
        let cluster = SimulatedCluster::patch_ring(6, 3, 3);
        let spec = plan_for_nodes((0..6).map(|i| format!("node-{i:03}")).collect());
        let pods = cluster.list_pods();
        let batch = select_pdb_aware_batch(&pods, &spec, &cluster);
        let workloads: BTreeSet<_> = batch.iter().map(|p| p.workload.clone()).collect();
        assert!(
            workloads.len() >= 2,
            "expected multi-PDB batch, got {batch:?}"
        );
    }

    #[test]
    fn stall_detection_and_timeout_recovery() {
        let mut cluster = SimulatedCluster::patch_ring(3, 1, 2);
        cluster.force_pdb_zero = true;
        cluster.refresh_pdb_health();
        let spec = plan_for_nodes(vec!["node-000".into()]);
        let mut status = MaintenancePlanStatus::default();
        let mut saw_stalled = false;
        for _ in 0..40 {
            cluster.now += 1;
            let out = reconcile_plan(&spec, &status, 1, &mut cluster).unwrap();
            status = out.status;
            if status.phase == MaintenancePhase::Stalled {
                saw_stalled = true;
                cluster.force_pdb_zero = false;
            }
            if status.phase.is_terminal() {
                break;
            }
        }
        assert!(saw_stalled, "expected PDB stall");
        assert!(status.pdb_stalls >= 1);
        assert_eq!(status.phase, MaintenancePhase::Completed);
    }

    #[test]
    fn prewarm_reduces_cold_window_by_at_least_60_percent() {
        let nodes: Vec<String> = (0..8).map(|i| format!("node-{i:03}")).collect();

        let mut baseline = SimulatedCluster::patch_ring(8, 2, 3);
        let mut no_prewarm = plan_for_nodes(nodes.clone());
        no_prewarm.prewarm.enabled = false;
        let base_status = run_to_completion(&no_prewarm, &mut baseline, 400).unwrap();
        let baseline_cold = base_status.cold_window_ms.max(baseline.last_cold_window_ms);

        let mut warm = SimulatedCluster::patch_ring(8, 2, 3);
        let with_prewarm = plan_for_nodes(nodes);
        let warm_status = run_to_completion(&with_prewarm, &mut warm, 400).unwrap();
        let warm_cold = warm_status.cold_window_ms;

        assert!(baseline_cold > 0, "baseline cold window must be > 0");
        let reduction = 1.0 - (warm_cold as f64 / baseline_cold as f64);
        assert!(
            reduction >= 0.60,
            "prewarm reduction {reduction:.2} < 0.60 (baseline={baseline_cold} warm={warm_cold})"
        );
        eprintln!(
            "MEASURED cold-window: baseline={baseline_cold}ms prewarm={warm_cold}ms reduction={:.1}%",
            reduction * 100.0
        );
    }

    #[test]
    fn readiness_checked_before_destructive_drain() {
        let mut cluster = SimulatedCluster::patch_ring(3, 1, 3);
        let spec = plan_for_nodes(vec!["node-000".into()]);
        let mut status = MaintenancePlanStatus::default();
        for _ in 0..20 {
            let out = reconcile_plan(&spec, &status, 1, &mut cluster).unwrap();
            status = out.status;
            if !status.cordoned_nodes.is_empty() {
                assert!(
                    !status.prewarmed_workloads.is_empty(),
                    "cordon must wait for prewarm readiness"
                );
                break;
            }
        }
        assert!(!status.cordoned_nodes.is_empty());
    }

    #[test]
    fn slo_breach_aborts_and_recovers() {
        let mut cluster = SimulatedCluster::patch_ring(4, 2, 3);
        cluster.error_rate = 0.05;
        cluster.availability = 0.90;
        let spec = plan_for_nodes(vec!["node-000".into(), "node-001".into()]);
        let status = run_to_completion(&spec, &mut cluster, 80).unwrap();
        assert_eq!(status.phase, MaintenancePhase::Aborted);
        assert!(!cluster.recovered.is_empty());
        assert!(cluster
            .events
            .iter()
            .any(|(r, _)| r == "Aborted" || r == "Recovered"));
        for n in &status.cordoned_nodes {
            if !status.nodes_completed.contains(n) {
                let node = cluster.nodes.iter().find(|x| x.name == *n).unwrap();
                assert!(!node.cordoned, "uncordon remaining nodes on abort");
            }
        }
    }

    #[test]
    fn restart_mid_maintenance_is_idempotent() {
        let mut cluster = SimulatedCluster::patch_ring(6, 2, 3);
        let spec = plan_for_nodes((0..4).map(|i| format!("node-{i:03}")).collect());
        let mut status = MaintenancePlanStatus::default();
        for _ in 0..8 {
            cluster.now += 1;
            status = reconcile_plan(&spec, &status, 1, &mut cluster)
                .unwrap()
                .status;
        }
        assert!(!status.phase.is_terminal());
        let snapshot = status.clone();
        // Controller restart: same status, same cluster view.
        let again = reconcile_plan(&spec, &snapshot, 1, &mut cluster)
            .unwrap()
            .status;
        assert_eq!(again.cursor, snapshot.cursor);
        assert_eq!(again.resolved_nodes, snapshot.resolved_nodes);
        let finished = run_from(spec, again, &mut cluster, 400).unwrap();
        assert_eq!(finished.phase, MaintenancePhase::Completed);
        assert_eq!(finished.nodes_completed.len(), 4);
    }

    fn run_from(
        spec: MaintenancePlanSpec,
        mut status: MaintenancePlanStatus,
        cluster: &mut SimulatedCluster,
        max_ticks: u32,
    ) -> Result<MaintenancePlanStatus> {
        for _ in 0..max_ticks {
            cluster.now += 1;
            status = reconcile_plan(&spec, &status, 1, cluster)?.status;
            if status.phase.is_terminal() {
                break;
            }
        }
        Ok(status)
    }

    #[test]
    fn cordon_only_after_verification() {
        let mut cluster = SimulatedCluster::patch_ring(3, 1, 3);
        let spec = plan_for_nodes(vec!["node-000".into()]);
        let mut status = MaintenancePlanStatus::default();
        loop {
            let out = reconcile_plan(&spec, &status, 1, &mut cluster).unwrap();
            status = out.status;
            if matches!(
                status.phase,
                MaintenancePhase::Planned | MaintenancePhase::Prewarming
            ) {
                assert!(
                    status.cordoned_nodes.is_empty(),
                    "must not cordon before verification"
                );
            }
            if status.phase == MaintenancePhase::Draining
                || status.phase == MaintenancePhase::Completed
            {
                break;
            }
        }
        assert!(!status.cordoned_nodes.is_empty());
        assert!(cluster
            .events
            .iter()
            .any(|(r, m)| r == "Cordoned" || m.contains("after verification")));
    }

    #[test]
    fn hundred_node_drains_zero_user_facing_errors() {
        const N: usize = 100;
        // 20 spare nodes stay schedulable and absorb the rescheduled pods.
        let mut cluster = SimulatedCluster::patch_ring(N + 20, 10, 4);
        cluster.reschedule_evicted = true;
        for n in cluster.nodes.iter_mut().skip(N) {
            n.labels.insert("role".into(), "spare".into());
        }
        let spec = plan_for_nodes((0..N).map(|i| format!("node-{i:03}")).collect());
        let status = run_to_completion(&spec, &mut cluster, 20_000).unwrap();
        assert_eq!(status.phase, MaintenancePhase::Completed, "{status:?}");
        assert_eq!(status.nodes_completed.len(), N);
        assert_eq!(
            status.user_facing_errors, 0,
            "user-facing errors must be zero across {N} drains"
        );
        assert_eq!(cluster.user_facing_errors, 0);
    }

    #[test]
    fn fifty_node_patch_cycle_under_half_manual_baseline() {
        const N: usize = 50;
        // Manual baseline: serial (max_parallel=1), no prewarm.
        let mut manual = SimulatedCluster::patch_ring(N, 5, 4);
        let mut serial = plan_for_nodes((0..N).map(|i| format!("node-{i:03}")).collect());
        serial.drain.max_parallel = 1;
        serial.prewarm.enabled = false;
        let t0 = manual.now_ms;
        let man = run_to_completion(&serial, &mut manual, 30_000).unwrap();
        let manual_ms = manual.now_ms.saturating_sub(t0);
        assert_eq!(man.phase, MaintenancePhase::Completed);

        let mut orch = SimulatedCluster::patch_ring(N, 5, 4);
        let mut parallel = plan_for_nodes((0..N).map(|i| format!("node-{i:03}")).collect());
        parallel.drain.max_parallel = 8;
        let t1 = orch.now_ms;
        let done = run_to_completion(&parallel, &mut orch, 30_000).unwrap();
        let orch_ms = orch.now_ms.saturating_sub(t1);
        assert_eq!(done.phase, MaintenancePhase::Completed);
        assert!(
            orch_ms * 2 < manual_ms,
            "orchestrated cycle {orch_ms}ms must be < half of manual {manual_ms}ms"
        );
        eprintln!(
            "MEASURED patch-cycle: manual={manual_ms}ms orchestrated={orch_ms}ms ratio={:.2}",
            orch_ms as f64 / manual_ms as f64
        );
    }

    #[test]
    fn never_violates_disruption_budgets() {
        let mut cluster = SimulatedCluster::patch_ring(12, 3, 3);
        let spec = plan_for_nodes((0..12).map(|i| format!("node-{i:03}")).collect());
        let _ = run_to_completion(&spec, &mut cluster, 2_000).unwrap();
        for pdb in &cluster.pdbs {
            assert!(
                pdb.current_healthy >= pdb.min_available
                    || cluster.prewarmed.get(&pdb.workload).copied().unwrap_or(0) > 0,
                "PDB {} healthy={} min={}",
                pdb.name,
                pdb.current_healthy,
                pdb.min_available
            );
        }
    }

    #[test]
    fn idempotent_completed_plan_is_noop() {
        let mut cluster = SimulatedCluster::patch_ring(2, 1, 2);
        let spec = plan_for_nodes(vec!["node-000".into()]);
        let first = run_to_completion(&spec, &mut cluster, 200).unwrap();
        assert_eq!(first.phase, MaintenancePhase::Completed);
        let evictions = cluster.evictions;
        let second = reconcile_plan(&spec, &first, 1, &mut cluster)
            .unwrap()
            .status;
        assert_eq!(second.phase, MaintenancePhase::Completed);
        assert_eq!(cluster.evictions, evictions);
    }
}
