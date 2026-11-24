//! Dynamic Node Topology Spread Constraint Enforcer
//!
//! This module implements a mutating controller that:
//! 1. Inspects all schedulable cluster nodes for availability-zone labels.
//! 2. Derives the appropriate [`TopologyRuleSet`] (hard vs. soft) using
//!    [`crate::controller::topology::rules`].
//! 3. Patches the pod-template section of a Stellar-node `StatefulSet` to
//!    inject `topologySpreadConstraints` and `podAntiAffinity` rules.
//!
//! # Behaviour
//!
//! | Detected zones | Action                                         |
//! |----------------|------------------------------------------------|
//! | ≥ 3            | Hard `DoNotSchedule` zone + host spread        |
//! | 1–2            | Soft `ScheduleAnyway` host spread (dev mode)   |
//! | 0              | Soft host spread (no zone labels present)      |
//!
//! The controller is idempotent — it computes the desired constraints on every
//! call and applies a server-side `Patch::Merge` so repeated invocations are
//! no-ops when nothing has changed.
//!
//! # Related modules
//!
//! - [`super::rules`] — rule generation logic and types
//! - `controller/resources.rs` — StatefulSet resource naming helpers

use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::Node;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use serde_json::{json, Value};
use tracing::{debug, info, instrument, warn};

use crate::error::{Error, Result};

use super::rules::{
    build_rule_set, zone_node_affinity_terms, ClusterTopology, TopologyMode, TopologyRuleSet,
    TOPOLOGY_ZONE_KEY,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Field manager name used in server-side apply operations.
const FIELD_MANAGER: &str = "stellar-topology-enforcer";

/// Label applied to StatefulSets after enforcement to record the active mode.
const TOPOLOGY_MODE_ANNOTATION: &str = "stellar.org/topology-mode";

/// Label applied to StatefulSets to record the zone count that was observed.
const TOPOLOGY_ZONES_ANNOTATION: &str = "stellar.org/topology-zone-count";

// ── Public API ────────────────────────────────────────────────────────────────

/// Result of a single topology-enforcement run.
#[derive(Debug, Clone)]
pub struct EnforcementResult {
    /// Name of the StatefulSet that was inspected / patched.
    pub statefulset_name: String,
    /// Namespace of the StatefulSet.
    pub namespace: String,
    /// Topology mode that was selected.
    pub mode: TopologyMode,
    /// Zones that were detected during this run.
    pub zones: Vec<String>,
    /// Whether the patch was actually applied (`true`) or was a no-op (`false`).
    pub patched: bool,
}

/// Discover all availability zones known to the cluster by reading node labels.
///
/// Only nodes that are `Ready` and not `Unschedulable` are considered.
///
/// # Errors
///
/// Returns [`Error::KubeError`] if the node list API call fails.
#[instrument(skip(client))]
pub async fn discover_cluster_topology(client: &Client) -> Result<ClusterTopology> {
    let node_api: Api<Node> = Api::all(client.clone());
    let nodes = node_api
        .list(&kube::api::ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    let mut zones: Vec<String> = Vec::new();
    let mut schedulable_count = 0usize;

    for node in &nodes.items {
        // Skip nodes marked as unschedulable
        if node
            .spec
            .as_ref()
            .and_then(|s| s.unschedulable)
            .unwrap_or(false)
        {
            debug!("Skipping unschedulable node {}", node.name_any());
            continue;
        }

        // Only count nodes that are Ready
        let is_ready = node
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .map(|conds| {
                conds
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            })
            .unwrap_or(false);

        if !is_ready {
            debug!("Skipping non-ready node {}", node.name_any());
            continue;
        }

        schedulable_count += 1;

        if let Some(zone) = node
            .labels()
            .get(TOPOLOGY_ZONE_KEY)
            .map(|z| z.to_string())
        {
            if !zones.contains(&zone) {
                zones.push(zone);
            }
        }
    }

    zones.sort();

    info!(
        "Discovered {} schedulable nodes across {} zone(s): {:?}",
        schedulable_count,
        zones.len(),
        zones
    );

    Ok(ClusterTopology {
        zones,
        node_count: schedulable_count,
    })
}

/// Build the pod-template JSON patch that injects topology rules.
///
/// Returns a [`serde_json::Value`] suitable for [`Patch::Merge`].
pub fn build_statefulset_patch(
    rule_set: &TopologyRuleSet,
    zone_count: usize,
) -> Result<Value> {
    // Serialize spread constraints to plain JSON objects
    let spread_constraints: Vec<Value> = rule_set
        .spread_constraints
        .iter()
        .map(|c| {
            json!({
                "maxSkew": c.max_skew,
                "topologyKey": c.topology_key,
                "whenUnsatisfiable": c.when_unsatisfiable,
                "labelSelector": {
                    "matchLabels": c.label_selector
                }
            })
        })
        .collect();

    // Serialize anti-affinity
    let anti_affinity =
        serde_json::to_value(&rule_set.anti_affinity).map_err(Error::SerializationError)?;

    let patch = json!({
        "metadata": {
            "annotations": {
                TOPOLOGY_MODE_ANNOTATION: rule_set.mode.to_string(),
                TOPOLOGY_ZONES_ANNOTATION: zone_count.to_string()
            }
        },
        "spec": {
            "template": {
                "spec": {
                    "topologySpreadConstraints": spread_constraints,
                    "affinity": {
                        "podAntiAffinity": anti_affinity
                    }
                }
            }
        }
    });

    Ok(patch)
}

/// Enforce topology spread constraints on a specific StatefulSet.
///
/// Looks up the StatefulSet, derives the correct rule set for the given
/// topology, then applies the patch.  Returns an [`EnforcementResult`]
/// describing what happened.
///
/// # Arguments
///
/// * `client`    – Kubernetes API client
/// * `namespace` – namespace of the StatefulSet
/// * `name`      – name of the StatefulSet
/// * `topology`  – pre-fetched cluster topology
/// * `dry_run`   – if `true`, compute but do not apply the patch
#[instrument(skip(client, topology), fields(name = %name, namespace = %namespace))]
pub async fn enforce_on_statefulset(
    client: &Client,
    namespace: &str,
    name: &str,
    topology: &ClusterTopology,
    dry_run: bool,
) -> Result<EnforcementResult> {
    let sts_api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);

    let sts = sts_api.get(name).await.map_err(|e| Error::NotFound {
        kind: "StatefulSet".to_string(),
        name: name.to_string(),
        namespace: namespace.to_string(),
    })?;

    // Collect pod-template labels to use as spread selector
    let selector_labels: BTreeMap<String, String> = sts
        .spec
        .as_ref()
        .and_then(|s| s.selector.match_labels.as_ref())
        .cloned()
        .unwrap_or_default();

    let rule_set = build_rule_set(topology, selector_labels, Some(namespace));

    info!(
        "Enforcing topology mode '{}' on StatefulSet {}/{} ({} zone(s))",
        rule_set.mode,
        namespace,
        name,
        topology.zones.len()
    );

    let patch_value = build_statefulset_patch(&rule_set, topology.zones.len())?;

    if dry_run {
        debug!("dry_run=true — skipping patch for {name}");
        return Ok(EnforcementResult {
            statefulset_name: name.to_string(),
            namespace: namespace.to_string(),
            mode: rule_set.mode,
            zones: topology.zones.clone(),
            patched: false,
        });
    }

    let patch_params = PatchParams::apply(FIELD_MANAGER);
    sts_api
        .patch(name, &patch_params, &Patch::Merge(&patch_value))
        .await
        .map_err(Error::KubeError)?;

    info!("Successfully patched StatefulSet {namespace}/{name} with topology rules");

    Ok(EnforcementResult {
        statefulset_name: name.to_string(),
        namespace: namespace.to_string(),
        mode: rule_set.mode,
        zones: topology.zones.clone(),
        patched: true,
    })
}

/// Enforce topology rules on all Stellar-node StatefulSets in a namespace.
///
/// Discovers all StatefulSets carrying the `app.kubernetes.io/managed-by=stellar-operator`
/// label and applies [`enforce_on_statefulset`] to each.
///
/// Returns one [`EnforcementResult`] per processed StatefulSet.
#[instrument(skip(client), fields(namespace = %namespace))]
pub async fn enforce_namespace(
    client: &Client,
    namespace: &str,
    dry_run: bool,
) -> Result<Vec<EnforcementResult>> {
    // 1. Detect cluster topology
    let topology = discover_cluster_topology(client).await?;

    // 2. Find all managed StatefulSets in the namespace
    let sts_api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
    let label_selector = "app.kubernetes.io/managed-by=stellar-operator";
    let list = sts_api
        .list(&kube::api::ListParams::default().labels(label_selector))
        .await
        .map_err(Error::KubeError)?;

    if list.items.is_empty() {
        warn!("No managed StatefulSets found in namespace {namespace}");
        return Ok(vec![]);
    }

    info!(
        "Found {} managed StatefulSet(s) in namespace {namespace}",
        list.items.len()
    );

    // 3. Enforce on each StatefulSet
    let mut results = Vec::with_capacity(list.items.len());
    for sts in &list.items {
        let sts_name = sts.name_any();
        match enforce_on_statefulset(client, namespace, &sts_name, &topology, dry_run).await {
            Ok(r) => results.push(r),
            Err(e) => {
                warn!("Failed to enforce topology on {namespace}/{sts_name}: {e}");
            }
        }
    }

    Ok(results)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::topology::rules::{TopologyMode, TOPOLOGY_HOST_KEY, TOPOLOGY_ZONE_KEY};

    fn three_zone_topo() -> ClusterTopology {
        ClusterTopology {
            zones: vec!["us-east-1a".into(), "us-east-1b".into(), "us-east-1c".into()],
            node_count: 9,
        }
    }

    fn single_zone_topo() -> ClusterTopology {
        ClusterTopology {
            zones: vec!["local".into()],
            node_count: 3,
        }
    }

    fn selector_labels() -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert(
            "app.kubernetes.io/name".to_string(),
            "stellar-node".to_string(),
        );
        m
    }

    // ── build_statefulset_patch ───────────────────────────────────────────

    #[test]
    fn test_patch_hard_mode_contains_zone_key() {
        let rule_set = build_rule_set(&three_zone_topo(), selector_labels(), Some("stellar"));
        let patch = build_statefulset_patch(&rule_set, 3).unwrap();

        let constraints = &patch["spec"]["template"]["spec"]["topologySpreadConstraints"];
        let has_zone_key = constraints
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["topologyKey"].as_str() == Some(TOPOLOGY_ZONE_KEY));

        assert!(has_zone_key, "hard mode must include a zone spread constraint");
    }

    #[test]
    fn test_patch_hard_mode_contains_host_key() {
        let rule_set = build_rule_set(&three_zone_topo(), selector_labels(), Some("stellar"));
        let patch = build_statefulset_patch(&rule_set, 3).unwrap();

        let constraints = &patch["spec"]["template"]["spec"]["topologySpreadConstraints"];
        let has_host_key = constraints
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["topologyKey"].as_str() == Some(TOPOLOGY_HOST_KEY));

        assert!(has_host_key, "hard mode must include a host spread constraint");
    }

    #[test]
    fn test_patch_hard_mode_two_constraints() {
        let rule_set = build_rule_set(&three_zone_topo(), selector_labels(), Some("stellar"));
        let patch = build_statefulset_patch(&rule_set, 3).unwrap();
        let constraints = patch["spec"]["template"]["spec"]["topologySpreadConstraints"]
            .as_array()
            .unwrap();
        assert_eq!(constraints.len(), 2);
    }

    #[test]
    fn test_patch_soft_mode_one_constraint() {
        let rule_set = build_rule_set(&single_zone_topo(), selector_labels(), None);
        let patch = build_statefulset_patch(&rule_set, 1).unwrap();
        let constraints = patch["spec"]["template"]["spec"]["topologySpreadConstraints"]
            .as_array()
            .unwrap();
        assert_eq!(constraints.len(), 1);
    }

    #[test]
    fn test_patch_soft_mode_schedule_anyway() {
        let rule_set = build_rule_set(&single_zone_topo(), selector_labels(), None);
        let patch = build_statefulset_patch(&rule_set, 1).unwrap();
        let constraints = patch["spec"]["template"]["spec"]["topologySpreadConstraints"]
            .as_array()
            .unwrap();
        assert_eq!(
            constraints[0]["whenUnsatisfiable"].as_str(),
            Some("ScheduleAnyway")
        );
    }

    #[test]
    fn test_patch_annotations_include_mode() {
        let rule_set = build_rule_set(&three_zone_topo(), selector_labels(), Some("stellar"));
        let patch = build_statefulset_patch(&rule_set, 3).unwrap();
        let mode = &patch["metadata"]["annotations"][TOPOLOGY_MODE_ANNOTATION];
        assert_eq!(mode.as_str(), Some("HardMultiZone"));
    }

    #[test]
    fn test_patch_annotations_include_zone_count() {
        let rule_set = build_rule_set(&three_zone_topo(), selector_labels(), Some("stellar"));
        let patch = build_statefulset_patch(&rule_set, 3).unwrap();
        let count = &patch["metadata"]["annotations"][TOPOLOGY_ZONES_ANNOTATION];
        assert_eq!(count.as_str(), Some("3"));
    }

    #[test]
    fn test_patch_anti_affinity_present() {
        let rule_set = build_rule_set(&three_zone_topo(), selector_labels(), Some("stellar"));
        let patch = build_statefulset_patch(&rule_set, 3).unwrap();
        let anti = &patch["spec"]["template"]["spec"]["affinity"]["podAntiAffinity"];
        assert!(!anti.is_null(), "podAntiAffinity must be present");
    }

    // ── Topology mode selection ───────────────────────────────────────────

    #[test]
    fn test_three_zones_selects_hard_mode() {
        let rule_set = build_rule_set(&three_zone_topo(), selector_labels(), Some("stellar"));
        assert_eq!(rule_set.mode, TopologyMode::HardMultiZone);
    }

    #[test]
    fn test_single_zone_selects_soft_mode() {
        let rule_set = build_rule_set(&single_zone_topo(), selector_labels(), None);
        assert_eq!(rule_set.mode, TopologyMode::SoftSingleZone);
    }

    #[test]
    fn test_two_zones_selects_soft_mode() {
        let topo = ClusterTopology {
            zones: vec!["a".into(), "b".into()],
            node_count: 4,
        };
        let rule_set = build_rule_set(&topo, selector_labels(), None);
        assert_eq!(rule_set.mode, TopologyMode::SoftSingleZone);
    }

    #[test]
    fn test_no_zones_selects_soft_mode() {
        let topo = ClusterTopology {
            zones: vec![],
            node_count: 2,
        };
        let rule_set = build_rule_set(&topo, selector_labels(), None);
        assert_eq!(rule_set.mode, TopologyMode::SoftSingleZone);
    }
}
