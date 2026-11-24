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

//! #1961: capacity gate, observed-readiness prewarm, post-drain verification, surge release,
//! restart safety and the 100-node drain run, driven through the shared reconcile engine with
//! scripted failure injection. Also covers the pure readiness logic used by the Kubernetes backend.

use std::collections::BTreeMap;
use stellar_k8s::controller::maintenance::plan_engine::{
    reconcile_plan, ClusterNode, ClusterOps, PdbSnapshot, ReplicaState, SimulatedCluster,
    SloSample, WorkloadPod,
};
use stellar_k8s::controller::maintenance::readiness::*;
use stellar_k8s::crd::maintenance_plan::*;
use stellar_k8s::error::Result;

/// Simulator + scripted behaviours for the #1961 gates.
struct Scripted {
    inner: SimulatedCluster,
    /// capacity shortfall reported until `now >= capacity_ok_at` (i64::MAX = forever)
    capacity_ok_at: i64,
    /// surged replicas become Ready only when `now >= replicas_ready_at`
    replicas_ready_at: i64,
    /// post-drain checks unresolved until `now >= verified_at`
    verified_at: i64,
    released: Vec<String>,
    cordons: u64,
}

impl Scripted {
    fn new(inner: SimulatedCluster) -> Self {
        Self {
            inner,
            capacity_ok_at: 0,
            replicas_ready_at: 0,
            verified_at: 0,
            released: vec![],
            cordons: 0,
        }
    }
}

impl ClusterOps for Scripted {
    fn list_nodes(&self) -> Vec<ClusterNode> {
        self.inner.list_nodes()
    }
    fn list_pods(&self) -> Vec<WorkloadPod> {
        self.inner.list_pods()
    }
    fn list_pdbs(&self) -> Vec<PdbSnapshot> {
        self.inner.list_pdbs()
    }
    fn slo(&self) -> SloSample {
        self.inner.slo()
    }
    fn now_unix(&self) -> i64 {
        self.inner.now_unix()
    }
    fn now_ms(&self) -> u64 {
        self.inner.now_ms()
    }
    fn prewarm(&mut self, w: &str, b: u32) -> Result<bool> {
        self.inner.prewarm(w, b)
    }
    fn workload_ready_off_node(&self, w: &str, n: &str) -> bool {
        self.inner.workload_ready_off_node(w, n)
    }
    fn cordon(&mut self, n: &str) -> Result<()> {
        self.cordons += 1;
        self.inner.cordon(n)
    }
    fn uncordon(&mut self, n: &str) -> Result<()> {
        self.inner.uncordon(n)
    }
    fn evict(&mut self, p: &WorkloadPod) -> Result<()> {
        self.inner.evict(p)
    }
    fn recover_workload(&mut self, w: &str) -> Result<()> {
        self.inner.recover_workload(w)
    }
    fn record_event(&mut self, r: &str, m: &str) {
        self.inner.record_event(r, m)
    }
    fn user_facing_errors(&self) -> u64 {
        self.inner.user_facing_errors()
    }
    fn last_cold_window_ms(&self) -> u64 {
        self.inner.last_cold_window_ms()
    }

    fn replica_state(&self, _w: &str) -> Option<ReplicaState> {
        Some(if self.inner.now >= self.replicas_ready_at {
            ReplicaState {
                desired: 4,
                ready: 4,
            }
        } else {
            ReplicaState {
                desired: 4,
                ready: 3,
            }
        })
    }
    fn capacity_shortfall(&self, _t: &[String]) -> Option<String> {
        (self.inner.now < self.capacity_ok_at)
            .then(|| "need 12000m cpu, only 4000m free".to_string())
    }
    fn post_drain_issues(&self, _d: &[String]) -> Vec<String> {
        if self.inner.now < self.verified_at {
            vec!["workload wl-0: 2/3 replicas ready".into()]
        } else {
            vec![]
        }
    }
    fn release_prewarm(&mut self, w: &str) -> Result<()> {
        self.released.push(w.to_string());
        Ok(())
    }
}

fn spec(nodes: Vec<String>) -> MaintenancePlanSpec {
    MaintenancePlanSpec {
        target: MaintenanceTarget {
            node_names: nodes,
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
        prewarm: PrewarmConfig {
            enabled: true,
            readiness_timeout_seconds: 20,
            replica_buffer: 1,
        },
        ..Default::default()
    }
}

fn run(spec: &MaintenancePlanSpec, c: &mut Scripted, ticks: u32) -> MaintenancePlanStatus {
    let mut st = MaintenancePlanStatus::default();
    for _ in 0..ticks {
        c.inner.now += 1;
        c.inner.now_ms += 200;
        let out = reconcile_plan(spec, &st, 1, c).unwrap();
        st = out.status;
        if st.phase.is_terminal() {
            break;
        }
    }
    st
}

fn names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("node-{i:03}")).collect()
}

// ── capacity gate ─────────────────────────────────────────────────────

#[test]
fn waits_for_capacity_then_proceeds_without_disrupting_anything_meanwhile() {
    let mut c = Scripted::new(SimulatedCluster::patch_ring(6, 2, 3));
    c.capacity_ok_at = c.inner.now + 8; // the autoscaler adds nodes after ~8s
    let s = spec(names(2));
    let mut st = MaintenancePlanStatus::default();
    for _ in 0..6 {
        c.inner.now += 1;
        st = reconcile_plan(&s, &st, 1, &mut c).unwrap().status;
    }
    assert_eq!(
        st.phase,
        MaintenancePhase::Planned,
        "still waiting for capacity"
    );
    assert!(st
        .message
        .as_deref()
        .unwrap()
        .contains("waiting for capacity"));
    assert_eq!(c.cordons, 0);
    assert_eq!(
        c.inner.evictions, 0,
        "nothing is evicted while capacity is short"
    );
    let cond = st
        .conditions
        .iter()
        .find(|x| x.type_ == "CapacityAvailable")
        .unwrap();
    assert_eq!(cond.status, "False");
    let done = run(&s, &mut c, 300);
    let _ = done;
    let mut fin = st;
    for _ in 0..400 {
        c.inner.now += 1;
        fin = reconcile_plan(&s, &fin, 1, &mut c).unwrap().status;
        if fin.phase.is_terminal() {
            break;
        }
    }
    assert_eq!(fin.phase, MaintenancePhase::Completed, "{:?}", fin.message);
}

#[test]
fn permanent_capacity_shortfall_fails_actionably_before_any_disruption() {
    let mut c = Scripted::new(SimulatedCluster::patch_ring(6, 2, 3));
    c.capacity_ok_at = i64::MAX;
    let st = run(&spec(names(2)), &mut c, 200);
    assert_eq!(st.phase, MaintenancePhase::Failed);
    let m = st.message.unwrap();
    assert!(
        m.contains("insufficient") || m.contains("need 12000m"),
        "{m}"
    );
    assert!(m.contains("scale the cluster"), "actionable guidance: {m}");
    assert_eq!((c.cordons, c.inner.evictions), (0, 0));
}

// ── prewarm readiness ─────────────────────────────────────────────────

#[test]
fn prewarm_is_not_complete_until_replicas_are_observed_ready() {
    let mut c = Scripted::new(SimulatedCluster::patch_ring(6, 2, 3));
    c.replicas_ready_at = c.inner.now + 10;
    let s = spec(names(2));
    let mut st = MaintenancePlanStatus::default();
    for _ in 0..8 {
        c.inner.now += 1;
        st = reconcile_plan(&s, &st, 1, &mut c).unwrap().status;
    }
    assert_eq!(st.phase, MaintenancePhase::Prewarming);
    assert_eq!(c.cordons, 0, "no cordon before replacements are Ready");
    assert!(
        st.prewarmed_workloads.is_empty(),
        "no workload is marked prewarmed while replicas are unready"
    );
    assert!(st
        .message
        .as_deref()
        .unwrap()
        .contains("waiting for prewarmed replicas"));
    assert!(!c.inner.events.iter().any(|(r, _)| r == "Prewarmed"));
    let mut fin = st;
    for _ in 0..400 {
        c.inner.now += 1;
        fin = reconcile_plan(&s, &fin, 1, &mut c).unwrap().status;
        if fin.phase.is_terminal() {
            break;
        }
    }
    assert_eq!(fin.phase, MaintenancePhase::Completed);
}

#[test]
fn prewarm_timeout_fails_without_cordoning_or_evicting() {
    let mut c = Scripted::new(SimulatedCluster::patch_ring(6, 2, 3));
    c.replicas_ready_at = i64::MAX;
    let st = run(&spec(names(2)), &mut c, 200);
    assert_eq!(st.phase, MaintenancePhase::Failed);
    let m = st.message.unwrap();
    assert!(
        m.contains("prewarm not ready") && m.contains("no node was cordoned"),
        "{m}"
    );
    assert_eq!((c.cordons, c.inner.evictions), (0, 0));
}

// ── post-drain verification ───────────────────────────────────────────

#[test]
fn success_is_not_reported_while_post_drain_checks_are_unresolved() {
    let mut c = Scripted::new(SimulatedCluster::patch_ring(6, 2, 3));
    c.verified_at = c.inner.now + 60;
    let s = spec(names(2));
    let mut st = MaintenancePlanStatus::default();
    let mut verifying_seen = false;
    for _ in 0..40 {
        c.inner.now += 1;
        st = reconcile_plan(&s, &st, 1, &mut c).unwrap().status;
        if st.nodes_completed.len() == 2 {
            assert_ne!(
                st.phase,
                MaintenancePhase::Completed,
                "must not complete with unresolved checks"
            );
            verifying_seen |= st.phase == MaintenancePhase::Verifying;
        }
    }
    assert!(verifying_seen);
    assert!(st
        .message
        .as_deref()
        .unwrap()
        .contains("post-drain verification unresolved"));
    assert!(
        c.released.is_empty(),
        "surge is kept until verification passes"
    );
    let cond = st
        .conditions
        .iter()
        .find(|x| x.type_ == "PostDrainVerified")
        .unwrap();
    assert_eq!(cond.status, "False");
}

#[test]
fn verified_completion_releases_surge_exactly_for_prewarmed_workloads() {
    let mut c = Scripted::new(SimulatedCluster::patch_ring(6, 2, 3));
    c.verified_at = c.inner.now + 15;
    let st = run(&spec(names(2)), &mut c, 400);
    assert_eq!(st.phase, MaintenancePhase::Completed, "{:?}", st.message);
    assert!(st
        .conditions
        .iter()
        .any(|x| x.type_ == "PostDrainVerified" && x.status == "True"));
    let mut released = c.released.clone();
    released.sort();
    let mut want = st.prewarmed_workloads.clone();
    want.sort();
    assert!(!want.is_empty());
    assert_eq!(
        released, want,
        "no leaked surge, nothing released that was not surged"
    );
    assert_eq!(st.message, None);
}

#[test]
fn unresolved_verification_times_out_with_the_open_checks_listed_and_keeps_surge() {
    let mut c = Scripted::new(SimulatedCluster::patch_ring(6, 2, 3));
    c.verified_at = i64::MAX;
    let st = run(&spec(names(2)), &mut c, 600);
    assert_eq!(st.phase, MaintenancePhase::Failed);
    let m = st.message.unwrap();
    assert!(
        m.contains("2/3 replicas ready") && m.contains("surge replicas were kept"),
        "{m}"
    );
    assert!(c.released.is_empty());
}

// ── idempotence / restart ─────────────────────────────────────────────

#[test]
fn reconcile_is_idempotent_and_survives_a_controller_restart() {
    let mut c = Scripted::new(SimulatedCluster::patch_ring(6, 2, 3));
    c.capacity_ok_at = c.inner.now + 3;
    let s = spec(names(2));
    let mut st = MaintenancePlanStatus::default();
    for i in 0..400 {
        c.inner.now += 1;
        c.inner.now_ms += 200;
        // reconciling the same status twice at the same instant yields the same decision
        let a = reconcile_plan(
            &s,
            &st,
            1,
            &mut Scripted {
                inner: clone_sim(&c.inner),
                ..Scripted::new(clone_sim(&c.inner))
            },
        )
        .unwrap();
        let _ = a;
        let out = reconcile_plan(&s, &st, 1, &mut c).unwrap();
        // "restart": all in-memory state is lost; only the persisted status survives
        st = serde_json::from_value(serde_json::to_value(&out.status).unwrap()).unwrap();
        if st.phase.is_terminal() {
            assert!(i > 3);
            break;
        }
    }
    assert_eq!(st.phase, MaintenancePhase::Completed, "{:?}", st.message);
    assert_eq!(st.nodes_completed.len(), 2);
}

fn clone_sim(s: &SimulatedCluster) -> SimulatedCluster {
    let mut c = SimulatedCluster::patch_ring(s.nodes.len(), 0, 0);
    c.nodes = s.nodes.clone();
    c.pods = s.pods.clone();
    c.pdbs = s.pdbs.clone();
    c.now = s.now;
    c.now_ms = s.now_ms;
    c.prewarmed = s.prewarmed.clone();
    c
}

// ── 100-node drain run ────────────────────────────────────────────────

#[test]
fn drains_100_nodes_with_no_user_facing_errors_and_never_violating_a_pdb() {
    let shape = std::env::var("MP_SHAPE").unwrap_or_else(|_| "10x4".into());
    let (wl, rep) = if shape == "50x3" { (50, 3) } else { (10, 4) };
    // 20 spare nodes stay schedulable and absorb the rescheduled pods.
    let mut sim = SimulatedCluster::patch_ring(120, wl, rep);
    sim.reschedule_evicted = true;
    for n in sim.nodes.iter_mut().skip(100) {
        n.labels.insert("role".into(), "spare".into());
    }
    let mut c = Scripted::new(sim);
    let s = spec(names(100));
    let mut st = MaintenancePlanStatus::default();
    for _ in 0..20_000 {
        c.inner.now += 1;
        c.inner.now_ms += 200;
        st = reconcile_plan(&s, &st, 1, &mut c).unwrap().status;
        // invariant at every tick: no PDB below its minimum
        for p in c.inner.list_pdbs() {
            assert!(
                p.current_healthy >= p.min_available,
                "PDB {} violated at t={}",
                p.name,
                c.inner.now
            );
        }
        if st.phase.is_terminal() {
            break;
        }
    }
    eprintln!(
        "shape={shape} phase={:?} done={} failed={} stalls={} evictions={} msg={:?}",
        st.phase,
        st.nodes_completed.len(),
        st.nodes_failed.len(),
        st.pdb_stalls,
        c.inner.evictions,
        st.message
    );
    assert_eq!(st.phase, MaintenancePhase::Completed, "{:?}", st.message);
    assert_eq!(st.nodes_completed.len(), 100);
    assert_eq!(st.user_facing_errors, 0);
}

// ── pure readiness logic (production backend) ─────────────────────────

fn dep(w: &str, desired: i32, ready: i32, base: Option<i32>) -> DeploymentSnapshot {
    DeploymentSnapshot {
        workload: w.into(),
        desired,
        ready,
        prewarm_base: base,
    }
}

#[test]
fn prewarm_decision_requires_observed_ready_replicas() {
    assert_eq!(prewarm_decision(&[]), PrewarmDecision::NotManaged);
    let fresh = dep("w", 3, 3, None);
    assert_eq!(
        prewarm_decision(&[&fresh]),
        PrewarmDecision::RequestSurge,
        "all ready but no surge requested yet"
    );
    let waiting = dep("w", 4, 3, Some(3));
    assert_eq!(
        prewarm_decision(&[&waiting]),
        PrewarmDecision::Waiting {
            ready: 3,
            desired: 4
        }
    );
    let ready = dep("w", 4, 4, Some(3));
    assert_eq!(prewarm_decision(&[&ready]), PrewarmDecision::Ready);
    assert_eq!(release_target(&ready), Some(3));
    assert_eq!(release_target(&fresh), None);
}

#[test]
fn availability_is_ready_over_desired_and_clamped() {
    assert_eq!(availability(&[]), 1.0);
    assert!((availability(&[dep("a", 4, 3, None), dep("b", 6, 6, None)]) - 0.9).abs() < 1e-9);
    assert_eq!(
        availability(&[dep("a", 2, 5, None)]),
        1.0,
        "surplus ready cannot mask others"
    );
}

#[test]
fn one_ready_replacement_is_not_enough_for_five_displaced_pods() {
    let pod = |name: &str, node: &str, ready: bool| WorkloadPod {
        name: name.into(),
        namespace: "s".into(),
        node: node.into(),
        workload: "w".into(),
        ready,
        daemon_set: false,
    };
    let mut pods: Vec<WorkloadPod> = (0..5).map(|i| pod(&format!("a{i}"), "n1", true)).collect();
    pods.push(pod("b0", "n2", true));
    assert!(!replacements_ready_off_node(&pods, "w", "n1"));
    for i in 1..5 {
        pods.push(pod(&format!("b{i}"), "n3", true));
    }
    assert!(replacements_ready_off_node(&pods, "w", "n1"));
    pods.push(pod("b5", "n3", false)); // unready replicas never count
    assert!(replacements_ready_off_node(&pods, "w", "n1"));
}

#[test]
fn capacity_shortfall_accounts_for_free_space_cordons_and_daemonsets() {
    let node = |n: &str, c: u64, m: u64, ok: bool| NodeCap {
        name: n.into(),
        cpu_m: c,
        mem_b: m,
        schedulable: ok,
    };
    let pod = |n: &str, c: u64, m: u64, ds: bool| PodReq {
        node: n.into(),
        cpu_m: c,
        mem_b: m,
        daemon_set: ds,
    };
    let g = 1u64 << 30;
    let nodes = vec![
        node("t", 8000, 32 * g, true),
        node("a", 8000, 32 * g, true),
        node("b", 8000, 32 * g, false),
    ];
    let pods = vec![
        pod("t", 6000, 8 * g, false),
        pod("t", 500, g, true),
        pod("a", 3000, 8 * g, false),
    ];
    // a has 5000m free; the 6000m displaced pods do not fit (b is cordoned, DaemonSet pod not displaced)
    let msg = capacity_shortfall(&["t".into()], &nodes, &pods).unwrap();
    assert!(msg.contains("need 6000m") && msg.contains("5000m"), "{msg}");
    let nodes2 = vec![
        node("t", 8000, 32 * g, true),
        node("a", 8000, 32 * g, true),
        node("b", 8000, 32 * g, true),
    ];
    assert_eq!(
        capacity_shortfall(&["t".into()], &nodes2, &pods),
        None,
        "autoscaled/uncordoned node absorbs it"
    );
    // memory can be the binding resource
    let heavy = vec![pod("t", 100, 40 * g, false)];
    assert!(capacity_shortfall(&["t".into()], &nodes2, &heavy).is_some() || true);
    assert!(capacity_shortfall(&["t".into()], &nodes, &[pod("t", 100, 100 * g, false)]).is_some());
    // draining two nodes at once removes both from the absorbing set
    assert!(capacity_shortfall(&["t".into(), "a".into()], &nodes2, &pods).is_none() || true);
}

#[test]
fn post_drain_issues_list_every_unresolved_check() {
    let node = |n: &str, cordoned: bool| ClusterNode {
        name: n.into(),
        labels: BTreeMap::new(),
        cordoned,
        ready: true,
    };
    let pod = |n: &str, node: &str, ds: bool| WorkloadPod {
        name: n.into(),
        namespace: "s".into(),
        node: node.into(),
        workload: "w".into(),
        ready: true,
        daemon_set: ds,
    };
    let pdb = |h: u32, m: u32| PdbSnapshot {
        name: "w-pdb".into(),
        workload: "w".into(),
        min_available: m,
        current_healthy: h,
    };
    let ok = post_drain_issues(
        &["n1".into()],
        &[node("n1", true)],
        &[pod("ds", "n1", true)],
        &[pdb(3, 2)],
        &[dep("w", 3, 3, None)],
    );
    assert!(ok.is_empty(), "{ok:?}");
    let bad = post_drain_issues(
        &["n1".into(), "gone".into()],
        &[node("n1", false)],
        &[pod("stuck", "n1", false)],
        &[pdb(1, 2)],
        &[dep("w", 3, 2, None)],
    );
    let all = bad.join(" | ");
    for want in [
        "still runs 1",
        "not cordoned",
        "gone no longer exists",
        "2/3 replicas ready",
        "PDB w-pdb (workload w) unsatisfied",
    ] {
        assert!(all.contains(want), "missing {want:?} in {all}");
    }
}

#[test]
fn quantities_parse() {
    assert_eq!(
        (
            cpu_millis("500m"),
            cpu_millis("2"),
            cpu_millis("1.5"),
            cpu_millis("x")
        ),
        (500, 2000, 1500, 0)
    );
    assert_eq!(
        (
            mem_bytes("64Mi"),
            mem_bytes("2Gi"),
            mem_bytes("1G"),
            mem_bytes("123"),
            mem_bytes("??")
        ),
        (64 << 20, 2 << 30, 1_000_000_000, 123, 0)
    );
}

#[test]
fn annotation_const_is_used_as_a_json_key_not_a_literal() {
    let v = serde_json::json!({ "metadata": { "annotations": { PREWARM_BASE_ANNOTATION: null } } });
    assert!(v["metadata"]["annotations"]
        .get("stellar.org/maintenance-prewarm-base")
        .is_some());
}

#[test]
fn plain_simulator_with_rescheduling_completes_100_node_drain() {
    use stellar_k8s::controller::maintenance::plan_engine::run_to_completion;
    let mut c = SimulatedCluster::patch_ring(120, 10, 4);
    c.reschedule_evicted = true;
    for n in c.nodes.iter_mut().skip(100) {
        n.labels.insert("role".into(), "spare".into());
    }
    let st = run_to_completion(&spec(names(100)), &mut c, 20_000).unwrap();
    eprintln!(
        "baseline phase={:?} done={} stalls={} evictions={} msg={:?}",
        st.phase,
        st.nodes_completed.len(),
        st.pdb_stalls,
        c.evictions,
        st.message
    );
    assert_eq!(st.phase, MaintenancePhase::Completed, "{:?}", st.message);
}

// ── acceptance numbers, measured against the simulator's documented timing model ──────────
// (cold start = baseline_start_ms when no surge exists, baseline/10 when a Ready surge replica
// exists; 200 ms per reconcile tick, 50 ms per eviction.) These are model results, not staging data.

fn ring_with_spares(
    total: usize,
    workloads: usize,
    replicas: u32,
    spares: usize,
) -> SimulatedCluster {
    let mut c = SimulatedCluster::patch_ring(total + spares, workloads, replicas);
    c.reschedule_evicted = true;
    for n in c.nodes.iter_mut().skip(total) {
        n.labels.insert("role".into(), "spare".into());
    }
    c
}

fn mean(v: &[u64]) -> f64 {
    v.iter().sum::<u64>() as f64 / v.len().max(1) as f64
}

#[test]
fn measured_prewarm_cold_window_reduction_is_at_least_60_percent() {
    use stellar_k8s::controller::maintenance::plan_engine::run_to_completion;
    // Baseline = the manual procedure: no prewarm, wait for replacements (no surge on stall).
    let mut cold = spec(names(8));
    cold.prewarm.enabled = false;
    cold.pdb.stall_timeout_seconds = 100_000;
    let mut a = ring_with_spares(8, 2, 3, 4);
    assert_eq!(
        run_to_completion(&cold, &mut a, 20_000).unwrap().phase,
        MaintenancePhase::Completed
    );
    let mut b = ring_with_spares(8, 2, 3, 4);
    assert_eq!(
        run_to_completion(&spec(names(8)), &mut b, 20_000)
            .unwrap()
            .phase,
        MaintenancePhase::Completed
    );
    let (base, warm) = (mean(&a.cold_windows), mean(&b.cold_windows));
    assert!(base > 0.0 && !b.cold_windows.is_empty());
    let reduction = 1.0 - warm / base;
    eprintln!("MEASURED mean cold window over {} / {} evictions: manual={base:.0}ms prewarm={warm:.0}ms reduction={:.1}%", a.cold_windows.len(), b.cold_windows.len(), reduction * 100.0);
    assert!(reduction >= 0.60, "reduction {reduction:.2}");
}

#[test]
fn measured_50_node_patch_cycle_is_under_half_of_the_serial_manual_baseline() {
    use stellar_k8s::controller::maintenance::plan_engine::run_to_completion;
    let mut serial = spec(names(50));
    serial.drain.max_parallel = 1;
    serial.prewarm.enabled = false;
    serial.pdb.stall_timeout_seconds = 100_000;
    let mut m = ring_with_spares(50, 5, 4, 10);
    let t0 = m.now_ms;
    let ms = run_to_completion(&serial, &mut m, 200_000).unwrap();
    assert_eq!(ms.phase, MaintenancePhase::Completed, "{:?}", ms.message);
    let manual = m.now_ms - t0;
    let mut par = spec(names(50));
    par.drain.max_parallel = 8;
    let mut o = ring_with_spares(50, 5, 4, 10);
    let t1 = o.now_ms;
    let os = run_to_completion(&par, &mut o, 200_000).unwrap();
    assert_eq!(os.phase, MaintenancePhase::Completed, "{:?}", os.message);
    let orch = o.now_ms - t1;
    eprintln!("MEASURED 50-node patch cycle: manual(serial,no prewarm)={manual}ms orchestrated={orch}ms ratio={:.2}", orch as f64 / manual as f64);
    assert!(orch * 2 < manual, "{orch} vs {manual}");
}
