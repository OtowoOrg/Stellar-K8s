use super::graph::{TopologyGraph, Node};
use std::collections::HashSet;

#[derive(Debug, PartialEq)]
pub enum TopologyStatus {
    Healthy,
    MissingDependencies(Vec<String>),
    SplitBrainRisk,
}

pub fn analyze_quorum_intersection(graph: &TopologyGraph) -> TopologyStatus {
    let mut missing_deps = HashSet::new();
    
    // Pass 1: find missing dependencies
    for node in graph.nodes.values() {
        for validator in &node.validators {
            if !graph.nodes.contains_key(validator) {
                missing_deps.insert(validator.clone());
            }
        }
    }
    
    if !missing_deps.is_empty() {
        let mut missing: Vec<String> = missing_deps.into_iter().collect();
        missing.sort(); // Predictable order for tests
        return TopologyStatus::MissingDependencies(missing);
    }

    // Pass 2: find split brain risk
    // A heuristic for split brain: threshold <= nodes/2 means that 
    // two disjoint sub-quorums can both reach agreement independently.
    for node in graph.nodes.values() {
        if !node.validators.is_empty() && node.quorum_threshold <= node.validators.len() / 2 {
            return TopologyStatus::SplitBrainRisk;
        }
    }

    TopologyStatus::Healthy
}

pub fn emit_k8s_status_condition(node_id: &str, status: &TopologyStatus) {
    // Stub for Kubernetes custom status emission
    match status {
        TopologyStatus::Healthy => {
            println!("Condition: Node {} QuorumIntersection=Healthy", node_id);
        }
        TopologyStatus::MissingDependencies(m) => {
            println!("Condition: Node {} QuorumIntersection=Degraded (Missing: {:?})", node_id, m);
        }
        TopologyStatus::SplitBrainRisk => {
            println!("Condition: Node {} QuorumIntersection=Critical (Split-Brain Risk)", node_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_healthy_topology() {
        let mut graph = TopologyGraph::new();
        graph.add_node(Node { id: "node1".into(), quorum_threshold: 2, validators: vec!["node2".into(), "node3".into()] });
        graph.add_node(Node { id: "node2".into(), quorum_threshold: 2, validators: vec!["node1".into(), "node3".into()] });
        graph.add_node(Node { id: "node3".into(), quorum_threshold: 2, validators: vec!["node1".into(), "node2".into()] });

        assert_eq!(analyze_quorum_intersection(&graph), TopologyStatus::Healthy);
    }

    #[test]
    fn test_missing_dependencies() {
        let mut graph = TopologyGraph::new();
        graph.add_node(Node { id: "node1".into(), quorum_threshold: 2, validators: vec!["node2".into(), "node_unknown".into()] });
        graph.add_node(Node { id: "node2".into(), quorum_threshold: 2, validators: vec!["node1".into(), "node_unknown".into()] });

        let status = analyze_quorum_intersection(&graph);
        assert_eq!(status, TopologyStatus::MissingDependencies(vec!["node_unknown".into()]));
    }

    #[test]
    fn test_split_brain_risk() {
        let mut graph = TopologyGraph::new();
        // A threshold of 1 out of 2 validators is a split-brain risk (both can agree independently on different values)
        graph.add_node(Node { id: "node1".into(), quorum_threshold: 1, validators: vec!["node2".into(), "node3".into()] });
        graph.add_node(Node { id: "node2".into(), quorum_threshold: 2, validators: vec!["node1".into(), "node3".into()] });
        graph.add_node(Node { id: "node3".into(), quorum_threshold: 2, validators: vec!["node1".into(), "node2".into()] });

        assert_eq!(analyze_quorum_intersection(&graph), TopologyStatus::SplitBrainRisk);
    }
}
