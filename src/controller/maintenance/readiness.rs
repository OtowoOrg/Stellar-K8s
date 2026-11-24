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

//! Pure readiness / capacity / verification logic for planned maintenance (#1961).
//!
//! These functions hold the decisions the production backend (`plan_controller`) makes
//! from API snapshots, so they can be tested without a cluster:
//!
//! * **prewarm readiness** is judged from replica counts, never assumed because a
//!   scale-up was requested;
//! * **capacity** is checked before any disruptive step;
//! * **post-drain verification** lists every unresolved check so the plan cannot report
//!   success while any remain open.

use super::plan_engine::{ClusterNode, PdbSnapshot, WorkloadPod};
use std::collections::BTreeSet;

/// Annotation recording a Deployment's replica count before maintenance surge.
pub const PREWARM_BASE_ANNOTATION: &str = "stellar.org/maintenance-prewarm-base";

/// Observed Deployment replica state for one workload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentSnapshot {
    pub workload: String,
    pub desired: i32,
    pub ready: i32,
    /// Replica count before surge, if a surge is active.
    pub prewarm_base: Option<i32>,
}

/// Result of evaluating a workload's prewarm state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrewarmDecision {
    /// No Deployment backs this workload (e.g. StatefulSet): nothing to surge here; readiness
    /// of replacements is still verified from pod placement.
    NotManaged,
    /// Surge not requested yet: request it, and report not ready.
    RequestSurge,
    /// Surge requested; waiting for the new replicas to become Ready.
    Waiting { ready: i32, desired: i32 },
    /// All desired (surged) replicas are Ready.
    Ready,
}

/// Decide prewarm state from the Deployments backing `workload`.
pub fn prewarm_decision(deployments: &[&DeploymentSnapshot]) -> PrewarmDecision {
    if deployments.is_empty() {
        return PrewarmDecision::NotManaged;
    }
    if deployments.iter().any(|d| d.prewarm_base.is_none()) {
        return PrewarmDecision::RequestSurge;
    }
    let (ready, desired) = deployments
        .iter()
        .fold((0, 0), |(r, d), s| (r + s.ready, d + s.desired));
    if ready >= desired {
        PrewarmDecision::Ready
    } else {
        PrewarmDecision::Waiting { ready, desired }
    }
}

/// Replica count to restore when releasing a surge (None if no surge is recorded).
pub fn release_target(d: &DeploymentSnapshot) -> Option<i32> {
    d.prewarm_base
}

/// Fraction of desired replicas that are Ready across Deployments (1.0 when nothing is desired).
pub fn availability(deployments: &[DeploymentSnapshot]) -> f64 {
    let desired: i64 = deployments.iter().map(|d| d.desired.max(0) as i64).sum();
    if desired == 0 {
        return 1.0;
    }
    let ready: i64 = deployments
        .iter()
        .map(|d| d.ready.clamp(0, d.desired.max(0)) as i64)
        .sum();
    ready as f64 / desired as f64
}

/// Are there enough Ready replicas **off** `node` to replace every pod of `workload` on it?
/// (One ready pod elsewhere is not enough when five run on the node.)
pub fn replacements_ready_off_node(pods: &[WorkloadPod], workload: &str, node: &str) -> bool {
    let on_node = pods
        .iter()
        .filter(|p| p.workload == workload && p.node == node && !p.daemon_set)
        .count();
    let off_ready = pods
        .iter()
        .filter(|p| p.workload == workload && p.node != node && p.ready)
        .count();
    off_ready >= on_node
}

/// Allocatable capacity of a node.
#[derive(Clone, Debug)]
pub struct NodeCap {
    pub name: String,
    pub cpu_m: u64,
    pub mem_b: u64,
    /// Ready and not cordoned.
    pub schedulable: bool,
}

/// Requests of a scheduled pod.
#[derive(Clone, Debug)]
pub struct PodReq {
    pub node: String,
    pub cpu_m: u64,
    pub mem_b: u64,
    pub daemon_set: bool,
}

/// `Some(reason)` if the pods running on `targets` cannot fit in the free capacity of the
/// schedulable nodes that are **not** being maintained. DaemonSet pods are not displaced.
pub fn capacity_shortfall(
    targets: &[String],
    nodes: &[NodeCap],
    pods: &[PodReq],
) -> Option<String> {
    let target_set: BTreeSet<&str> = targets.iter().map(String::as_str).collect();
    let displaced = pods
        .iter()
        .filter(|p| target_set.contains(p.node.as_str()) && !p.daemon_set)
        .fold((0u64, 0u64), |(c, m), p| (c + p.cpu_m, m + p.mem_b));
    let mut free = (0u64, 0u64);
    for n in nodes
        .iter()
        .filter(|n| n.schedulable && !target_set.contains(n.name.as_str()))
    {
        let used = pods
            .iter()
            .filter(|p| p.node == n.name)
            .fold((0u64, 0u64), |(c, m), p| (c + p.cpu_m, m + p.mem_b));
        free.0 += n.cpu_m.saturating_sub(used.0);
        free.1 += n.mem_b.saturating_sub(used.1);
    }
    if displaced.0 > free.0 || displaced.1 > free.1 {
        Some(format!(
            "insufficient replacement capacity: need {}m cpu / {} MiB for pods on {} node(s), only {}m / {} MiB free on the remaining schedulable nodes",
            displaced.0,
            displaced.1 >> 20,
            target_set.len(),
            free.0,
            free.1 >> 20
        ))
    } else {
        None
    }
}

/// Every check that must hold before maintenance may be declared complete. Empty = verified.
///
/// * no non-DaemonSet pod remains on a drained node, and each drained node is still cordoned;
/// * every Deployment has all desired replicas Ready (replicas available, rescheduled pods ready);
/// * every PodDisruptionBudget is currently satisfied (`currentHealthy >= minAvailable`);
/// * no unready pod is left over from the maintenance on a non-target node.
pub fn post_drain_issues(
    drained: &[String],
    nodes: &[ClusterNode],
    pods: &[WorkloadPod],
    pdbs: &[PdbSnapshot],
    deployments: &[DeploymentSnapshot],
) -> Vec<String> {
    let mut issues = Vec::new();
    for n in drained {
        let left = pods
            .iter()
            .filter(|p| &p.node == n && !p.daemon_set)
            .count();
        if left > 0 {
            issues.push(format!("node {n} still runs {left} workload pod(s)"));
        }
        match nodes.iter().find(|x| &x.name == n) {
            Some(node) if !node.cordoned => issues.push(format!("node {n} is not cordoned")),
            None => issues.push(format!("node {n} no longer exists / is not observable")),
            _ => {}
        }
    }
    for d in deployments {
        if d.ready < d.desired {
            issues.push(format!(
                "workload {}: {}/{} replicas ready",
                d.workload, d.ready, d.desired
            ));
        }
    }
    for p in pdbs {
        if p.current_healthy < p.min_available {
            issues.push(format!(
                "PDB {} (workload {}) unsatisfied: {} healthy < {} required",
                p.name, p.workload, p.current_healthy, p.min_available
            ));
        }
    }
    issues
}

/// Parse a CPU quantity into millicores (`"500m"`, `"2"`, `"1.5"`); invalid => 0.
pub fn cpu_millis(q: &str) -> u64 {
    let q = q.trim();
    if let Some(m) = q.strip_suffix('m') {
        return m.parse().unwrap_or(0);
    }
    q.parse::<f64>()
        .map(|v| (v * 1000.0).round().max(0.0) as u64)
        .unwrap_or(0)
}

/// Parse a memory quantity into bytes (`"64Mi"`, `"2Gi"`, `"1G"`, plain bytes); invalid => 0.
pub fn mem_bytes(q: &str) -> u64 {
    let q = q.trim();
    for (suf, mult) in [
        ("Ki", 1u64 << 10),
        ("Mi", 1 << 20),
        ("Gi", 1 << 30),
        ("Ti", 1 << 40),
        ("k", 1_000),
        ("K", 1_000),
        ("M", 1_000_000),
        ("G", 1_000_000_000),
    ] {
        if let Some(n) = q.strip_suffix(suf) {
            return n
                .parse::<f64>()
                .map(|v| (v * mult as f64).round().max(0.0) as u64)
                .unwrap_or(0);
        }
    }
    q.parse().unwrap_or(0)
}
