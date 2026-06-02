//! Advanced Network Topology Management controller (epic #869).
//!
//! Pure, side-effect-free analysis of a
//! [`StellarTopology`](crate::crd::StellarTopology): quorum health, network
//! partition detection, peer-optimization recommendations, and a what-if
//! **network simulation** for proposed validator failures. Keeping the logic
//! pure lets it be unit tested without a live validator network; a reconciler
//! feeds it observed liveness and writes the results to status.
//!
//! Scope of this slice: topology graph analysis, partition detection, quorum
//! health, recommendations, and failure simulation. SCP→Kafka streaming, the
//! real-time visualization dashboard, and historical querying are tracked as
//! follow-up work in the epic.

use crate::crd::stellar_topology::{StellarTopologySpec, TopologyPhase};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Computed view of a topology under a given liveness assumption.
#[derive(Clone, Debug, PartialEq)]
pub struct TopologyAnalysis {
    pub total_validators: usize,
    pub online_validators: usize,
    pub quorum_health_pct: f64,
    pub partition_detected: bool,
    /// Connected groups among online validators (sorted, deterministic).
    pub components: Vec<Vec<String>>,
    pub recommendations: Vec<String>,
    pub phase: TopologyPhase,
}

/// Build an undirected adjacency map restricted to declared validators.
///
/// Peer references to names that are not themselves declared validators are
/// ignored for reachability analysis.
fn build_adjacency(spec: &StellarTopologySpec) -> BTreeMap<String, BTreeSet<String>> {
    let names: BTreeSet<&str> = spec.validators.iter().map(|v| v.name.as_str()).collect();
    let mut adj: BTreeMap<String, BTreeSet<String>> = spec
        .validators
        .iter()
        .map(|v| (v.name.clone(), BTreeSet::new()))
        .collect();

    for v in &spec.validators {
        for peer in &v.peers {
            if peer != &v.name && names.contains(peer.as_str()) {
                adj.get_mut(&v.name).unwrap().insert(peer.clone());
                adj.get_mut(peer).unwrap().insert(v.name.clone());
            }
        }
    }
    adj
}

/// Connected components of the subgraph induced by `online` validators.
fn connected_components(
    adj: &BTreeMap<String, BTreeSet<String>>,
    online: &BTreeSet<String>,
) -> Vec<Vec<String>> {
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut components: Vec<Vec<String>> = Vec::new();

    for start in online.iter() {
        if visited.contains(start) {
            continue;
        }
        // BFS over online neighbours only.
        let mut component: Vec<String> = Vec::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        queue.push_back(start.clone());
        visited.insert(start.clone());

        while let Some(node) = queue.pop_front() {
            component.push(node.clone());
            if let Some(neighbours) = adj.get(&node) {
                for n in neighbours {
                    if online.contains(n) && !visited.contains(n) {
                        visited.insert(n.clone());
                        queue.push_back(n.clone());
                    }
                }
            }
        }
        component.sort();
        components.push(component);
    }
    // Largest groups first for stable, readable output.
    components.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    components
}

/// Generate peer-optimization and remediation recommendations.
fn recommendations(
    spec: &StellarTopologySpec,
    components: &[Vec<String>],
    quorum_health_pct: f64,
) -> Vec<String> {
    let mut recs = Vec::new();

    // Poorly connected validators are a resilience risk.
    for v in &spec.validators {
        let declared_peers = v
            .peers
            .iter()
            .filter(|p| *p != &v.name)
            .collect::<BTreeSet<_>>()
            .len();
        if declared_peers < 2 {
            recs.push(format!(
                "Validator '{}' has only {} peer(s); add redundant peers (>= 2) for resilience.",
                v.name, declared_peers
            ));
        }
    }

    if components.len() > 1 {
        recs.push(format!(
            "Network is partitioned into {} groups; add cross-group peer connections to restore a single connected component.",
            components.len()
        ));
    }

    if quorum_health_pct < spec.min_online_pct {
        recs.push(format!(
            "Only {:.1}% of validators are online (minimum {:.1}%); investigate and restore offline validators.",
            quorum_health_pct, spec.min_online_pct
        ));
    }

    recs
}

fn derive_phase(
    partition_detected: bool,
    quorum_health_pct: f64,
    min_online_pct: f64,
    total: usize,
) -> TopologyPhase {
    if total == 0 {
        return TopologyPhase::Pending;
    }
    if partition_detected {
        TopologyPhase::Partitioned
    } else if quorum_health_pct < min_online_pct {
        TopologyPhase::Degraded
    } else {
        TopologyPhase::Healthy
    }
}

/// Analyze the topology given the set of validators currently online.
///
/// Names in `online` that are not declared validators are ignored.
pub fn analyze(spec: &StellarTopologySpec, online: &BTreeSet<String>) -> TopologyAnalysis {
    let adj = build_adjacency(spec);
    let declared: BTreeSet<String> = adj.keys().cloned().collect();
    let online_declared: BTreeSet<String> = online.intersection(&declared).cloned().collect();

    let total = declared.len();
    let online_count = online_declared.len();
    let quorum_health_pct = if total == 0 {
        0.0
    } else {
        online_count as f64 / total as f64 * 100.0
    };

    let components = connected_components(&adj, &online_declared);
    let partition_detected = components.len() > 1;
    let recommendations = recommendations(spec, &components, quorum_health_pct);
    let phase = derive_phase(partition_detected, quorum_health_pct, spec.min_online_pct, total);

    TopologyAnalysis {
        total_validators: total,
        online_validators: online_count,
        quorum_health_pct,
        partition_detected,
        components,
        recommendations,
        phase,
    }
}

/// Simulate a failure scenario: analyze the topology as if `failed` validators
/// were offline (all other declared validators are assumed online).
///
/// This powers the epic's network-simulation capability — operators can
/// predict whether removing nodes would partition the network before it
/// happens in production.
pub fn simulate_failures(spec: &StellarTopologySpec, failed: &[String]) -> TopologyAnalysis {
    let failed_set: BTreeSet<&str> = failed.iter().map(|s| s.as_str()).collect();
    let online: BTreeSet<String> = spec
        .validators
        .iter()
        .map(|v| v.name.clone())
        .filter(|n| !failed_set.contains(n.as_str()))
        .collect();
    analyze(spec, &online)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::stellar_topology::TopologyValidator;

    fn val(name: &str, peers: &[&str]) -> TopologyValidator {
        TopologyValidator {
            name: name.to_string(),
            peers: peers.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// A fully-meshed three-validator network.
    fn mesh3() -> StellarTopologySpec {
        StellarTopologySpec {
            validators: vec![
                val("a", &["b", "c"]),
                val("b", &["a", "c"]),
                val("c", &["a", "b"]),
            ],
            min_online_pct: 66.0,
            partition_detection_window_seconds: 30,
        }
    }

    fn all_online(spec: &StellarTopologySpec) -> BTreeSet<String> {
        spec.validators.iter().map(|v| v.name.clone()).collect()
    }

    #[test]
    fn fully_connected_network_is_healthy() {
        let spec = mesh3();
        let a = analyze(&spec, &all_online(&spec));
        assert_eq!(a.phase, TopologyPhase::Healthy);
        assert!(!a.partition_detected);
        assert_eq!(a.components.len(), 1);
        assert_eq!(a.online_validators, 3);
        assert!((a.quorum_health_pct - 100.0).abs() < 1e-9);
        assert!(a.recommendations.is_empty());
    }

    #[test]
    fn split_network_is_detected_as_partition() {
        // Two pairs with no link between them: {a,b} and {c,d}.
        let spec = StellarTopologySpec {
            validators: vec![
                val("a", &["b"]),
                val("b", &["a"]),
                val("c", &["d"]),
                val("d", &["c"]),
            ],
            min_online_pct: 66.0,
            partition_detection_window_seconds: 30,
        };
        let a = analyze(&spec, &all_online(&spec));
        assert!(a.partition_detected);
        assert_eq!(a.phase, TopologyPhase::Partitioned);
        assert_eq!(a.components.len(), 2);
        assert!(a
            .recommendations
            .iter()
            .any(|r| r.contains("partitioned")));
    }

    #[test]
    fn too_few_online_is_degraded() {
        let spec = mesh3();
        // Only one of three online: connected (single node) but below 66%.
        let online: BTreeSet<String> = ["a".to_string()].into_iter().collect();
        let a = analyze(&spec, &online);
        assert!(!a.partition_detected);
        assert_eq!(a.phase, TopologyPhase::Degraded);
        assert!((a.quorum_health_pct - (100.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn unknown_peer_references_are_ignored() {
        let spec = StellarTopologySpec {
            validators: vec![val("a", &["b", "ghost"]), val("b", &["a"])],
            min_online_pct: 50.0,
            partition_detection_window_seconds: 30,
        };
        let a = analyze(&spec, &all_online(&spec));
        // "ghost" is not a declared validator, so a-b are still a single group.
        assert_eq!(a.components.len(), 1);
        assert_eq!(a.total_validators, 2);
    }

    #[test]
    fn simulation_predicts_partition_from_node_removal() {
        // Line topology a-b-c: removing the bridge 'b' splits a from c.
        let spec = StellarTopologySpec {
            validators: vec![val("a", &["b"]), val("b", &["a", "c"]), val("c", &["b"])],
            min_online_pct: 66.0,
            partition_detection_window_seconds: 30,
        };
        let intact = simulate_failures(&spec, &[]);
        assert!(!intact.partition_detected);

        let without_bridge = simulate_failures(&spec, &["b".to_string()]);
        assert!(without_bridge.partition_detected);
        assert_eq!(without_bridge.online_validators, 2);
        assert_eq!(without_bridge.components.len(), 2);
    }

    #[test]
    fn poorly_connected_validator_is_flagged() {
        let spec = StellarTopologySpec {
            validators: vec![val("a", &["b"]), val("b", &["a"])],
            min_online_pct: 50.0,
            partition_detection_window_seconds: 30,
        };
        let a = analyze(&spec, &all_online(&spec));
        // Each validator has only one peer (< 2).
        assert!(a
            .recommendations
            .iter()
            .any(|r| r.contains("only 1 peer")));
    }
}
