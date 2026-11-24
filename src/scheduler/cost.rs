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
//! Cost-aware scheduling decisions.

use serde::{Deserialize, Serialize};

use crate::crd::WorkloadTier;

use super::capacity::{
    capacity_class_from_labels, filter_nodes_for_tier, place_workloads, score_capacity_fit,
    PlacementReport, WorkloadRequest,
};
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
    pub fn find_cheapest_viable(
        nodes: &[NodeResources],
        req_cpu_milli: u64,
        req_memory_mb: u64,
        max_hourly_cost: Option<f64>,
    ) -> Option<&NodeResources> {
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

    /// Cost-aware placement: critical stays off spot; best-effort prefers it.
    pub fn place(workloads: &[WorkloadRequest], nodes: &[NodeResources]) -> PlacementReport {
        place_workloads(workloads, nodes, &[])
    }

    /// Cheapest viable node that is legal for `tier` (spot excluded for critical).
    pub fn find_cheapest_for_tier<'a>(
        nodes: &'a [NodeResources],
        tier: WorkloadTier,
        req_cpu_milli: u64,
        req_memory_mb: u64,
    ) -> Option<&'a NodeResources> {
        let mut viable: Vec<&NodeResources> = filter_nodes_for_tier(nodes, tier)
            .into_iter()
            .filter(|n| n.free_cpu() >= req_cpu_milli && n.free_memory_mb() >= req_memory_mb)
            .collect();
        viable.sort_by(|a, b| {
            let sa = score_capacity_fit(
                tier,
                capacity_class_from_labels(&a.labels),
                a.hourly_cost_usd,
            );
            let sb = score_capacity_fit(
                tier,
                capacity_class_from_labels(&b.labels),
                b.hourly_cost_usd,
            );
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        viable.into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::CapacityClass;
    use crate::scheduler::capacity::defined_test_cluster;

    #[test]
    fn cheapest_for_critical_never_returns_spot() {
        let nodes = defined_test_cluster();
        let chosen =
            CostAwareScheduler::find_cheapest_for_tier(&nodes, WorkloadTier::Critical, 100, 128)
                .expect("on-demand exists");
        assert_eq!(
            chosen
                .labels
                .get("stellar.org/capacity-class")
                .map(String::as_str),
            Some(CapacityClass::OnDemand.as_label())
        );
    }

    #[test]
    fn cheapest_for_best_effort_prefers_spot() {
        let nodes = defined_test_cluster();
        let chosen =
            CostAwareScheduler::find_cheapest_for_tier(&nodes, WorkloadTier::BestEffort, 100, 128)
                .expect("spot exists");
        assert_eq!(
            chosen
                .labels
                .get("stellar.org/capacity-class")
                .map(String::as_str),
            Some(CapacityClass::Spot.as_label())
        );
    }
}
