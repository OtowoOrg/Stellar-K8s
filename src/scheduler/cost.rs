//! Cost-aware scheduling decisions.

use serde::{Deserialize, Serialize};

use super::optimizer::NodeResources;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCost {
    pub node_name: String,
    pub instance_type: String,
    pub hourly_cost_usd: f64,
    pub region: String,
    pub spot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementCostEstimate {
    pub node_name: String,
    pub hourly_cost_usd: f64,
    pub monthly_cost_usd: f64,
    pub is_spot: bool,
    pub savings_vs_on_demand_pct: f64,
}

pub struct CostAwareScheduler;

impl CostAwareScheduler {
    pub fn estimate_placement_cost(node: &NodeResources) -> PlacementCostEstimate {
        PlacementCostEstimate {
            node_name: node.name.clone(),
            hourly_cost_usd: node.hourly_cost_usd,
            monthly_cost_usd: node.hourly_cost_usd * 730.0,
            is_spot: node
                .labels
                .get("node.kubernetes.io/lifecycle")
                .map(|v| v == "spot")
                .unwrap_or(false),
            savings_vs_on_demand_pct: 0.0,
        }
    }

    /// Find the cheapest node that satisfies minimum resource requirements.
    pub fn find_cheapest_viable<'a>(
        nodes: &'a [NodeResources],
        req_cpu_milli: u64,
        req_memory_mb: u64,
        max_hourly_cost: Option<f64>,
    ) -> Option<&'a NodeResources> {
        let mut viable: Vec<&NodeResources> = nodes
            .iter()
            .filter(|n| {
                n.free_cpu() >= req_cpu_milli
                    && n.free_memory_mb() >= req_memory_mb
                    && max_hourly_cost
                        .map(|max| n.hourly_cost_usd <= max)
                        .unwrap_or(true)
            })
            .collect();

        viable.sort_by(|a, b| a.hourly_cost_usd.partial_cmp(&b.hourly_cost_usd).unwrap());
        viable.into_iter().next()
    }

    /// Compute total cluster cost per hour.
    pub fn total_cluster_cost(nodes: &[NodeResources]) -> f64 {
        nodes.iter().map(|n| n.hourly_cost_usd).sum()
    }

    /// Identify over-provisioned nodes (utilization < 20%).
    pub fn find_underutilized(nodes: &[NodeResources], threshold_pct: f64) -> Vec<&NodeResources> {
        nodes
            .iter()
            .filter(|n| n.utilization_pct() < threshold_pct)
            .collect()
    }
}
