//! Multi-objective optimization algorithm for scheduling decisions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::constraints::{SchedulingConstraint, SchedulingPolicy};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeResources {
    pub name: String,
    pub allocatable_cpu_milli: u64,
    pub allocatable_memory_mb: u64,
    pub used_cpu_milli: u64,
    pub used_memory_mb: u64,
    pub zone: String,
    pub region: String,
    pub hourly_cost_usd: f64,
    pub labels: HashMap<String, String>,
    pub taints: Vec<(String, String, String)>, // (key, value, effect)
}

impl NodeResources {
    pub fn free_cpu(&self) -> u64 {
        self.allocatable_cpu_milli
            .saturating_sub(self.used_cpu_milli)
    }

    pub fn free_memory_mb(&self) -> u64 {
        self.allocatable_memory_mb
            .saturating_sub(self.used_memory_mb)
    }

    pub fn utilization_pct(&self) -> f64 {
        if self.allocatable_cpu_milli == 0 {
            return 0.0;
        }
        self.used_cpu_milli as f64 / self.allocatable_cpu_milli as f64
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredNode {
    pub node_name: String,
    pub total_score: f64,
    pub breakdown: HashMap<String, f64>,
    pub feasible: bool,
}

pub struct MultiObjectiveOptimizer;

impl MultiObjectiveOptimizer {
    /// Filter nodes that satisfy hard constraints, then score remaining nodes.
    pub fn optimize(
        nodes: &[NodeResources],
        policy: &SchedulingPolicy,
        request_cpu_milli: u64,
        request_memory_mb: u64,
    ) -> Vec<ScoredNode> {
        let feasible: Vec<&NodeResources> = nodes
            .iter()
            .filter(|n| Self::is_feasible(n, policy, request_cpu_milli, request_memory_mb))
            .collect();

        let mut scored: Vec<ScoredNode> = feasible
            .iter()
            .map(|n| Self::score(n, policy, request_cpu_milli, request_memory_mb))
            .collect();

        scored.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap());
        scored
    }

    fn is_feasible(
        node: &NodeResources,
        policy: &SchedulingPolicy,
        req_cpu: u64,
        req_mem: u64,
    ) -> bool {
        // Check resource availability
        if node.free_cpu() < req_cpu || node.free_memory_mb() < req_mem {
            return false;
        }
        // Check hard constraints
        for constraint in &policy.constraints {
            match constraint {
                SchedulingConstraint::NodeAffinity { key, values } => {
                    if let Some(v) = node.labels.get(key) {
                        if !values.contains(v) {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                SchedulingConstraint::NodeTaint { key, value, effect } => {
                    let has_taint = node
                        .taints
                        .iter()
                        .any(|(k, v, e)| k == key && v == value && e == effect);
                    if has_taint {
                        return false;
                    }
                }
                SchedulingConstraint::CostBudget {
                    max_hourly_cost_usd,
                } => {
                    if node.hourly_cost_usd > *max_hourly_cost_usd {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }

    fn score(
        node: &NodeResources,
        policy: &SchedulingPolicy,
        req_cpu: u64,
        req_mem: u64,
    ) -> ScoredNode {
        let w = &policy.priority_weights;
        let mut breakdown = HashMap::new();

        // Resource fit: prefer nodes with just enough resources (avoid waste)
        let cpu_fit = if node.allocatable_cpu_milli > 0 {
            1.0 - (node.free_cpu().saturating_sub(req_cpu)) as f64
                / node.allocatable_cpu_milli as f64
        } else {
            0.0
        };
        let mem_fit = if node.allocatable_memory_mb > 0 {
            1.0 - (node.free_memory_mb().saturating_sub(req_mem)) as f64
                / node.allocatable_memory_mb as f64
        } else {
            0.0
        };
        let resource_score = (cpu_fit + mem_fit) / 2.0;
        breakdown.insert("resource_fit".to_string(), resource_score);

        // Cost: lower cost = higher score
        let max_cost = 5.0_f64; // normalize against $5/hr
        let cost_score = 1.0 - (node.hourly_cost_usd / max_cost).min(1.0);
        breakdown.insert("cost".to_string(), cost_score);

        // Balance: prefer less utilized nodes
        let balance_score = 1.0 - node.utilization_pct().min(1.0);
        breakdown.insert("balance".to_string(), balance_score);

        // Locality: placeholder (would use zone/region affinity)
        let locality_score = 0.5;
        breakdown.insert("locality".to_string(), locality_score);

        let total = resource_score * w.resource_fit
            + cost_score * w.cost
            + balance_score * w.balance
            + locality_score * w.locality;

        ScoredNode {
            node_name: node.name.clone(),
            total_score: total,
            breakdown,
            feasible: true,
        }
    }
}
