//! Scheduling simulation and what-if analysis.

use serde::{Deserialize, Serialize};

use super::constraints::SchedulingPolicy;
use super::optimizer::{MultiObjectiveOptimizer, NodeResources};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadSpec {
    pub name: String,
    pub replicas: u32,
    pub cpu_request_milli: u64,
    pub memory_request_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementDecision {
    pub pod_name: String,
    pub node_name: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub placements: Vec<PlacementDecision>,
    pub unschedulable: Vec<String>,
    pub total_cost_hourly: f64,
    pub cluster_utilization_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactReport {
    pub change_description: String,
    pub before_cost: f64,
    pub after_cost: f64,
    pub cost_delta: f64,
    pub before_utilization: f64,
    pub after_utilization: f64,
    pub affected_pods: usize,
}

pub struct SchedulingSimulator;

impl SchedulingSimulator {
    /// Simulate placing a workload across available nodes.
    pub fn simulate_placement(
        workload: &WorkloadSpec,
        nodes: &[NodeResources],
        policy: &SchedulingPolicy,
    ) -> SimulationResult {
        let mut available_nodes: Vec<NodeResources> = nodes.to_vec();
        let mut placements = Vec::new();
        let mut unschedulable = Vec::new();

        for replica in 0..workload.replicas {
            let pod_name = format!("{}-{}", workload.name, replica);
            let scored = MultiObjectiveOptimizer::optimize(
                &available_nodes,
                policy,
                workload.cpu_request_milli,
                workload.memory_request_mb,
            );

            if let Some(best) = scored.first() {
                placements.push(PlacementDecision {
                    pod_name: pod_name.clone(),
                    node_name: best.node_name.clone(),
                    score: best.total_score,
                });
                // Update node utilization for next iteration
                if let Some(node) = available_nodes
                    .iter_mut()
                    .find(|n| n.name == best.node_name)
                {
                    node.used_cpu_milli += workload.cpu_request_milli;
                    node.used_memory_mb += workload.memory_request_mb;
                }
            } else {
                unschedulable.push(pod_name);
            }
        }

        let total_cost: f64 = available_nodes.iter().map(|n| n.hourly_cost_usd).sum();
        let total_cpu: u64 = available_nodes
            .iter()
            .map(|n| n.allocatable_cpu_milli)
            .sum();
        let used_cpu: u64 = available_nodes.iter().map(|n| n.used_cpu_milli).sum();
        let utilization = if total_cpu > 0 {
            used_cpu as f64 / total_cpu as f64 * 100.0
        } else {
            0.0
        };

        SimulationResult {
            placements,
            unschedulable,
            total_cost_hourly: total_cost,
            cluster_utilization_pct: utilization,
        }
    }

    /// What-if: compare current vs. proposed node set.
    pub fn what_if_analysis(
        workload: &WorkloadSpec,
        current_nodes: &[NodeResources],
        proposed_nodes: &[NodeResources],
        policy: &SchedulingPolicy,
        change_description: &str,
    ) -> ImpactReport {
        let before = Self::simulate_placement(workload, current_nodes, policy);
        let after = Self::simulate_placement(workload, proposed_nodes, policy);

        ImpactReport {
            change_description: change_description.to_string(),
            before_cost: before.total_cost_hourly,
            after_cost: after.total_cost_hourly,
            cost_delta: after.total_cost_hourly - before.total_cost_hourly,
            before_utilization: before.cluster_utilization_pct,
            after_utilization: after.cluster_utilization_pct,
            affected_pods: workload.replicas as usize,
        }
    }
}
