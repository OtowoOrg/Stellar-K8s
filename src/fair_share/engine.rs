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

//! Scheduling decisions over a validated quota tree: admission (filter), fair-share
//! scoring (prioritize) and hierarchy-aware preemption.
//!
//! ## Rules
//!
//! **Admission.** A pod of tenant leaf `L` is admitted iff
//! 1. for `L` and every ancestor `X`: `usage(X) + req <= max(X)`; and
//! 2. the part of `req` that stays within `L`'s *guarantee* is always admitted; the
//!    **elastic** part (above the guarantee) is admitted only if it fits in the cluster's
//!    free capacity *after reserving* the unmet guarantees of other tenants that have
//!    pending pods, and (when others are waiting) stays within `L`'s fair entitlement.
//!    Borrowing therefore never starves a waiting guaranteed tenant.
//!
//! **Fairness under contention.** While any other tenant has pending pods, a tenant may
//! borrow only up to its demand-limited, weighted **entitlement** (see `Tree::entitlements`),
//! so contended elastic capacity is shared by weight rather than by arrival order. (A
//! kube-scheduler extender cannot reorder the scheduling queue; fairness is therefore
//! enforced at admission.)
//!
//! **Preemption.** Only a pod that fits entirely inside its tenant's remaining guarantee
//! may preempt. A victim must belong to a tenant (and every node between the victim's
//! leaf and the lowest common ancestor with the preemptor) that stays at or above its
//! guarantee after the eviction. Sibling/cousin tenants at or below their guarantee are
//! therefore protected, and borrowed capacity is reclaimed from the most over-guarantee
//! tenants first. Within a single tenant only strictly lower-priority pods may be evicted
//! (otherwise a tenant would swap its own pods endlessly).

use super::tree::Tree;
use super::ResVec;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A pod as seen by the fair-share engine.
#[derive(Debug, Clone, PartialEq)]
pub struct PodRec {
    pub uid: String,
    pub namespace: String,
    pub node: Option<String>,
    pub req: ResVec,
    pub priority: i32,
    /// Monotonic start order (larger = newer); used as a victim tie-break.
    pub started: u64,
}

/// Admission outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Namespace is not governed by any `TenantQuota` (left to ResourceQuota / default behaviour).
    Ungoverned,
    /// Fits entirely within the tenant's guarantee.
    AllowGuaranteed,
    /// Uses borrowed (elastic) capacity.
    AllowElastic,
    DenyMax {
        node: String,
    },
    DenyNoElasticCapacity,
    /// The pod is inside its tenant's guarantee but an ancestor is at its ceiling because
    /// siblings borrowed capacity: the pod must be placed through preemption of that borrowing.
    NeedsReclaim {
        node: String,
    },
}

impl Verdict {
    pub fn allowed(&self) -> bool {
        matches!(
            self,
            Verdict::Ungoverned | Verdict::AllowGuaranteed | Verdict::AllowElastic
        )
    }
    pub fn reason(&self) -> String {
        match self {
            Verdict::DenyMax { node } => format!("tenant quota {node}: max exceeded"),
            Verdict::NeedsReclaim { node } => {
                format!("tenant quota {node} is at max through borrowed capacity: reclaim required")
            }
            Verdict::DenyNoElasticCapacity => {
                "no free elastic capacity after reserving other tenants' guarantees".to_string()
            }
            _ => String::new(),
        }
    }
}

/// Why a preemption was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreemptRefusal {
    Ungoverned,
    ExceedsGuarantee,
    ProtectedVictim {
        victim: String,
        node: String,
    },
    Insufficient,
    /// Victims would not bring an ancestor back under its max.
    DoesNotCureQuota {
        node: String,
    },
}

const LAT_BUCKETS: [f64; 8] = [0.1, 0.5, 1.0, 5.0, 15.0, 60.0, 300.0, 1800.0];

/// Counters (atomics so the engine can be shared behind `Arc` without locks for metrics).
#[derive(Default)]
pub struct Metrics {
    pub admit_guaranteed: AtomicU64,
    pub admit_elastic: AtomicU64,
    pub deny_max: AtomicU64,
    pub deny_capacity: AtomicU64,
    pub needs_reclaim: AtomicU64,
    pub preempt_approved: AtomicU64,
    pub preempt_refused_guarantee: AtomicU64,
    pub preempt_refused_protected: AtomicU64,
    pub preempt_refused_insufficient: AtomicU64,
    pub config_valid: AtomicU64,
    pub config_reloads: AtomicU64,
    pub config_rejected: AtomicU64,
    lat: [AtomicU64; 8],
    lat_count: AtomicU64,
    lat_sum_ms: AtomicU64,
}

impl Metrics {
    pub fn observe_scheduling_delay(&self, secs: f64) {
        for (i, b) in LAT_BUCKETS.iter().enumerate() {
            if secs <= *b {
                self.lat[i].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.lat_count.fetch_add(1, Ordering::Relaxed);
        self.lat_sum_ms
            .fetch_add((secs * 1000.0) as u64, Ordering::Relaxed);
    }
}

/// The decision engine.
pub struct Engine {
    pub tree: Tree,
    pub capacity: ResVec,
    running: HashMap<String, PodRec>,
    pending: HashMap<String, PodRec>,
    pub metrics: Arc<Metrics>,
    /// Memoized `usage()`; cleared on every mutation.
    usage_cache: Mutex<Option<(Vec<ResVec>, ResVec)>>,
}

impl Engine {
    pub fn new(tree: Tree, capacity: ResVec) -> Self {
        let metrics = Arc::new(Metrics::default());
        metrics.config_valid.store(1, Ordering::Relaxed);
        Self {
            tree,
            capacity,
            running: HashMap::new(),
            pending: HashMap::new(),
            metrics,
            usage_cache: Mutex::new(None),
        }
    }

    /// Swap in a new (already validated) tree; existing pods are kept and re-mapped lazily.
    pub fn set_tree(&mut self, tree: Tree) {
        self.invalidate();
        self.tree = tree;
        self.metrics.config_reloads.fetch_add(1, Ordering::Relaxed);
        self.metrics.config_valid.store(1, Ordering::Relaxed);
    }
    pub fn note_invalid_config(&self) {
        self.metrics.config_rejected.fetch_add(1, Ordering::Relaxed);
        self.metrics.config_valid.store(0, Ordering::Relaxed);
    }
    pub fn set_capacity(&mut self, c: ResVec) {
        self.invalidate();
        self.capacity = c;
    }
    /// Replace the pod view in one step (used by the periodic reconcile).
    pub fn replace_pods(&mut self, running: Vec<PodRec>, pending: Vec<PodRec>) {
        self.invalidate();
        self.running = running.into_iter().map(|p| (p.uid.clone(), p)).collect();
        self.pending = pending.into_iter().map(|p| (p.uid.clone(), p)).collect();
    }
    pub fn bind(&mut self, pod: PodRec) {
        self.invalidate();
        self.pending.remove(&pod.uid);
        self.running.insert(pod.uid.clone(), pod);
    }
    pub fn add_pending(&mut self, pod: PodRec) {
        self.pending.insert(pod.uid.clone(), pod);
    }
    pub fn remove(&mut self, uid: &str) -> Option<PodRec> {
        self.invalidate();
        self.pending.remove(uid);
        self.running.remove(uid)
    }
    pub fn running(&self) -> impl Iterator<Item = &PodRec> {
        self.running.values()
    }
    pub fn pod(&self, uid: &str) -> Option<&PodRec> {
        self.running.get(uid).or_else(|| self.pending.get(uid))
    }

    /// Aggregated usage per tree node (index-aligned with `tree.nodes`) and total cluster use.
    pub fn usage(&self) -> (Vec<ResVec>, ResVec) {
        if let Some(c) = self.usage_cache.lock().unwrap().as_ref() {
            return c.clone();
        }
        let r = self.compute_usage();
        *self.usage_cache.lock().unwrap() = Some(r.clone());
        r
    }

    fn invalidate(&self) {
        *self.usage_cache.lock().unwrap() = None;
    }

    fn compute_usage(&self) -> (Vec<ResVec>, ResVec) {
        let mut u = vec![ResVec::ZERO; self.tree.len()];
        let mut total = ResVec::ZERO;
        for p in self.running.values() {
            total = total.add(p.req);
            if let Some(leaf) = self.tree.leaf_for_namespace(&p.namespace) {
                for x in self.tree.path_to_root(leaf) {
                    u[x] = u[x].add(p.req);
                }
            }
        }
        (u, total)
    }

    fn pending_by_leaf(&self) -> HashMap<usize, ResVec> {
        let mut m: HashMap<usize, ResVec> = HashMap::new();
        for p in self.pending.values() {
            if let Some(leaf) = self.tree.leaf_for_namespace(&p.namespace) {
                let e = m.entry(leaf).or_insert(ResVec::ZERO);
                *e = e.add(p.req);
            }
        }
        m
    }

    /// Admission decision (filter).
    pub fn admit(&self, pod: &PodRec) -> Verdict {
        let v = self.admit_inner(pod);
        let c = match &v {
            Verdict::AllowGuaranteed => &self.metrics.admit_guaranteed,
            Verdict::AllowElastic => &self.metrics.admit_elastic,
            Verdict::DenyMax { .. } => &self.metrics.deny_max,
            Verdict::DenyNoElasticCapacity => &self.metrics.deny_capacity,
            Verdict::NeedsReclaim { .. } => &self.metrics.needs_reclaim,
            Verdict::Ungoverned => return v,
        };
        c.fetch_add(1, Ordering::Relaxed);
        v
    }

    fn admit_inner(&self, pod: &PodRec) -> Verdict {
        let Some(leaf) = self.tree.leaf_for_namespace(&pod.namespace) else {
            return Verdict::Ungoverned;
        };
        let (usage, total) = self.usage();
        let g = self.tree.nodes[leaf].guaranteed;
        let within = pod.req.min(g.sub(usage[leaf]));
        let elastic = pod.req.sub(within);
        // Ceilings at every level. A pod entirely inside its guarantee can only trip an ancestor's
        // ceiling because siblings borrowed: that is curable by reclaim, not a denial.
        for x in self.tree.path_to_root(leaf) {
            if !usage[x].add(pod.req).fits_in(self.tree.nodes[x].max) {
                let node = self.tree.nodes[x].name.clone();
                return if elastic.is_zero() {
                    Verdict::NeedsReclaim { node }
                } else {
                    Verdict::DenyMax { node }
                };
            }
        }
        if elastic.is_zero() {
            return Verdict::AllowGuaranteed;
        }
        let free = self.capacity.sub(total);
        let mut pend = self.pending_by_leaf();
        if !self.pending.contains_key(&pod.uid) {
            let e = pend.entry(leaf).or_insert(ResVec::ZERO);
            *e = e.add(pod.req);
        }
        let mut reserved = ResVec::ZERO;
        for (&j, &p) in &pend {
            if j != leaf {
                reserved = reserved.add(self.tree.nodes[j].guaranteed.sub(usage[j]).min(p));
            }
        }
        if pend.iter().any(|(&j, p)| j != leaf && !p.is_zero()) {
            // Contention: borrow only up to the weighted fair entitlement.
            let mut demand = pend.clone();
            for l in self.tree.leaves() {
                let e = demand.entry(l).or_insert(ResVec::ZERO);
                *e = e.add(usage[l]);
            }
            let ent = self.tree.entitlements(&demand, self.capacity);
            if !usage[leaf].add(pod.req).fits_in(ent.by_node[leaf]) {
                return Verdict::DenyNoElasticCapacity;
            }
        }
        if elastic.fits_in(free.sub(reserved)) {
            Verdict::AllowElastic
        } else {
            Verdict::DenyNoElasticCapacity
        }
    }

    /// Check that evicting `victims` for `pending` is allowed. Does not mutate state.
    pub fn check_preemption(
        &self,
        pending: &PodRec,
        victims: &[&PodRec],
    ) -> Result<(), PreemptRefusal> {
        let Some(pl) = self.tree.leaf_for_namespace(&pending.namespace) else {
            return Err(PreemptRefusal::Ungoverned);
        };
        let (mut usage, _) = self.usage();
        if !pending
            .req
            .fits_in(self.tree.nodes[pl].guaranteed.sub(usage[pl]))
        {
            self.metrics
                .preempt_refused_guarantee
                .fetch_add(1, Ordering::Relaxed);
            return Err(PreemptRefusal::ExceedsGuarantee);
        }
        for v in victims {
            if let Err(r) = self.victim_ok(pl, pending, v, &mut usage) {
                self.metrics
                    .preempt_refused_protected
                    .fetch_add(1, Ordering::Relaxed);
                return Err(r);
            }
        }
        if let Some(x) = self.uncured_ceiling(pl, pending.req, &usage) {
            self.metrics
                .preempt_refused_insufficient
                .fetch_add(1, Ordering::Relaxed);
            return Err(PreemptRefusal::DoesNotCureQuota {
                node: self.tree.nodes[x].name.clone(),
            });
        }
        self.metrics
            .preempt_approved
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Hierarchy check for one victim; on success the cumulative `usage` is reduced.
    fn victim_ok(
        &self,
        pl: usize,
        pending: &PodRec,
        v: &PodRec,
        usage: &mut [ResVec],
    ) -> Result<(), PreemptRefusal> {
        let Some(vl) = self.tree.leaf_for_namespace(&v.namespace) else {
            // Pods outside any quota tree are not borrowed capacity of a tenant.
            return Err(PreemptRefusal::ProtectedVictim {
                victim: v.uid.clone(),
                node: "<ungoverned>".into(),
            });
        };
        if vl == pl {
            // Within one tenant only ordinary priority preemption applies. Evicting an equal- or
            // higher-priority pod of the same tenant is a zero-sum swap that causes churn.
            return if v.priority < pending.priority {
                for x in self.tree.path_to_root(vl) {
                    usage[x] = usage[x].sub(v.req);
                }
                Ok(())
            } else {
                Err(PreemptRefusal::ProtectedVictim {
                    victim: v.uid.clone(),
                    node: self.tree.nodes[pl].name.clone(),
                })
            };
        }
        let lca = self.tree.lca(pl, vl);
        for x in self.tree.path_to_root(vl) {
            if Some(x) == lca {
                break;
            }
            let after = usage[x].sub(v.req);
            if !self.tree.nodes[x].guaranteed.fits_in(after) {
                return Err(PreemptRefusal::ProtectedVictim {
                    victim: v.uid.clone(),
                    node: self.tree.nodes[x].name.clone(),
                });
            }
        }
        for x in self.tree.path_to_root(vl) {
            usage[x] = usage[x].sub(v.req);
        }
        Ok(())
    }

    /// First ancestor (or the leaf) whose ceiling would still be exceeded by `req` given `usage`.
    fn uncured_ceiling(&self, leaf: usize, req: ResVec, usage: &[ResVec]) -> Option<usize> {
        self.tree
            .path_to_root(leaf)
            .into_iter()
            .find(|&x| !usage[x].add(req).fits_in(self.tree.nodes[x].max))
    }

    /// Choose victims among `candidates` (pods on one node) so that `pending` fits on that node
    /// (given `node_free`) **and** every ceiling on the pending tenant's path is respected.
    /// `None` if preemption is not permitted or cannot achieve both.
    pub fn select_victims(
        &self,
        pending: &PodRec,
        candidates: &[&PodRec],
        node_free: ResVec,
    ) -> Option<Vec<String>> {
        let node_need = pending.req.sub(node_free);
        let pl = self.tree.leaf_for_namespace(&pending.namespace)?;
        let (mut usage, _) = self.usage();
        if !pending
            .req
            .fits_in(self.tree.nodes[pl].guaranteed.sub(usage[pl]))
        {
            return None;
        }
        let mut excess_nodes: Vec<usize> = self.tree.path_to_root(pl);
        excess_nodes.retain(|&x| !usage[x].add(pending.req).fits_in(self.tree.nodes[x].max));
        if node_need.is_zero() && excess_nodes.is_empty() {
            return Some(Vec::new());
        }
        let over = |p: &PodRec| -> f64 {
            match self.tree.leaf_for_namespace(&p.namespace) {
                Some(l) => {
                    let g = self.tree.nodes[l].guaranteed;
                    if g.is_zero() {
                        1e9
                    } else {
                        usage[l].dominant_share(g)
                    }
                }
                None => -1.0,
            }
        };
        let relieves = |p: &PodRec| -> bool {
            excess_nodes.is_empty()
                || self
                    .tree
                    .leaf_for_namespace(&p.namespace)
                    .map(|l| {
                        self.tree
                            .path_to_root(l)
                            .iter()
                            .any(|x| excess_nodes.contains(x))
                    })
                    .unwrap_or(false)
        };
        let mut order: Vec<&&PodRec> = candidates.iter().collect();
        let keys: HashMap<String, f64> = order.iter().map(|p| (p.uid.clone(), over(p))).collect();
        order.sort_by(|a, b| {
            // pods that relieve an exceeded ceiling first, then most over their guarantee
            relieves(b)
                .cmp(&relieves(a))
                .then(
                    keys[&b.uid]
                        .partial_cmp(&keys[&a.uid])
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(a.priority.cmp(&b.priority))
                .then(b.started.cmp(&a.started))
        });
        let mut freed = ResVec::ZERO;
        let mut chosen = Vec::new();
        for v in order {
            if !excess_nodes.is_empty() && !relieves(v) && node_need.fits_in(freed) {
                continue; // node already satisfied; do not evict pods that cannot cure the ceiling
            }
            if self.victim_ok(pl, pending, v, &mut usage).is_ok() {
                freed = freed.add(v.req);
                chosen.push(v.uid.clone());
                if node_need.fits_in(freed)
                    && self.uncured_ceiling(pl, pending.req, &usage).is_none()
                {
                    return Some(chosen);
                }
            }
        }
        self.metrics
            .preempt_refused_insufficient
            .fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Per-leaf fairness ratios (`dominant_share(usage, entitlement)`) for contending tenants.
    pub fn fairness_ratios(&self) -> Vec<(String, f64)> {
        let (usage, _) = self.usage();
        let mut demand = self.pending_by_leaf();
        for l in self.tree.leaves() {
            let e = demand.entry(l).or_insert(ResVec::ZERO);
            *e = e.add(usage[l]);
        }
        let ent = self.tree.entitlements(&demand, self.capacity);
        self.tree
            .leaves()
            .filter(|&l| !demand[&l].is_zero() && !ent.by_node[l].is_zero())
            .map(|l| {
                (
                    self.tree.nodes[l].name.clone(),
                    usage[l].dominant_share(ent.by_node[l]),
                )
            })
            .collect()
    }

    /// Jain's fairness index over contending tenants' usage/entitlement ratios.
    pub fn fairness_index(&self) -> f64 {
        let xs: Vec<f64> = self.fairness_ratios().into_iter().map(|(_, r)| r).collect();
        super::jain_index(&xs)
    }

    /// Invariant check for the guarantee: returns tenants that have a pending pod which fits
    /// entirely inside their *remaining guarantee* (so it must be schedulable, by preemption if
    /// needed) yet is not running. Empty = guarantees hold at this instant.
    ///
    /// Guarantees are vectors: a pod that would push a tenant above its guarantee in *any*
    /// dimension (e.g. memory) is elastic in that dimension and is not covered by this check.
    pub fn guarantee_violations(&self) -> Vec<String> {
        let (usage, _) = self.usage();
        let mut out: Vec<String> = Vec::new();
        for p in self.pending.values() {
            if let Some(l) = self.tree.leaf_for_namespace(&p.namespace) {
                if p.req.fits_in(self.tree.nodes[l].guaranteed.sub(usage[l]))
                    && !out.contains(&self.tree.nodes[l].name)
                {
                    out.push(self.tree.nodes[l].name.clone());
                }
            }
        }
        out.sort();
        out
    }

    /// Prometheus text. Tenant labels are bounded by the number of `TenantQuota` objects
    /// (admin-defined); pods and namespaces are never used as labels.
    pub fn render_metrics(&self) -> String {
        let m = &self.metrics;
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let (usage, total) = self.usage();
        let mut demand = self.pending_by_leaf();
        for l in self.tree.leaves() {
            let e = demand.entry(l).or_insert(ResVec::ZERO);
            *e = e.add(usage[l]);
        }
        let ent = self.tree.entitlements(&demand, self.capacity);
        let mut o = String::new();
        let mut gauge = |name: &str, help: &str| {
            let _ = writeln!(o, "# HELP {name} {help}\n# TYPE {name} gauge");
        };
        for (name, help) in [
            (
                "stellar_fairshare_allocated",
                "Resources in use per tenant node.",
            ),
            (
                "stellar_fairshare_guaranteed",
                "Configured effective guarantee per tenant node.",
            ),
            (
                "stellar_fairshare_max",
                "Configured effective ceiling per tenant node (omitted when unbounded).",
            ),
            (
                "stellar_fairshare_entitlement",
                "Demand-limited fair-share entitlement per tenant node.",
            ),
            (
                "stellar_fairshare_elastic_usage",
                "Usage above the guarantee (borrowed capacity).",
            ),
            (
                "stellar_fairshare_guaranteed_utilization_ratio",
                "min(usage, guarantee) / guarantee.",
            ),
            (
                "stellar_fairshare_quota_saturation_ratio",
                "usage / max (omitted when unbounded).",
            ),
        ] {
            gauge(name, help);
        }
        for (i, n) in self.tree.nodes.iter().enumerate() {
            for (res, u, gu, mx, en) in [
                (
                    "cpu",
                    usage[i].cpu_m as f64 / 1000.0,
                    n.guaranteed.cpu_m as f64 / 1000.0,
                    n.max.cpu_m,
                    ent.by_node[i].cpu_m as f64 / 1000.0,
                ),
                (
                    "memory",
                    usage[i].mem_b as f64,
                    n.guaranteed.mem_b as f64,
                    n.max.mem_b,
                    ent.by_node[i].mem_b as f64,
                ),
            ] {
                let l = format!("tenant=\"{}\",resource=\"{res}\"", n.name);
                let scale = if res == "cpu" { 1000.0 } else { 1.0 };
                let _ = writeln!(o, "stellar_fairshare_allocated{{{l}}} {u}");
                let _ = writeln!(o, "stellar_fairshare_guaranteed{{{l}}} {gu}");
                let _ = writeln!(o, "stellar_fairshare_entitlement{{{l}}} {en}");
                let _ = writeln!(
                    o,
                    "stellar_fairshare_elastic_usage{{{l}}} {}",
                    (u - gu).max(0.0)
                );
                if gu > 0.0 {
                    let _ = writeln!(
                        o,
                        "stellar_fairshare_guaranteed_utilization_ratio{{{l}}} {}",
                        u.min(gu) / gu
                    );
                }
                if mx < super::UNBOUNDED {
                    let mxf = mx as f64 / scale;
                    let _ = writeln!(o, "stellar_fairshare_max{{{l}}} {mxf}");
                    if mxf > 0.0 {
                        let _ = writeln!(
                            o,
                            "stellar_fairshare_quota_saturation_ratio{{{l}}} {}",
                            u / mxf
                        );
                    }
                }
            }
        }
        let mut c = |name: &str, help: &str, kind: &str, v: String| {
            let _ = writeln!(o, "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {v}");
        };
        c(
            "stellar_fairshare_jain_index",
            "Jain fairness index over contending tenants (1 = fair).",
            "gauge",
            format!("{}", self.fairness_index()),
        );
        c(
            "stellar_fairshare_cluster_used_ratio",
            "Cluster usage / capacity (dominant resource).",
            "gauge",
            format!("{}", total.dominant_share(self.capacity)),
        );
        c("stellar_fairshare_config_valid", "1 if the active quota tree validated; 0 if the latest update was rejected (last good config kept).", "gauge", g(&m.config_valid).to_string());
        c(
            "stellar_fairshare_config_reloads_total",
            "Accepted quota tree updates.",
            "counter",
            g(&m.config_reloads).to_string(),
        );
        c(
            "stellar_fairshare_config_rejected_total",
            "Rejected quota tree updates.",
            "counter",
            g(&m.config_rejected).to_string(),
        );
        let _ = writeln!(o, "# HELP stellar_fairshare_admission_total Admission decisions.\n# TYPE stellar_fairshare_admission_total counter");
        for (k, v) in [
            ("allow_guaranteed", &m.admit_guaranteed),
            ("allow_elastic", &m.admit_elastic),
            ("deny_max", &m.deny_max),
            ("deny_no_capacity", &m.deny_capacity),
            ("needs_reclaim", &m.needs_reclaim),
        ] {
            let _ = writeln!(
                o,
                "stellar_fairshare_admission_total{{decision=\"{k}\"}} {}",
                g(v)
            );
        }
        let _ = writeln!(o, "# HELP stellar_fairshare_preemption_total Preemption decisions.\n# TYPE stellar_fairshare_preemption_total counter");
        for (k, v) in [
            ("approved", &m.preempt_approved),
            ("refused_exceeds_guarantee", &m.preempt_refused_guarantee),
            ("refused_protected_victim", &m.preempt_refused_protected),
            ("refused_insufficient", &m.preempt_refused_insufficient),
        ] {
            let _ = writeln!(
                o,
                "stellar_fairshare_preemption_total{{decision=\"{k}\"}} {}",
                g(v)
            );
        }
        let name = "stellar_fairshare_scheduling_delay_seconds";
        let _ = writeln!(
            o,
            "# HELP {name} Pod creation to first admitted filter.\n# TYPE {name} histogram"
        );
        for (i, b) in LAT_BUCKETS.iter().enumerate() {
            let _ = writeln!(o, "{name}_bucket{{le=\"{b}\"}} {}", g(&m.lat[i]));
        }
        let _ = writeln!(
            o,
            "{name}_bucket{{le=\"+Inf\"}} {}\n{name}_sum {}\n{name}_count {}",
            g(&m.lat_count),
            g(&m.lat_sum_ms) as f64 / 1000.0,
            g(&m.lat_count)
        );
        o
    }
}
