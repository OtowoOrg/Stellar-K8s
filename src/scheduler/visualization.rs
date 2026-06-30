//! Scheduling policy visualization and debugging.

use super::constraints::SchedulingPolicy;
use super::optimizer::{NodeResources, ScoredNode};

pub struct SchedulingVisualizer;

impl SchedulingVisualizer {
    /// Render an ASCII table of node utilization.
    pub fn render_node_utilization(nodes: &[NodeResources]) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{:<30} {:>10} {:>10} {:>10} {:>10} {:>8}\n",
            "NODE", "CPU_FREE", "MEM_FREE", "CPU_UTIL%", "COST/HR", "ZONE"
        ));
        out.push_str(&"-".repeat(82));
        out.push('\n');

        for node in nodes {
            let cpu_util = node.utilization_pct() * 100.0;
            out.push_str(&format!(
                "{:<30} {:>9}m {:>8}Mi {:>9.1}% {:>9.3}$ {:>8}\n",
                truncate(&node.name, 30),
                node.free_cpu(),
                node.free_memory_mb(),
                cpu_util,
                node.hourly_cost_usd,
                truncate(&node.zone, 8),
            ));
        }
        out
    }

    /// Render a debug view of a scheduling decision.
    pub fn render_policy_debug(policy: &SchedulingPolicy, scored: &[ScoredNode]) -> String {
        let mut out = format!("=== Scheduling Policy: {} ===\n", policy.name);
        out.push_str(&format!("Constraints: {}\n", policy.constraints.len()));
        out.push_str(&format!(
            "Weights: resource_fit={:.2} cost={:.2} locality={:.2} balance={:.2}\n\n",
            policy.priority_weights.resource_fit,
            policy.priority_weights.cost,
            policy.priority_weights.locality,
            policy.priority_weights.balance,
        ));

        out.push_str(&format!(
            "{:<30} {:>8} {:>12} {:>8} {:>8} {:>8}\n",
            "NODE", "SCORE", "RESOURCE_FIT", "COST", "BALANCE", "LOCALITY"
        ));
        out.push_str(&"-".repeat(78));
        out.push('\n');

        for node in scored.iter().take(10) {
            let rf = node.breakdown.get("resource_fit").copied().unwrap_or(0.0);
            let cost = node.breakdown.get("cost").copied().unwrap_or(0.0);
            let bal = node.breakdown.get("balance").copied().unwrap_or(0.0);
            let loc = node.breakdown.get("locality").copied().unwrap_or(0.0);
            out.push_str(&format!(
                "{:<30} {:>8.4} {:>12.4} {:>8.4} {:>8.4} {:>8.4}\n",
                truncate(&node.node_name, 30),
                node.total_score,
                rf,
                cost,
                bal,
                loc,
            ));
        }

        if scored.is_empty() {
            out.push_str("  (no feasible nodes)\n");
        }
        out
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}
