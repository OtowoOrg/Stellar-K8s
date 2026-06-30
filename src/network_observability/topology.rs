//! Network topology visualization and service dependency mapping.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::flow::NetworkFlow;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDependency {
    pub source: String,
    pub destination: String,
    pub call_count: u64,
    pub total_bytes: u64,
    pub avg_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyNode {
    pub name: String,
    pub kind: String, // "pod", "service", "external"
    pub namespace: String,
    pub ip: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyEdge {
    pub source: String,
    pub destination: String,
    pub flow_count: u64,
    pub bytes: u64,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TopologyGraph {
    pub nodes: Vec<TopologyNode>,
    pub edges: Vec<TopologyEdge>,
}

impl TopologyGraph {
    pub fn build_from_flows(flows: &[NetworkFlow]) -> Self {
        let mut nodes: HashMap<String, TopologyNode> = HashMap::new();
        let mut edge_map: HashMap<(String, String), TopologyEdge> = HashMap::new();

        for flow in flows {
            // Source node
            nodes
                .entry(flow.pod_name.clone())
                .or_insert_with(|| TopologyNode {
                    name: flow.pod_name.clone(),
                    kind: "pod".to_string(),
                    namespace: flow.namespace.clone(),
                    ip: flow.src_ip.clone(),
                });

            // Destination node (use service name if available, else IP)
            let dst_name = flow
                .service_name
                .clone()
                .unwrap_or_else(|| flow.dst_ip.clone());
            nodes
                .entry(dst_name.clone())
                .or_insert_with(|| TopologyNode {
                    name: dst_name.clone(),
                    kind: if flow.service_name.is_some() {
                        "service"
                    } else {
                        "external"
                    }
                    .to_string(),
                    namespace: flow.namespace.clone(),
                    ip: flow.dst_ip.clone(),
                });

            // Edge
            let key = (flow.pod_name.clone(), dst_name.clone());
            let edge = edge_map.entry(key).or_insert_with(|| TopologyEdge {
                source: flow.pod_name.clone(),
                destination: dst_name.clone(),
                flow_count: 0,
                bytes: 0,
                protocol: format!("{:?}", flow.protocol),
            });
            edge.flow_count += 1;
            edge.bytes += flow.bytes;
        }

        TopologyGraph {
            nodes: nodes.into_values().collect(),
            edges: edge_map.into_values().collect(),
        }
    }

    pub fn get_dependencies(&self, service_name: &str) -> Vec<ServiceDependency> {
        self.edges
            .iter()
            .filter(|e| e.source == service_name || e.destination == service_name)
            .map(|e| ServiceDependency {
                source: e.source.clone(),
                destination: e.destination.clone(),
                call_count: e.flow_count,
                total_bytes: e.bytes,
                avg_latency_ms: 0.0, // populated from flow duration data
            })
            .collect()
    }

    pub fn find_isolated_nodes(&self) -> Vec<String> {
        let connected: HashSet<String> = self
            .edges
            .iter()
            .flat_map(|e| [e.source.clone(), e.destination.clone()])
            .collect();
        self.nodes
            .iter()
            .filter(|n| !connected.contains(&n.name))
            .map(|n| n.name.clone())
            .collect()
    }
}
