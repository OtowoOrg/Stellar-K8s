//! Graph-based quorum analysis engine

use super::types::QuorumSetInfo;
use std::collections::{HashMap, HashSet};
use tracing::debug;

/// Quorum graph for analysis
#[derive(Clone, Debug)]
pub struct QuorumGraph {
    nodes: HashMap<String, QuorumNode>,
}

/// Node in the quorum graph
#[derive(Clone, Debug)]
pub struct QuorumNode {
    pub validator_key: String,
    pub quorum_set: QuorumSetInfo,
}

/// Result of critical node analysis
#[derive(Clone, Debug)]
pub struct CriticalNodeAnalysis {
    pub critical_nodes: Vec<String>,
    pub quorum_intersection_valid: bool,
}

/// Result of overlap analysis
#[derive(Clone, Debug)]
pub struct OverlapAnalysis {
    pub min_overlap: usize,
    pub max_overlap: usize,
    pub avg_overlap: f64,
    pub overlap_pairs: Vec<(String, String, usize)>,
}

impl QuorumGraph {
    /// Construct a quorum graph from quorum set configurations
    pub fn from_quorum_sets(quorum_sets: Vec<(String, QuorumSetInfo)>) -> Self {
        let mut nodes = HashMap::new();

        for (validator_key, quorum_set) in quorum_sets {
            nodes.insert(
                validator_key.clone(),
                QuorumNode {
                    validator_key,
                    quorum_set,
                },
            );
        }

        Self { nodes }
    }

    /// Find critical nodes in the quorum
    /// 
    /// A node is critical if removing it breaks quorum intersection
    pub fn find_critical_nodes(&self) -> CriticalNodeAnalysis {
        let mut critical_nodes = Vec::new();

        // First check if current graph has valid quorum intersection
        let initial_valid = self.has_quorum_intersection();

        if !initial_valid {
            return CriticalNodeAnalysis {
                critical_nodes: vec![],
                quorum_intersection_valid: false,
            };
        }

        // Test each node for criticality
        for node_key in self.nodes.keys() {
            let graph_without_node = self.remove_node(node_key);
            
            if !graph_without_node.has_quorum_intersection() {
                debug!("Node {} is critical", node_key);
                critical_nodes.push(node_key.clone());
            }
        }

        CriticalNodeAnalysis {
            critical_nodes,
            quorum_intersection_valid: true,
        }
    }

    /// Calculate quorum overlaps between all validator pairs
    pub fn calculate_overlaps(&self) -> OverlapAnalysis {
        let mut overlap_pairs = Vec::new();
        let mut min_overlap = usize::MAX;
        let mut max_overlap = 0;
        let mut total_overlap = 0;
        let mut pair_count = 0;

        // Get all quorum slices for each validator
        let mut validator_slices: HashMap<String, Vec<HashSet<String>>> = HashMap::new();
        
        for validator_key in self.nodes.keys() {
            let slices = self.compute_quorum_slices(validator_key);
            validator_slices.insert(validator_key.clone(), slices);
        }

        // Compare all pairs of validators
        let validators: Vec<_> = self.nodes.keys().cloned().collect();
        
        for i in 0..validators.len() {
            for j in (i + 1)..validators.len() {
                let v1 = &validators[i];
                let v2 = &validators[j];

                if let (Some(slices1), Some(slices2)) = 
                    (validator_slices.get(v1), validator_slices.get(v2)) {
                    
                    // Find maximum overlap between any pair of slices
                    let mut max_pair_overlap = 0;
                    
                    for slice1 in slices1 {
                        for slice2 in slices2 {
                            let overlap = self.check_quorum_intersection_size(slice1, slice2);
                            max_pair_overlap = max_pair_overlap.max(overlap);
                        }
                    }

                    if max_pair_overlap > 0 {
                        overlap_pairs.push((v1.clone(), v2.clone(), max_pair_overlap));
                        min_overlap = min_overlap.min(max_pair_overlap);
                        max_overlap = max_overlap.max(max_pair_overlap);
                        total_overlap += max_pair_overlap;
                        pair_count += 1;
                    }
                }
            }
        }

        let avg_overlap = if pair_count > 0 {
            total_overlap as f64 / pair_count as f64
        } else {
            0.0
        };

        // Handle case where no overlaps were found
        if min_overlap == usize::MAX {
            min_overlap = 0;
        }

        OverlapAnalysis {
            min_overlap,
            max_overlap,
            avg_overlap,
            overlap_pairs,
        }
    }

    /// Compute all possible quorum slices for a validator
    /// 
    /// A quorum slice is a minimal set of validators that satisfies the threshold
    fn compute_quorum_slices(&self, validator_key: &str) -> Vec<HashSet<String>> {
        let node = match self.nodes.get(validator_key) {
            Some(n) => n,
            None => return vec![],
        };

        let mut slices = Vec::new();
        self.enumerate_slices(&node.quorum_set, &mut slices);
        slices
    }

    /// Recursively enumerate all valid quorum slices from a quorum set
    fn enumerate_slices(&self, qset: &QuorumSetInfo, slices: &mut Vec<HashSet<String>>) {
        // Collect all validators (direct + from inner sets)
        let mut all_validators = qset.validators.clone();
        
        // Add validators from inner sets
        for inner_set in &qset.inner_sets {
            all_validators.extend(inner_set.validators.clone());
        }

        if all_validators.is_empty() {
            return;
        }

        // Generate combinations that meet the threshold
        let threshold = qset.threshold as usize;
        let combinations = self.generate_combinations(&all_validators, threshold);
        
        for combo in combinations {
            slices.push(combo);
        }
    }

    /// Generate all combinations of size k from the given validators
    fn generate_combinations(&self, validators: &[String], k: usize) -> Vec<HashSet<String>> {
        let mut result = Vec::new();
        
        if k == 0 {
            result.push(HashSet::new());
            return result;
        }
        
        if k > validators.len() {
            return result;
        }

        // Simple combination generation
        self.combinations_helper(validators, k, 0, &mut vec![], &mut result);
        result
    }

    fn combinations_helper(
        &self,
        validators: &[String],
        k: usize,
        start: usize,
        current: &mut Vec<String>,
        result: &mut Vec<HashSet<String>>,
    ) {
        if current.len() == k {
            result.push(current.iter().cloned().collect());
            return;
        }

        for i in start..validators.len() {
            current.push(validators[i].clone());
            self.combinations_helper(validators, k, i + 1, current, result);
            current.pop();
        }
    }

    /// Check if the quorum graph has valid quorum intersection
    /// 
    /// Quorum intersection means any two quorum slices share at least one node
    fn has_quorum_intersection(&self) -> bool {
        if self.nodes.is_empty() {
            return false;
        }

        // Get all quorum slices for all validators
        let mut all_slices = Vec::new();
        
        for validator_key in self.nodes.keys() {
            let slices = self.compute_quorum_slices(validator_key);
            all_slices.extend(slices);
        }

        if all_slices.len() < 2 {
            return true; // Trivially true for 0 or 1 slice
        }

        // Check all pairs of slices for intersection
        for i in 0..all_slices.len() {
            for j in (i + 1)..all_slices.len() {
                if self.check_quorum_intersection_size(&all_slices[i], &all_slices[j]) == 0 {
                    return false;
                }
            }
        }

        true
    }

    /// Calculate the size of intersection between two quorum slices
    fn check_quorum_intersection_size(&self, slice_a: &HashSet<String>, slice_b: &HashSet<String>) -> usize {
        slice_a.intersection(slice_b).count()
    }

    /// Remove a node from the graph and return a new graph
    fn remove_node(&self, node_key: &str) -> Self {
        let mut new_nodes = self.nodes.clone();
        new_nodes.remove(node_key);

        Self { nodes: new_nodes }
    }

    /// Get the number of nodes in the graph
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_simple_quorum_set(validators: Vec<String>, threshold: u32) -> QuorumSetInfo {
        QuorumSetInfo {
            threshold,
            validators,
            inner_sets: vec![],
        }
    }

    #[test]
    fn test_graph_creation() {
        let qsets = vec![
            ("V1".to_string(), create_simple_quorum_set(vec!["V2".to_string(), "V3".to_string()], 2)),
            ("V2".to_string(), create_simple_quorum_set(vec!["V1".to_string(), "V3".to_string()], 2)),
        ];

        let graph = QuorumGraph::from_quorum_sets(qsets);
        assert_eq!(graph.node_count(), 2);
    }

    #[test]
    fn test_empty_graph() {
        let graph = QuorumGraph::from_quorum_sets(vec![]);
        assert_eq!(graph.node_count(), 0);
        assert!(!graph.has_quorum_intersection());
    }
}
