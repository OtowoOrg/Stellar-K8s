//! Dependency graph generation and export (JSON/DOT format)

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

use super::registry::ServiceRegistration;

/// Edge in the dependency graph
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub from: String,
    pub to: String,
    pub edge_type: String,
}

/// Full topology graph
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub nodes: HashMap<String, GraphNode>,
    pub edges: Vec<DependencyEdge>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub name: String,
    pub service_type: String,
    pub namespace: String,
    pub health_score: f64,
}

/// Export formats for the topology graph
pub enum TopologyExport {
    Json(serde_json::Value),
    Dot(String),
}

impl DependencyGraph {
    pub fn from_registrations(services: &[ServiceRegistration]) -> Self {
        let mut graph = Self::default();

        for svc in services {
            graph.nodes.insert(
                svc.id.clone(),
                GraphNode {
                    id: svc.id.clone(),
                    name: svc.name.clone(),
                    service_type: format!("{:?}", svc.service_type),
                    namespace: svc.namespace.clone(),
                    health_score: svc.health_score.score,
                },
            );

            for dep in &svc.dependencies {
                graph.edges.push(DependencyEdge {
                    from: svc.id.clone(),
                    to: dep.clone(),
                    edge_type: "depends_on".into(),
                });
            }
        }

        graph
    }

    /// Export as JSON
    pub fn to_json(&self) -> TopologyExport {
        TopologyExport::Json(serde_json::json!({
            "nodes": self.nodes.values().collect::<Vec<_>>(),
            "edges": self.edges,
        }))
    }

    /// Export as Graphviz DOT format
    pub fn to_dot(&self) -> TopologyExport {
        let mut dot = String::from("digraph StellarTopology {\n  rankdir=LR;\n  node [shape=box];\n");

        for node in self.nodes.values() {
            let color = if node.health_score >= 0.8 {
                "green"
            } else if node.health_score >= 0.5 {
                "yellow"
            } else {
                "red"
            };
            dot.push_str(&format!(
                "  \"{}\" [label=\"{}\\n{}\" color=\"{}\" style=filled fillcolor=\"{}\"];\n",
                node.id, node.name, node.service_type, color, color
            ));
        }

        for edge in &self.edges {
            dot.push_str(&format!(
                "  \"{}\" -> \"{}\" [label=\"{}\"];\n",
                edge.from, edge.to, edge.edge_type
            ));
        }

        dot.push('}');
        TopologyExport::Dot(dot)
    }

    /// Topological sort (BFS Kahn's algorithm) — returns None on cycle
    pub fn topological_order(&self) -> Option<Vec<String>> {
        let mut in_degree: HashMap<String, usize> =
            self.nodes.keys().map(|k| (k.clone(), 0)).collect();

        for edge in &self.edges {
            *in_degree.entry(edge.to.clone()).or_insert(0) += 1;
        }

        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(k, _)| k.clone())
            .collect();

        let mut order = Vec::new();

        while let Some(node) = queue.pop_front() {
            order.push(node.clone());
            for edge in self.edges.iter().filter(|e| e.from == node) {
                let dep = in_degree.get_mut(&edge.to)?;
                *dep -= 1;
                if *dep == 0 {
                    queue.push_back(edge.to.clone());
                }
            }
        }

        if order.len() == self.nodes.len() { Some(order) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_discovery::registry::{ServiceRegistration, ServiceType};

    #[test]
    fn test_graph_from_registrations() {
        let mut svc = ServiceRegistration::new("s1", "horizon", "default", ServiceType::Horizon, "10.0.0.1", 8000);
        svc.dependencies.push("s2".into());
        let graph = DependencyGraph::from_registrations(&[svc]);
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.edges.len(), 1);
    }

    #[test]
    fn test_dot_export_contains_nodes() {
        let svc = ServiceRegistration::new("s1", "core", "ns1", ServiceType::StellarCore, "10.0.0.2", 11626);
        let graph = DependencyGraph::from_registrations(&[svc]);
        if let TopologyExport::Dot(dot) = graph.to_dot() {
            assert!(dot.contains("digraph"));
            assert!(dot.contains("s1"));
        } else {
            panic!("expected DOT output");
        }
    }
}
