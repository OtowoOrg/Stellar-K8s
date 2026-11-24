//! Dynamic zone-awareness for anti-affinity/topology-spread decisions.
//!
//! Downgrades a requested "Hard" (strict) anti-affinity strength to "Soft"
//! (preferred) when the cluster does not have enough distinct zones to
//! satisfy strict topology spread across sibling StellarNode pods — e.g.
//! single-node Kind/Minikube dev clusters.

use std::collections::HashSet;

use k8s_openapi::api::core::v1::Node;
use kube::api::{Api, ListParams};
use kube::Client;

use crate::crd::types::PodAntiAffinityStrength;
use crate::crd::{StellarNode, StellarNodeSpec};

/// Label used to determine zone membership for topology spread decisions.
pub const ZONE_LABEL: &str = "topology.kubernetes.io/zone";

/// Summary of the cluster's zone topology, used to decide whether strict
/// ("Hard") anti-affinity / topology spread constraints are achievable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneTopology {
    pub zone_count: usize,
    pub node_count: usize,
}

impl ZoneTopology {
    /// Whether the cluster has enough distinct zones to strictly schedule
    /// `sibling_count` pods, one per zone (or more, if zones outnumber pods).
    pub fn can_satisfy_strict(&self, sibling_count: usize) -> bool {
        self.zone_count >= sibling_count.max(1)
    }
}

/// Lists all cluster nodes and extracts the distinct set of zone labels.
///
/// Nodes without a `topology.kubernetes.io/zone` label are counted toward
/// `node_count` but not toward `zone_count` — this naturally covers
/// single-node dev clusters (Kind/Minikube) where the label is often absent
/// entirely, since an empty zone set will never satisfy `can_satisfy_strict`.
pub async fn fetch_zone_topology(client: &Client) -> kube::Result<ZoneTopology> {
    let nodes: Api<Node> = Api::all(client.clone());
    let node_list = nodes.list(&ListParams::default()).await?;

    let mut zones: HashSet<String> = HashSet::new();
    let mut node_count = 0usize;

    for node in &node_list.items {
        node_count += 1;
        if let Some(labels) = node.metadata.labels.as_ref() {
            if let Some(zone) = labels.get(ZONE_LABEL) {
                if !zone.is_empty() {
                    zones.insert(zone.clone());
                }
            }
        }
    }

    Ok(ZoneTopology {
        zone_count: zones.len(),
        node_count,
    })
}

/// Counts sibling `StellarNode` objects that would be spread by
/// `network_spread_label_selector` — i.e. same network + same component
/// (node type), across the cluster. Used as the "replica" count for
/// zone-satisfiability checks, since each `StellarNode` maps to a single
/// pod rather than a StatefulSet `replicas` field.
pub async fn count_sibling_nodes(client: &Client, spec: &StellarNodeSpec) -> kube::Result<usize> {
    let api: Api<StellarNode> = Api::all(client.clone());
    let list = api.list(&ListParams::default()).await?;

    let network_value = spec
        .network
        .scheduling_label_value(&spec.custom_network_passphrase);
    let component = spec.node_type.to_string().to_lowercase();

    let count = list
        .items
        .iter()
        .filter(|n| {
            n.spec
                .network
                .scheduling_label_value(&n.spec.custom_network_passphrase)
                == network_value
                && n.spec.node_type.to_string().to_lowercase() == component
        })
        .count();

    Ok(count.max(1))
}

/// Resolves the *effective* anti-affinity strength to apply, given what the
/// user requested and the cluster's actual zone topology.
///
/// - `Disabled` always passes through unchanged (explicit opt-out).
/// - `Soft` always passes through unchanged (already non-strict).
/// - `Hard` is downgraded to `Soft` when the cluster cannot satisfy strict
///   placement (fewer distinct zones than sibling pods) — this prevents
///   pods from being stuck `Pending` forever on dev clusters.
pub fn resolve_anti_affinity_strength(
    requested: PodAntiAffinityStrength,
    topology: &ZoneTopology,
    sibling_count: usize,
) -> PodAntiAffinityStrength {
    match requested {
        PodAntiAffinityStrength::Hard if !topology.can_satisfy_strict(sibling_count) => {
            PodAntiAffinityStrength::Soft
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_satisfied_when_zones_cover_siblings() {
        let topo = ZoneTopology {
            zone_count: 3,
            node_count: 6,
        };
        assert!(topo.can_satisfy_strict(3));
        assert_eq!(
            resolve_anti_affinity_strength(PodAntiAffinityStrength::Hard, &topo, 3),
            PodAntiAffinityStrength::Hard
        );
    }

    #[test]
    fn strict_downgrades_when_zones_insufficient() {
        let topo = ZoneTopology {
            zone_count: 1,
            node_count: 1,
        };
        assert!(!topo.can_satisfy_strict(3));
        assert_eq!(
            resolve_anti_affinity_strength(PodAntiAffinityStrength::Hard, &topo, 3),
            PodAntiAffinityStrength::Soft
        );
    }

    #[test]
    fn strict_downgrades_on_zero_zone_labels() {
        let topo = ZoneTopology {
            zone_count: 0,
            node_count: 1,
        };
        assert_eq!(
            resolve_anti_affinity_strength(PodAntiAffinityStrength::Hard, &topo, 1),
            PodAntiAffinityStrength::Soft
        );
    }

    #[test]
    fn soft_and_disabled_pass_through_unchanged() {
        let topo = ZoneTopology {
            zone_count: 0,
            node_count: 1,
        };
        assert_eq!(
            resolve_anti_affinity_strength(PodAntiAffinityStrength::Soft, &topo, 5),
            PodAntiAffinityStrength::Soft
        );
        assert_eq!(
            resolve_anti_affinity_strength(PodAntiAffinityStrength::Disabled, &topo, 5),
            PodAntiAffinityStrength::Disabled
        );
    }
}
