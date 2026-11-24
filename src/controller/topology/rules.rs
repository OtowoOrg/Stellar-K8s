//! Topology spread constraint rule generation for Stellar node StatefulSets.
//!
//! This module defines the rule types and generation logic used by the
//! [`super::enforcer`] to build `topologySpreadConstraints` and pod
//! anti-affinity rules that prevent multiple Stellar node pods from landing on
//! the same physical host or availability zone.
//!
//! # Rule hierarchy
//!
//! | Mode                | When applied                          | k8s semantics          |
//! |---------------------|---------------------------------------|------------------------|
//! | `HardZoneSpread`    | ≥ 3 zones detected                   | `DoNotSchedule`        |
//! | `SoftZoneSpread`    | 1–2 zones (e.g. local dev cluster)   | `ScheduleAnyway`       |
//! | `HardHostAntiAff`  | always                                | `requiredDuringSchedul…` |
//! | `SoftHostAntiAff`  | single-zone fallback                  | `preferredDuringSchedul…`|
//!
//! # Label keys
//!
//! The well-known Kubernetes topology labels are used:
//! - Zone:  `topology.kubernetes.io/zone`
//! - Host:  `kubernetes.io/hostname`

use k8s_openapi::api::core::v1::{
    NodeSelectorRequirement, NodeSelectorTerm, PodAffinityTerm, PodAntiAffinity,
    WeightedPodAffinityTerm,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Public constants ──────────────────────────────────────────────────────────

/// Well-known Kubernetes label that identifies the failure-domain zone.
pub const TOPOLOGY_ZONE_KEY: &str = "topology.kubernetes.io/zone";

/// Well-known Kubernetes label that identifies the individual node hostname.
pub const TOPOLOGY_HOST_KEY: &str = "kubernetes.io/hostname";

/// Minimum number of distinct availability zones required to enforce hard zone
/// spread rules.  Clusters with fewer zones fall back to soft rules.
pub const MIN_ZONES_FOR_HARD_SPREAD: usize = 3;

// ── Zone detection ────────────────────────────────────────────────────────────

/// Detected topology of the cluster, derived by inspecting live node labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterTopology {
    /// All distinct zone values found across all schedulable nodes.
    pub zones: Vec<String>,
    /// Total number of schedulable nodes.
    pub node_count: usize,
}

impl ClusterTopology {
    /// Return `true` when there are enough distinct zones to enforce hard spread.
    pub fn has_multi_zone(&self) -> bool {
        self.zones.len() >= MIN_ZONES_FOR_HARD_SPREAD
    }

    /// Return `true` for single-zone setups (local dev / staging clusters).
    pub fn is_single_zone(&self) -> bool {
        self.zones.len() <= 1
    }
}

// ── Spread-constraint unsatisfiable action ────────────────────────────────────

/// Maps to the Kubernetes `whenUnsatisfiable` field of a `TopologySpreadConstraint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WhenUnsatisfiable {
    /// Fail scheduling if the spread cannot be satisfied (hard rule).
    DoNotSchedule,
    /// Allow the pod to be placed on any node but record the violation (soft rule).
    ScheduleAnyway,
}

impl WhenUnsatisfiable {
    pub fn as_str(self) -> &'static str {
        match self {
            WhenUnsatisfiable::DoNotSchedule => "DoNotSchedule",
            WhenUnsatisfiable::ScheduleAnyway => "ScheduleAnyway",
        }
    }
}

// ── TopologySpreadConstraint (raw JSON representation) ───────────────────────
// k8s-openapi 0.22 exposes TopologySpreadConstraint only inside pod specs via
// serde_json values, so we model it here for clean generation and testing.

/// A single `topologySpreadConstraint` entry as it would appear in a pod spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TopologySpreadConstraint {
    /// Maximum skew between the most and least loaded topology domain.
    pub max_skew: i32,
    /// The label key whose values define the topology domains.
    pub topology_key: String,
    /// Scheduling action when the constraint cannot be satisfied.
    pub when_unsatisfiable: String,
    /// Pod label selector used to count existing pods in each domain.
    pub label_selector: BTreeMap<String, String>,
}

impl TopologySpreadConstraint {
    /// Build a zone-spread constraint for the given node selector labels.
    pub fn zone_spread(
        selector_labels: BTreeMap<String, String>,
        when_unsatisfiable: WhenUnsatisfiable,
    ) -> Self {
        Self {
            max_skew: 1,
            topology_key: TOPOLOGY_ZONE_KEY.to_string(),
            when_unsatisfiable: when_unsatisfiable.as_str().to_string(),
            label_selector: selector_labels,
        }
    }

    /// Build a per-host spread constraint for the given node selector labels.
    pub fn host_spread(
        selector_labels: BTreeMap<String, String>,
        when_unsatisfiable: WhenUnsatisfiable,
    ) -> Self {
        Self {
            max_skew: 1,
            topology_key: TOPOLOGY_HOST_KEY.to_string(),
            when_unsatisfiable: when_unsatisfiable.as_str().to_string(),
            label_selector: selector_labels,
        }
    }
}

// ── Anti-affinity helpers ─────────────────────────────────────────────────────

/// Generate a strict (required) `podAntiAffinity` rule that prevents two pods
/// with `selector_labels` from being co-located on the same host.
pub fn hard_host_anti_affinity(
    selector_labels: BTreeMap<String, String>,
    namespace: Option<&str>,
) -> PodAntiAffinity {
    let namespaces = namespace.map(|ns| vec![ns.to_string()]);
    let label_selector = label_selector_from_map(&selector_labels);

    PodAntiAffinity {
        required_during_scheduling_ignored_during_execution: Some(vec![PodAffinityTerm {
            label_selector: Some(label_selector),
            topology_key: TOPOLOGY_HOST_KEY.to_string(),
            namespaces,
            ..Default::default()
        }]),
        preferred_during_scheduling_ignored_during_execution: None,
    }
}

/// Generate a soft (preferred) `podAntiAffinity` rule that tries to spread pods
/// across hosts but will not block scheduling if no suitable host is found.
pub fn soft_host_anti_affinity(
    selector_labels: BTreeMap<String, String>,
    namespace: Option<&str>,
    weight: i32,
) -> PodAntiAffinity {
    let namespaces = namespace.map(|ns| vec![ns.to_string()]);
    let label_selector = label_selector_from_map(&selector_labels);

    PodAntiAffinity {
        required_during_scheduling_ignored_during_execution: None,
        preferred_during_scheduling_ignored_during_execution: Some(vec![
            WeightedPodAffinityTerm {
                weight,
                pod_affinity_term: PodAffinityTerm {
                    label_selector: Some(label_selector),
                    topology_key: TOPOLOGY_HOST_KEY.to_string(),
                    namespaces,
                    ..Default::default()
                },
            },
        ]),
    }
}

// ── Node affinity helpers ─────────────────────────────────────────────────────

/// Build a `nodeAffinity.requiredDuringScheduling…` rule that restricts pods
/// to nodes that carry at least one of the known zone labels.
pub fn zone_node_affinity_terms(zones: &[String]) -> Option<Vec<NodeSelectorTerm>> {
    if zones.is_empty() {
        return None;
    }
    let term = NodeSelectorTerm {
        match_expressions: Some(vec![NodeSelectorRequirement {
            key: TOPOLOGY_ZONE_KEY.to_string(),
            operator: "In".to_string(),
            values: Some(zones.to_vec()),
        }]),
        match_fields: None,
    };
    Some(vec![term])
}

// ── Rule set ──────────────────────────────────────────────────────────────────

/// The complete set of topology rules to inject into a StatefulSet pod template.
#[derive(Debug, Clone)]
pub struct TopologyRuleSet {
    /// `topologySpreadConstraints` to inject.
    pub spread_constraints: Vec<TopologySpreadConstraint>,
    /// `podAntiAffinity` to inject.
    pub anti_affinity: PodAntiAffinity,
    /// Human-readable summary of which mode was chosen.
    pub mode: TopologyMode,
}

/// Describes which enforcement mode is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopologyMode {
    /// Hard zone spread + hard host anti-affinity (≥ 3 zones).
    HardMultiZone,
    /// Soft zone spread + soft host anti-affinity (< 3 zones / dev clusters).
    SoftSingleZone,
}

impl std::fmt::Display for TopologyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TopologyMode::HardMultiZone => write!(f, "HardMultiZone"),
            TopologyMode::SoftSingleZone => write!(f, "SoftSingleZone"),
        }
    }
}

/// Build a [`TopologyRuleSet`] appropriate for the given cluster topology.
///
/// # Arguments
///
/// * `topology`        – detected cluster topology (zones + node count)
/// * `selector_labels` – pod labels used for matching in spread constraints
/// * `namespace`       – optional namespace for anti-affinity scoping
pub fn build_rule_set(
    topology: &ClusterTopology,
    selector_labels: BTreeMap<String, String>,
    namespace: Option<&str>,
) -> TopologyRuleSet {
    if topology.has_multi_zone() {
        // ── Hard mode: 3+ zones ────────────────────────────────────────────
        let spread_constraints = vec![
            TopologySpreadConstraint::zone_spread(
                selector_labels.clone(),
                WhenUnsatisfiable::DoNotSchedule,
            ),
            TopologySpreadConstraint::host_spread(
                selector_labels.clone(),
                WhenUnsatisfiable::DoNotSchedule,
            ),
        ];
        let anti_affinity = hard_host_anti_affinity(selector_labels, namespace);
        TopologyRuleSet {
            spread_constraints,
            anti_affinity,
            mode: TopologyMode::HardMultiZone,
        }
    } else {
        // ── Soft mode: single-zone or 2-zone fallback ─────────────────────
        let spread_constraints = vec![TopologySpreadConstraint::host_spread(
            selector_labels.clone(),
            WhenUnsatisfiable::ScheduleAnyway,
        )];
        let anti_affinity = soft_host_anti_affinity(selector_labels, namespace, 100);
        TopologyRuleSet {
            spread_constraints,
            anti_affinity,
            mode: TopologyMode::SoftSingleZone,
        }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn label_selector_from_map(labels: &BTreeMap<String, String>) -> LabelSelector {
    LabelSelector {
        match_labels: Some(labels.clone()),
        match_expressions: None,
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_labels() -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("app.kubernetes.io/name".to_string(), "stellar-node".to_string());
        m.insert("app.kubernetes.io/component".to_string(), "validator".to_string());
        m
    }

    fn three_zone_topology() -> ClusterTopology {
        ClusterTopology {
            zones: vec!["us-east-1a".into(), "us-east-1b".into(), "us-east-1c".into()],
            node_count: 9,
        }
    }

    fn single_zone_topology() -> ClusterTopology {
        ClusterTopology {
            zones: vec!["local".into()],
            node_count: 3,
        }
    }

    // ── ClusterTopology helpers ───────────────────────────────────────────

    #[test]
    fn test_has_multi_zone_true() {
        assert!(three_zone_topology().has_multi_zone());
    }

    #[test]
    fn test_has_multi_zone_false_for_two_zones() {
        let t = ClusterTopology {
            zones: vec!["a".into(), "b".into()],
            node_count: 4,
        };
        assert!(!t.has_multi_zone());
    }

    #[test]
    fn test_is_single_zone() {
        assert!(single_zone_topology().is_single_zone());
    }

    #[test]
    fn test_is_not_single_zone_for_two_zones() {
        let t = ClusterTopology {
            zones: vec!["a".into(), "b".into()],
            node_count: 4,
        };
        assert!(!t.is_single_zone());
    }

    // ── WhenUnsatisfiable ─────────────────────────────────────────────────

    #[test]
    fn test_when_unsatisfiable_as_str() {
        assert_eq!(WhenUnsatisfiable::DoNotSchedule.as_str(), "DoNotSchedule");
        assert_eq!(WhenUnsatisfiable::ScheduleAnyway.as_str(), "ScheduleAnyway");
    }

    // ── TopologySpreadConstraint builders ─────────────────────────────────

    #[test]
    fn test_zone_spread_hard() {
        let c = TopologySpreadConstraint::zone_spread(
            sample_labels(),
            WhenUnsatisfiable::DoNotSchedule,
        );
        assert_eq!(c.topology_key, TOPOLOGY_ZONE_KEY);
        assert_eq!(c.when_unsatisfiable, "DoNotSchedule");
        assert_eq!(c.max_skew, 1);
    }

    #[test]
    fn test_host_spread_soft() {
        let c = TopologySpreadConstraint::host_spread(
            sample_labels(),
            WhenUnsatisfiable::ScheduleAnyway,
        );
        assert_eq!(c.topology_key, TOPOLOGY_HOST_KEY);
        assert_eq!(c.when_unsatisfiable, "ScheduleAnyway");
    }

    // ── build_rule_set: multi-zone (hard mode) ────────────────────────────

    #[test]
    fn test_build_rule_set_multi_zone_produces_two_constraints() {
        let rs = build_rule_set(&three_zone_topology(), sample_labels(), Some("stellar"));
        assert_eq!(rs.spread_constraints.len(), 2);
        assert_eq!(rs.mode, TopologyMode::HardMultiZone);
    }

    #[test]
    fn test_build_rule_set_multi_zone_first_constraint_is_zone() {
        let rs = build_rule_set(&three_zone_topology(), sample_labels(), Some("stellar"));
        assert_eq!(rs.spread_constraints[0].topology_key, TOPOLOGY_ZONE_KEY);
        assert_eq!(rs.spread_constraints[0].when_unsatisfiable, "DoNotSchedule");
    }

    #[test]
    fn test_build_rule_set_multi_zone_second_constraint_is_host() {
        let rs = build_rule_set(&three_zone_topology(), sample_labels(), Some("stellar"));
        assert_eq!(rs.spread_constraints[1].topology_key, TOPOLOGY_HOST_KEY);
        assert_eq!(rs.spread_constraints[1].when_unsatisfiable, "DoNotSchedule");
    }

    #[test]
    fn test_build_rule_set_multi_zone_hard_anti_affinity() {
        let rs = build_rule_set(&three_zone_topology(), sample_labels(), Some("stellar"));
        // Hard mode: required_during_scheduling must be populated
        assert!(rs
            .anti_affinity
            .required_during_scheduling_ignored_during_execution
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false));
        assert!(rs
            .anti_affinity
            .preferred_during_scheduling_ignored_during_execution
            .is_none());
    }

    // ── build_rule_set: single-zone (soft mode) ───────────────────────────

    #[test]
    fn test_build_rule_set_single_zone_produces_one_constraint() {
        let rs = build_rule_set(&single_zone_topology(), sample_labels(), None);
        assert_eq!(rs.spread_constraints.len(), 1);
        assert_eq!(rs.mode, TopologyMode::SoftSingleZone);
    }

    #[test]
    fn test_build_rule_set_single_zone_constraint_is_host_soft() {
        let rs = build_rule_set(&single_zone_topology(), sample_labels(), None);
        assert_eq!(rs.spread_constraints[0].topology_key, TOPOLOGY_HOST_KEY);
        assert_eq!(rs.spread_constraints[0].when_unsatisfiable, "ScheduleAnyway");
    }

    #[test]
    fn test_build_rule_set_single_zone_soft_anti_affinity() {
        let rs = build_rule_set(&single_zone_topology(), sample_labels(), None);
        // Soft mode: preferred_during_scheduling must be populated
        assert!(rs
            .anti_affinity
            .preferred_during_scheduling_ignored_during_execution
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false));
        assert!(rs
            .anti_affinity
            .required_during_scheduling_ignored_during_execution
            .is_none());
    }

    // ── Zone node affinity ────────────────────────────────────────────────

    #[test]
    fn test_zone_node_affinity_terms_empty_zones_returns_none() {
        assert!(zone_node_affinity_terms(&[]).is_none());
    }

    #[test]
    fn test_zone_node_affinity_terms_non_empty() {
        let zones = vec!["us-east-1a".to_string(), "us-east-1b".to_string()];
        let terms = zone_node_affinity_terms(&zones).unwrap();
        assert_eq!(terms.len(), 1);
        let expr = &terms[0].match_expressions.as_ref().unwrap()[0];
        assert_eq!(expr.key, TOPOLOGY_ZONE_KEY);
        assert_eq!(expr.operator, "In");
        assert_eq!(expr.values.as_ref().unwrap().len(), 2);
    }

    // ── hard_host_anti_affinity ───────────────────────────────────────────

    #[test]
    fn test_hard_host_anti_affinity_topology_key() {
        let aff = hard_host_anti_affinity(sample_labels(), Some("stellar"));
        let terms = aff
            .required_during_scheduling_ignored_during_execution
            .unwrap();
        assert_eq!(terms[0].topology_key, TOPOLOGY_HOST_KEY);
    }

    #[test]
    fn test_hard_host_anti_affinity_namespace_scoped() {
        let aff = hard_host_anti_affinity(sample_labels(), Some("stellar"));
        let terms = aff
            .required_during_scheduling_ignored_during_execution
            .unwrap();
        assert_eq!(terms[0].namespaces, Some(vec!["stellar".to_string()]));
    }

    // ── soft_host_anti_affinity ───────────────────────────────────────────

    #[test]
    fn test_soft_host_anti_affinity_weight() {
        let aff = soft_host_anti_affinity(sample_labels(), None, 100);
        let terms = aff
            .preferred_during_scheduling_ignored_during_execution
            .unwrap();
        assert_eq!(terms[0].weight, 100);
        assert_eq!(terms[0].pod_affinity_term.topology_key, TOPOLOGY_HOST_KEY);
    }

    // ── TopologyMode display ──────────────────────────────────────────────

    #[test]
    fn test_topology_mode_display() {
        assert_eq!(TopologyMode::HardMultiZone.to_string(), "HardMultiZone");
        assert_eq!(TopologyMode::SoftSingleZone.to_string(), "SoftSingleZone");
    }
}
