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
//! Cost-aware capacity-class placement (#1484).
//!
//! Reuses existing scheduler affinity, topology-spread, and node-resource
//! models. Critical workloads are hard-filtered off spot; best-effort
//! workloads bin-pack onto spot while still honoring topology and affinity.

use std::collections::HashMap;

use k8s_openapi::api::core::v1::{Node, Pod};

use crate::crd::{CapacityClass, NodeType, StellarNode, WorkloadTier};

use super::affinity::{AffinityProcessor, AffinityRule, PodPlacement};
use super::optimizer::NodeResources;

/// Node label keys that encode capacity class. First match wins.
pub const CAPACITY_CLASS_LABELS: &[&str] = &[
    "stellar.org/capacity-class",
    "node.kubernetes.io/lifecycle",
    "node.kubernetes.io/capacity-type",
    "eks.amazonaws.com/capacityType",
    "cloud.google.com/gke-preemptible",
    "kubernetes.azure.com/scalesetpriority",
];

/// Workload-tier label / annotation keys.
pub const WORKLOAD_TIER_LABEL: &str = "stellar.org/workload-tier";
pub const WORKLOAD_TIER_ANNOTATION: &str = "stellar.org/workload-tier";
pub const WORKLOAD_TYPE_LABEL: &str = "stellar.org/workload-type";

/// Minimum share of eligible best-effort pods that must land on spot.
pub const BEST_EFFORT_SPOT_TARGET_RATIO: f64 = 0.70;

/// Default on-demand hourly USD used when a node has no cost annotation.
pub const DEFAULT_ON_DEMAND_HOURLY_USD: f64 = 0.096;
/// Default spot discount versus on-demand (industry-typical ~70%).
pub const DEFAULT_SPOT_DISCOUNT: f64 = 0.70;

/// Workload description used by the placement engine.
#[derive(Debug, Clone)]
pub struct WorkloadRequest {
    pub name: String,
    pub tier: WorkloadTier,
    pub cpu_milli: u64,
    pub memory_mb: u64,
    pub labels: HashMap<String, String>,
    pub affinity: Vec<AffinityRule>,
    pub anti_affinity_selector: Option<String>,
    pub topology_key: Option<String>,
    pub topology_max_skew: i32,
}

/// Result of placing one workload onto a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementResult {
    pub workload: String,
    pub node_name: String,
    pub capacity_class: CapacityClass,
    pub tier: WorkloadTier,
}

/// Cluster-wide placement outcome plus acceptance measurements.
#[derive(Debug, Clone, Default)]
pub struct PlacementReport {
    pub placements: Vec<PlacementResult>,
    pub unschedulable: Vec<String>,
}

impl PlacementReport {
    /// Fraction of best-effort pods that landed on spot (0.0 if none).
    pub fn best_effort_spot_ratio(&self) -> f64 {
        let best: Vec<_> = self
            .placements
            .iter()
            .filter(|p| p.tier == WorkloadTier::BestEffort)
            .collect();
        if best.is_empty() {
            return 0.0;
        }
        let on_spot = best
            .iter()
            .filter(|p| p.capacity_class == CapacityClass::Spot)
            .count();
        on_spot as f64 / best.len() as f64
    }

    /// Number of critical-tier pods that landed on spot (must be zero).
    pub fn critical_on_spot(&self) -> usize {
        self.placements
            .iter()
            .filter(|p| p.tier == WorkloadTier::Critical && p.capacity_class == CapacityClass::Spot)
            .count()
    }

    pub fn meets_spot_target(&self) -> bool {
        self.best_effort_spot_ratio() + f64::EPSILON >= BEST_EFFORT_SPOT_TARGET_RATIO
    }
}

/// Infer capacity class from a node label map.
pub fn capacity_class_from_labels(labels: &HashMap<String, String>) -> CapacityClass {
    for key in CAPACITY_CLASS_LABELS {
        if let Some(value) = labels.get(*key) {
            if *key == "cloud.google.com/gke-preemptible" {
                if value.eq_ignore_ascii_case("true") {
                    return CapacityClass::Spot;
                }
                continue;
            }
            if *key == "kubernetes.azure.com/scalesetpriority" && value.eq_ignore_ascii_case("spot")
            {
                return CapacityClass::Spot;
            }
            if *key == "eks.amazonaws.com/capacityType" && value.eq_ignore_ascii_case("SPOT") {
                return CapacityClass::Spot;
            }
            if let Some(class) = CapacityClass::parse_label(value) {
                return class;
            }
        }
    }
    CapacityClass::OnDemand
}

/// Infer capacity class from a Kubernetes node.
pub fn capacity_class_from_node(node: &Node) -> CapacityClass {
    let labels = node
        .metadata
        .labels
        .as_ref()
        .map(|l| l.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    capacity_class_from_labels(&labels)
}

/// True when the node is allowed for this tier (hard constraint).
pub fn is_capacity_allowed(tier: WorkloadTier, class: CapacityClass) -> bool {
    !matches!((tier, class), (WorkloadTier::Critical, CapacityClass::Spot))
}

/// Classify a StellarNode using explicit placement config, then node type.
pub fn classify_stellar_node(node: &StellarNode) -> WorkloadTier {
    if let Some(tier) = node.spec.placement.workload_tier {
        return tier;
    }
    classify_node_type(node.spec.node_type.clone())
}

/// Default tier by Stellar node type (matches `config/spot-instances.yaml`).
pub fn classify_node_type(node_type: NodeType) -> WorkloadTier {
    match node_type {
        NodeType::Validator | NodeType::Horizon => WorkloadTier::Critical,
        NodeType::SorobanRpc => WorkloadTier::Critical,
    }
}

/// Classify a pod from labels / annotations, then node-type fallback.
pub fn classify_pod(pod: &Pod) -> WorkloadTier {
    if let Some(labels) = pod.metadata.labels.as_ref() {
        if let Some(tier) = labels
            .get(WORKLOAD_TIER_LABEL)
            .and_then(|v| WorkloadTier::parse_label(v))
        {
            return tier;
        }
        if let Some(wt) = labels.get(WORKLOAD_TYPE_LABEL) {
            if wt.eq_ignore_ascii_case("spot-eligible")
                || wt.eq_ignore_ascii_case("best-effort")
                || wt.eq_ignore_ascii_case("indexer")
            {
                return WorkloadTier::BestEffort;
            }
        }
        if labels.get("stellar.org/role").map(|s| s.as_str()) == Some("read-replica") {
            return WorkloadTier::BestEffort;
        }
        if let Some(nt) = labels.get("stellar.org/node-type") {
            if nt.eq_ignore_ascii_case("Validator") || nt.eq_ignore_ascii_case("Horizon") {
                return WorkloadTier::Critical;
            }
        }
    }
    if let Some(annotations) = pod.metadata.annotations.as_ref() {
        if let Some(tier) = annotations
            .get(WORKLOAD_TIER_ANNOTATION)
            .and_then(|v| WorkloadTier::parse_label(v))
        {
            return tier;
        }
    }
    WorkloadTier::BestEffort
}

/// Hard-filter nodes by capacity class for a workload tier.
pub fn filter_nodes_for_tier<'a>(
    nodes: &'a [NodeResources],
    tier: WorkloadTier,
) -> Vec<&'a NodeResources> {
    nodes
        .iter()
        .filter(|n| is_capacity_allowed(tier, capacity_class_from_labels(&n.labels)))
        .collect()
}

/// Filter Kubernetes nodes that are schedulable and capacity-legal for `pod`.
pub fn filter_k8s_nodes<'a>(pod: &Pod, nodes: &'a [Node]) -> Vec<&'a Node> {
    let tier = classify_pod(pod);
    nodes
        .iter()
        .filter(|n| {
            if n.spec
                .as_ref()
                .and_then(|s| s.unschedulable)
                .unwrap_or(false)
            {
                return false;
            }
            is_capacity_allowed(tier, capacity_class_from_node(n))
        })
        .collect()
}

/// Score a feasible node: best-effort prefers spot; critical prefers on-demand.
pub fn score_capacity_fit(tier: WorkloadTier, class: CapacityClass, hourly_cost_usd: f64) -> f64 {
    let class_bonus = match (tier, class) {
        (WorkloadTier::BestEffort, CapacityClass::Spot) => 1.0,
        (WorkloadTier::BestEffort, CapacityClass::OnDemand) => 0.25,
        (WorkloadTier::Critical, CapacityClass::OnDemand) => 1.0,
        (WorkloadTier::Critical, CapacityClass::Spot) => -1.0,
    };
    let cost_score = 1.0 - (hourly_cost_usd / 5.0).clamp(0.0, 1.0);
    class_bonus * 10.0 + cost_score
}

/// Place workloads onto nodes while respecting capacity class, affinity,
/// anti-affinity, and topology spread. Does not invent extra node pools.
pub fn place_workloads(
    workloads: &[WorkloadRequest],
    nodes: &[NodeResources],
    existing: &[PodPlacement],
) -> PlacementReport {
    let mut report = PlacementReport::default();
    let mut remaining = nodes.to_vec();
    let mut placements = existing.to_vec();

    for wl in workloads {
        match select_node(wl, &remaining, &placements) {
            Some(node_name) => {
                let idx = remaining.iter().position(|n| n.name == node_name);
                if let Some(i) = idx {
                    remaining[i].used_cpu_milli += wl.cpu_milli;
                    remaining[i].used_memory_mb += wl.memory_mb;
                    let class = capacity_class_from_labels(&remaining[i].labels);
                    placements.push(PodPlacement {
                        pod_name: wl.name.clone(),
                        node_name: node_name.clone(),
                        labels: wl.labels.clone(),
                    });
                    report.placements.push(PlacementResult {
                        workload: wl.name.clone(),
                        node_name,
                        capacity_class: class,
                        tier: wl.tier,
                    });
                }
            }
            None => report.unschedulable.push(wl.name.clone()),
        }
    }
    report
}

fn select_node(
    wl: &WorkloadRequest,
    nodes: &[NodeResources],
    existing: &[PodPlacement],
) -> Option<String> {
    let capacity_ok: Vec<NodeResources> = filter_nodes_for_tier(nodes, wl.tier)
        .into_iter()
        .cloned()
        .collect();

    let affinity_ok: Vec<NodeResources> =
        AffinityProcessor::filter_by_affinity(&capacity_ok, &wl.affinity)
            .into_iter()
            .cloned()
            .collect();

    let anti_ok: Vec<NodeResources> = if let Some(selector) = &wl.anti_affinity_selector {
        AffinityProcessor::filter_by_anti_affinity(&affinity_ok, selector, existing)
            .into_iter()
            .cloned()
            .collect()
    } else {
        affinity_ok
    };

    let topology_ok: Vec<NodeResources> = if let Some(key) = &wl.topology_key {
        filter_topology_spread(&anti_ok, existing, nodes, key, wl.topology_max_skew)
    } else {
        anti_ok
    };

    let mut feasible: Vec<&NodeResources> = topology_ok
        .iter()
        .filter(|n| n.free_cpu() >= wl.cpu_milli && n.free_memory_mb() >= wl.memory_mb)
        .collect();

    if feasible.is_empty() {
        return None;
    }

    feasible.sort_by(|a, b| {
        let sa = score_capacity_fit(
            wl.tier,
            capacity_class_from_labels(&a.labels),
            a.hourly_cost_usd,
        );
        let sb = score_capacity_fit(
            wl.tier,
            capacity_class_from_labels(&b.labels),
            b.hourly_cost_usd,
        );
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    feasible.first().map(|n| n.name.clone())
}

fn filter_topology_spread(
    candidates: &[NodeResources],
    existing: &[PodPlacement],
    all_nodes: &[NodeResources],
    topology_key: &str,
    max_skew: i32,
) -> Vec<NodeResources> {
    let skew = AffinityProcessor::compute_topology_skew(existing, all_nodes, topology_key);
    let min_count = skew.values().copied().min().unwrap_or(0);
    candidates
        .iter()
        .filter(|n| {
            let zone = n
                .labels
                .get(topology_key)
                .cloned()
                .unwrap_or_else(|| n.zone.clone());
            let count = *skew.get(&zone).unwrap_or(&0);
            count - min_count < max_skew || count == min_count
        })
        .cloned()
        .collect()
}

/// Pick the best already-filtered k8s node for a pod (used by the scheduler).
pub fn pick_preferred_k8s_node<'a>(pod: &Pod, candidates: &[&'a Node]) -> Option<&'a Node> {
    let tier = classify_pod(pod);
    let mut scored: Vec<(&Node, f64)> = candidates
        .iter()
        .map(|n| {
            let class = capacity_class_from_node(n);
            let cost = node_hourly_cost(n);
            (*n, score_capacity_fit(tier, class, cost))
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.first().map(|(n, _)| *n)
}

fn node_hourly_cost(node: &Node) -> f64 {
    node.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("stellar.org/hourly-cost-usd"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_ON_DEMAND_HOURLY_USD)
}

/// Hourly USD cost of a node given its capacity class.
pub fn hourly_cost_for_class(class: CapacityClass, on_demand_hourly: f64) -> f64 {
    match class {
        CapacityClass::OnDemand => on_demand_hourly,
        CapacityClass::Spot => on_demand_hourly * (1.0 - DEFAULT_SPOT_DISCOUNT),
    }
}

/// Realized hourly savings versus running the same work on on-demand.
pub fn realized_hourly_savings(class: CapacityClass, on_demand_hourly: f64) -> f64 {
    match class {
        CapacityClass::Spot => on_demand_hourly * DEFAULT_SPOT_DISCOUNT,
        CapacityClass::OnDemand => 0.0,
    }
}

/// Helper for tests / simulations: labeled [`NodeResources`].
pub fn labeled_node(
    name: &str,
    class: CapacityClass,
    zone: &str,
    cpu_milli: u64,
    memory_mb: u64,
    hourly: f64,
) -> NodeResources {
    let mut labels = HashMap::new();
    labels.insert(
        "stellar.org/capacity-class".to_string(),
        class.as_label().to_string(),
    );
    labels.insert(
        "node.kubernetes.io/lifecycle".to_string(),
        class.as_label().to_string(),
    );
    labels.insert("topology.kubernetes.io/zone".to_string(), zone.to_string());
    NodeResources {
        name: name.to_string(),
        allocatable_cpu_milli: cpu_milli,
        allocatable_memory_mb: memory_mb,
        used_cpu_milli: 0,
        used_memory_mb: 0,
        zone: zone.to_string(),
        region: "us-east-1".to_string(),
        hourly_cost_usd: hourly,
        labels,
        taints: Vec::new(),
    }
}

/// Best-effort request used by the defined test workload.
pub fn best_effort_workload(name: &str) -> WorkloadRequest {
    WorkloadRequest {
        name: name.to_string(),
        tier: WorkloadTier::BestEffort,
        cpu_milli: 250,
        memory_mb: 256,
        labels: HashMap::from([(
            WORKLOAD_TIER_LABEL.to_string(),
            WorkloadTier::BestEffort.as_label().to_string(),
        )]),
        affinity: Vec::new(),
        anti_affinity_selector: None,
        topology_key: Some("topology.kubernetes.io/zone".to_string()),
        topology_max_skew: 1,
    }
}

/// Critical request used by the defined test workload.
pub fn critical_workload(name: &str) -> WorkloadRequest {
    WorkloadRequest {
        name: name.to_string(),
        tier: WorkloadTier::Critical,
        cpu_milli: 500,
        memory_mb: 512,
        labels: HashMap::from([(
            WORKLOAD_TIER_LABEL.to_string(),
            WorkloadTier::Critical.as_label().to_string(),
        )]),
        affinity: Vec::new(),
        anti_affinity_selector: None,
        topology_key: Some("topology.kubernetes.io/zone".to_string()),
        topology_max_skew: 1,
    }
}

/// Defined mixed test cluster: majority spot, enough on-demand for critical.
pub fn defined_test_cluster() -> Vec<NodeResources> {
    vec![
        labeled_node(
            "spot-a",
            CapacityClass::Spot,
            "us-east-1a",
            8000,
            16_384,
            hourly_cost_for_class(CapacityClass::Spot, DEFAULT_ON_DEMAND_HOURLY_USD),
        ),
        labeled_node(
            "spot-b",
            CapacityClass::Spot,
            "us-east-1b",
            8000,
            16_384,
            hourly_cost_for_class(CapacityClass::Spot, DEFAULT_ON_DEMAND_HOURLY_USD),
        ),
        labeled_node(
            "spot-c",
            CapacityClass::Spot,
            "us-east-1c",
            8000,
            16_384,
            hourly_cost_for_class(CapacityClass::Spot, DEFAULT_ON_DEMAND_HOURLY_USD),
        ),
        labeled_node(
            "od-a",
            CapacityClass::OnDemand,
            "us-east-1a",
            8000,
            16_384,
            DEFAULT_ON_DEMAND_HOURLY_USD,
        ),
        labeled_node(
            "od-b",
            CapacityClass::OnDemand,
            "us-east-1b",
            8000,
            16_384,
            DEFAULT_ON_DEMAND_HOURLY_USD,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::affinity::{AffinityOperator, AffinityRule};
    use super::*;

    #[test]
    fn critical_never_schedules_onto_spot() {
        let nodes = defined_test_cluster();
        let workloads: Vec<_> = (0..10)
            .map(|i| critical_workload(&format!("crit-{i}")))
            .collect();
        let report = place_workloads(&workloads, &nodes, &[]);
        assert_eq!(report.critical_on_spot(), 0);
        assert!(report.unschedulable.is_empty());
        assert!(report
            .placements
            .iter()
            .all(|p| p.capacity_class == CapacityClass::OnDemand));
    }

    #[test]
    fn best_effort_prefers_spot_above_target() {
        let nodes = defined_test_cluster();
        let workloads: Vec<_> = (0..20)
            .map(|i| best_effort_workload(&format!("be-{i}")))
            .collect();
        let report = place_workloads(&workloads, &nodes, &[]);
        assert!(
            report.meets_spot_target(),
            "spot ratio {:.2} < 0.70",
            report.best_effort_spot_ratio()
        );
        assert!(report.best_effort_spot_ratio() >= BEST_EFFORT_SPOT_TARGET_RATIO);
        assert_eq!(report.critical_on_spot(), 0);
    }

    #[test]
    fn topology_spread_respected_for_best_effort() {
        let nodes = defined_test_cluster();
        let workloads: Vec<_> = (0..6)
            .map(|i| best_effort_workload(&format!("be-{i}")))
            .collect();
        let report = place_workloads(&workloads, &nodes, &[]);
        let mut zone_counts: HashMap<String, i32> = HashMap::new();
        for p in &report.placements {
            let node = nodes.iter().find(|n| n.name == p.node_name).unwrap();
            *zone_counts.entry(node.zone.clone()).or_default() += 1;
        }
        let max = *zone_counts.values().max().unwrap_or(&0);
        let min = *zone_counts.values().min().unwrap_or(&0);
        assert!(
            max - min <= 1,
            "topology skew {max}-{min} exceeded maxSkew=1: {zone_counts:?}"
        );
    }

    #[test]
    fn required_affinity_is_respected() {
        let mut nodes = defined_test_cluster();
        nodes[0]
            .labels
            .insert("disk".to_string(), "ssd".to_string());
        let mut wl = best_effort_workload("pinned");
        wl.affinity.push(AffinityRule {
            key: "disk".to_string(),
            operator: AffinityOperator::In,
            values: vec!["ssd".to_string()],
            required: true,
        });
        let report = place_workloads(&[wl], &nodes, &[]);
        assert_eq!(report.placements.len(), 1);
        assert_eq!(report.placements[0].node_name, "spot-a");
    }

    #[test]
    fn anti_affinity_excludes_occupied_nodes() {
        let nodes = defined_test_cluster();
        let existing = vec![PodPlacement {
            pod_name: "other".to_string(),
            node_name: "spot-a".to_string(),
            labels: HashMap::from([("app".to_string(), "indexer".to_string())]),
        }];
        let mut wl = best_effort_workload("next");
        wl.anti_affinity_selector = Some("app=indexer".to_string());
        let report = place_workloads(&[wl], &nodes, &existing);
        assert_eq!(report.placements.len(), 1);
        assert_ne!(report.placements[0].node_name, "spot-a");
    }

    #[test]
    fn mixed_workload_meets_acceptance() {
        let nodes = defined_test_cluster();
        let mut workloads = Vec::new();
        for i in 0..8 {
            workloads.push(critical_workload(&format!("c-{i}")));
        }
        for i in 0..40 {
            workloads.push(best_effort_workload(&format!("b-{i}")));
        }
        let report = place_workloads(&workloads, &nodes, &[]);
        assert_eq!(report.critical_on_spot(), 0);
        assert!(report.meets_spot_target());
        assert!(report.unschedulable.is_empty());
    }

    #[test]
    fn unlabeled_nodes_are_on_demand() {
        let labels = HashMap::new();
        assert_eq!(capacity_class_from_labels(&labels), CapacityClass::OnDemand);
    }

    #[test]
    fn classify_pod_spot_eligible_is_best_effort() {
        let pod = Pod {
            metadata: kube::api::ObjectMeta {
                labels: Some(BTreeMap::from([(
                    WORKLOAD_TYPE_LABEL.to_string(),
                    "spot-eligible".to_string(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(classify_pod(&pod), WorkloadTier::BestEffort);
    }

    #[test]
    fn classify_pod_validator_is_critical() {
        let pod = Pod {
            metadata: kube::api::ObjectMeta {
                labels: Some(BTreeMap::from([(
                    "stellar.org/node-type".to_string(),
                    "Validator".to_string(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(classify_pod(&pod), WorkloadTier::Critical);
    }

    #[test]
    fn critical_unschedulable_when_only_spot_exists() {
        let nodes = vec![labeled_node(
            "spot-only",
            CapacityClass::Spot,
            "us-east-1a",
            8000,
            16_384,
            hourly_cost_for_class(CapacityClass::Spot, DEFAULT_ON_DEMAND_HOURLY_USD),
        )];
        let report = place_workloads(&[critical_workload("validator")], &nodes, &[]);
        assert!(report.placements.is_empty());
        assert_eq!(report.unschedulable, vec!["validator".to_string()]);
        assert_eq!(report.critical_on_spot(), 0);
    }

    #[test]
    fn measured_acceptance_mixed_cluster() {
        let nodes = defined_test_cluster();
        let mut workloads = Vec::new();
        for i in 0..8 {
            workloads.push(critical_workload(&format!("c-{i}")));
        }
        for i in 0..40 {
            workloads.push(best_effort_workload(&format!("b-{i}")));
        }
        let report = place_workloads(&workloads, &nodes, &[]);
        let ratio = report.best_effort_spot_ratio();
        assert_eq!(report.critical_on_spot(), 0);
        assert!(
            ratio >= BEST_EFFORT_SPOT_TARGET_RATIO,
            "measured best-effort spot ratio {ratio:.4} below 0.70"
        );
        assert!(report.unschedulable.is_empty());
    }
}
