//! Affinity and anti-affinity rule processing.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::optimizer::NodeResources;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffinityRule {
    pub key: String,
    pub operator: AffinityOperator,
    pub values: Vec<String>,
    pub required: bool, // true = hard, false = preferred
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AffinityOperator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodPlacement {
    pub pod_name: String,
    pub node_name: String,
    pub labels: HashMap<String, String>,
}

pub struct AffinityProcessor;

impl AffinityProcessor {
    /// Filter nodes by node affinity rules.
    pub fn filter_by_affinity<'a>(
        nodes: &'a [NodeResources],
        rules: &[AffinityRule],
    ) -> Vec<&'a NodeResources> {
        nodes
            .iter()
            .filter(|node| {
                rules.iter().all(|rule| {
                    if !rule.required {
                        return true; // soft rules don't filter
                    }
                    Self::evaluate_rule(node, rule)
                })
            })
            .collect()
    }

    /// Filter nodes by pod anti-affinity: exclude nodes already running pods
    /// matching the given label selector.
    pub fn filter_by_anti_affinity<'a>(
        nodes: &'a [NodeResources],
        label_selector: &str,
        existing_placements: &[PodPlacement],
    ) -> Vec<&'a NodeResources> {
        // Parse "key=value" selector
        let (sel_key, sel_val) = label_selector
            .split_once('=')
            .unwrap_or((label_selector, ""));

        // Find nodes that already have a matching pod
        let occupied_nodes: std::collections::HashSet<&str> = existing_placements
            .iter()
            .filter(|p| {
                p.labels
                    .get(sel_key)
                    .map(|v| sel_val.is_empty() || v == sel_val)
                    .unwrap_or(false)
            })
            .map(|p| p.node_name.as_str())
            .collect();

        nodes
            .iter()
            .filter(|n| !occupied_nodes.contains(n.name.as_str()))
            .collect()
    }

    fn evaluate_rule(node: &NodeResources, rule: &AffinityRule) -> bool {
        match rule.operator {
            AffinityOperator::In => node
                .labels
                .get(&rule.key)
                .map(|v| rule.values.contains(v))
                .unwrap_or(false),
            AffinityOperator::NotIn => node
                .labels
                .get(&rule.key)
                .map(|v| !rule.values.contains(v))
                .unwrap_or(true),
            AffinityOperator::Exists => node.labels.contains_key(&rule.key),
            AffinityOperator::DoesNotExist => !node.labels.contains_key(&rule.key),
        }
    }

    /// Compute topology spread: returns skew per topology key value.
    pub fn compute_topology_skew(
        placements: &[PodPlacement],
        nodes: &[NodeResources],
        topology_key: &str,
    ) -> HashMap<String, i32> {
        let mut zone_counts: HashMap<String, i32> = HashMap::new();
        for placement in placements {
            if let Some(node) = nodes.iter().find(|n| n.name == placement.node_name) {
                let zone = node
                    .labels
                    .get(topology_key)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                *zone_counts.entry(zone).or_default() += 1;
            }
        }
        zone_counts
    }
}
