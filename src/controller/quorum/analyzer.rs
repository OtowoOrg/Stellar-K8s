//! Quorum analyzer orchestrator
//!
//! This module orchestrates the quorum analysis process, calculating the fragility score
//! and coordinating between the SCP client, graph analysis, and latency tracking.

use super::error::{QuorumAnalysisError, Result};
use super::graph::QuorumGraph;
use super::latency::ConsensusLatencyTracker;
use super::scp_client::ScpClient;
use super::types::QuorumSetInfo;
use crate::crd::StellarNode;
use crate::crd::types::Condition;
use chrono::{DateTime, Utc};
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Main quorum analyzer
pub struct QuorumAnalyzer {
    scp_client: ScpClient,
    latency_tracker: ConsensusLatencyTracker,
    cache: QuorumAnalysisCache,
}

/// Result of quorum analysis
#[derive(Clone, Debug)]
pub struct QuorumAnalysisResult {
    pub critical_nodes: Vec<String>,
    pub min_overlap: usize,
    pub latency_variance: f64,
    pub fragility_score: f64,
    pub timestamp: DateTime<Utc>,
}

/// Cache for quorum analysis results
#[derive(Clone, Debug)]
struct QuorumAnalysisCache {
    last_analysis: Option<QuorumAnalysisResult>,
    last_topology_hash: Option<u64>,
    cache_expiry: DateTime<Utc>,
}

impl QuorumAnalyzer {
    /// Create a new quorum analyzer
    pub fn new(timeout: Duration, window_size: usize) -> Self {
        Self {
            scp_client: ScpClient::new(timeout),
            latency_tracker: ConsensusLatencyTracker::new(window_size),
            cache: QuorumAnalysisCache {
                last_analysis: None,
                last_topology_hash: None,
                cache_expiry: Utc::now(),
            },
        }
    }

    /// Analyze quorum for a set of validator pod IPs
    pub async fn analyze_quorum(&mut self, pod_ips: Vec<String>) -> Result<QuorumAnalysisResult> {
        info!("Starting quorum analysis for {} validators", pod_ips.len());

        // Query quorum sets from all validators
        let mut quorum_sets = Vec::new();
        
        for pod_ip in &pod_ips {
            match self.scp_client.query_scp_state(pod_ip).await {
                Ok(scp_state) => {
                    quorum_sets.push((scp_state.node_id.clone(), scp_state.quorum_set));
                }
                Err(e) => {
                    debug!("Failed to query SCP state from {}: {}", pod_ip, e);
                    // Continue with other validators
                }
            }
        }

        if quorum_sets.is_empty() {
            return Err(QuorumAnalysisError::InvalidTopology(
                "No quorum sets could be retrieved".to_string(),
            ));
        }

        // Check cache
        let topology_hash = self.compute_topology_hash(&quorum_sets);
        if self.should_use_cache(topology_hash) {
            debug!("Using cached quorum analysis result");
            return Ok(self.cache.last_analysis.clone().unwrap());
        }

        // Build quorum graph
        let graph = QuorumGraph::from_quorum_sets(quorum_sets);

        // Perform analysis
        let critical_analysis = graph.find_critical_nodes();
        let overlap_analysis = graph.calculate_overlaps();

        // Get latency variance
        let latency_variance = self.latency_tracker.get_variance_across_validators();

        // Calculate fragility score
        let fragility_score = self.calculate_fragility_score(
            critical_analysis.critical_nodes.len(),
            overlap_analysis.min_overlap,
            latency_variance,
            graph.node_count(),
        );

        let result = QuorumAnalysisResult {
            critical_nodes: critical_analysis.critical_nodes,
            min_overlap: overlap_analysis.min_overlap,
            latency_variance,
            fragility_score,
            timestamp: Utc::now(),
        };

        // Update cache
        self.cache.last_analysis = Some(result.clone());
        self.cache.last_topology_hash = Some(topology_hash);
        self.cache.cache_expiry = Utc::now() + chrono::Duration::minutes(5);

        info!(
            "Quorum analysis complete: fragility_score={:.3}, critical_nodes={}, min_overlap={}",
            fragility_score,
            result.critical_nodes.len(),
            result.min_overlap
        );

        Ok(result)
    }

    /// Calculate fragility score using weighted formula
    /// 
    /// Formula:
    /// fragility_score = w1 * critical_ratio + w2 * overlap_penalty + w3 * latency_penalty
    /// 
    /// where:
    ///   critical_ratio = critical_nodes / total_validators
    ///   overlap_penalty = 1.0 - (min_overlap / expected_overlap)
    ///   latency_penalty = normalized_variance (capped at 1.0)
    ///   weights: w1 = 0.5, w2 = 0.3, w3 = 0.2
    ///   expected_overlap = ceil(total_validators * 0.67) - 1
    fn calculate_fragility_score(
        &self,
        critical_nodes: usize,
        min_overlap: usize,
        latency_variance: f64,
        total_validators: usize,
    ) -> f64 {
        if total_validators == 0 {
            return 1.0; // Maximum fragility for empty quorum
        }

        // Weight factors
        const W1: f64 = 0.5; // Critical nodes weight
        const W2: f64 = 0.3; // Overlap weight
        const W3: f64 = 0.2; // Latency weight

        // Critical ratio
        let critical_ratio = critical_nodes as f64 / total_validators as f64;

        // Overlap penalty
        let expected_overlap = ((total_validators as f64 * 0.67).ceil() as usize).saturating_sub(1);
        let overlap_penalty = if expected_overlap > 0 {
            1.0 - (min_overlap as f64 / expected_overlap as f64).min(1.0)
        } else {
            0.0
        };

        // Latency penalty (normalize and cap at 1.0)
        let latency_penalty = (latency_variance / 1000.0).min(1.0);

        // Calculate weighted score
        let score = W1 * critical_ratio + W2 * overlap_penalty + W3 * latency_penalty;

        // Clamp to [0.0, 1.0]
        score.max(0.0).min(1.0)
    }

    /// Check if cached result should be used
    fn should_use_cache(&self, current_topology_hash: u64) -> bool {
        if let Some(last_hash) = self.cache.last_topology_hash {
            if last_hash == current_topology_hash && Utc::now() < self.cache.cache_expiry {
                return self.cache.last_analysis.is_some();
            }
        }
        false
    }

    /// Compute hash of quorum topology for cache invalidation
    fn compute_topology_hash(&self, quorum_sets: &[(String, QuorumSetInfo)]) -> u64 {
        let mut hasher = DefaultHasher::new();
        
        for (node_id, qset) in quorum_sets {
            node_id.hash(&mut hasher);
            qset.threshold.hash(&mut hasher);
            for validator in &qset.validators {
                validator.hash(&mut hasher);
            }
        }

        hasher.finish()
    }

    /// Record a latency measurement
    pub fn record_latency(&mut self, validator: &str, ledger: u64, latency_ms: u64) {
        self.latency_tracker.record_latency(validator, ledger, latency_ms);
    }

    /// Update the StellarNodeStatus with quorum analysis results
    pub async fn update_node_status(
        &self,
        client: &Client,
        node: &StellarNode,
        result: &QuorumAnalysisResult,
    ) -> Result<()> {
        let namespace = node.metadata.namespace.as_ref().ok_or_else(|| {
            QuorumAnalysisError::InvalidTopology("Node has no namespace".to_string())
        })?;

        let name = node.metadata.name.as_ref().ok_or_else(|| {
            QuorumAnalysisError::InvalidTopology("Node has no name".to_string())
        })?;

        let api: Api<StellarNode> = Api::namespaced(client.clone(), namespace);

        // Build status patch
        let mut status = node.status.clone().unwrap_or_default();
        
        // Update quorum fields
        status.quorum_fragility = Some(result.fragility_score);
        status.quorum_analysis_timestamp = Some(result.timestamp.to_rfc3339());

        // Add Degraded condition if fragility > 0.7
        if result.fragility_score > 0.7 {
            let degraded_condition = Condition {
                type_: "Degraded".to_string(),
                status: "True".to_string(),
                last_transition_time: Utc::now().to_rfc3339(),
                reason: "QuorumFragile".to_string(),
                message: format!(
                    "Quorum fragility score {:.3} exceeds threshold (critical_nodes={}, min_overlap={})",
                    result.fragility_score,
                    result.critical_nodes.len(),
                    result.min_overlap
                ),
                observed_generation: None,
            };

            // Update or add the Degraded condition
            if let Some(pos) = status.conditions.iter().position(|c| c.type_ == "Degraded") {
                status.conditions[pos] = degraded_condition;
            } else {
                status.conditions.push(degraded_condition);
            }
        } else {
            // Remove Degraded condition if fragility is acceptable
            status.conditions.retain(|c| c.type_ != "Degraded" || c.reason != "QuorumFragile");
        }

        // Patch the status
        let patch = serde_json::json!({
            "status": status
        });

        let _: StellarNode = api.patch_status(
            name,
            &PatchParams::default(),
            &Patch::Merge(&patch),
        )
        .await
        .map_err(QuorumAnalysisError::KubeError)?;

        info!(
            "Updated status for {}/{} with fragility score {:.3}",
            namespace, name, result.fragility_score
        );

        Ok(())
    }

    /// Update node status preserving last known good score on error
    pub async fn update_node_status_on_error(
        &self,
        _client: &Client,
        node: &StellarNode,
        error: &QuorumAnalysisError,
    ) -> Result<()> {
        let namespace = node.metadata.namespace.as_ref().ok_or_else(|| {
            QuorumAnalysisError::InvalidTopology("Node has no namespace".to_string())
        })?;

        let name = node.metadata.name.as_ref().ok_or_else(|| {
            QuorumAnalysisError::InvalidTopology("Node has no name".to_string())
        })?;

        warn!(
            "Quorum analysis failed for {}/{}: {}",
            namespace, name, error
        );

        // Preserve last known good score, don't update timestamp
        // This ensures stale data is not propagated
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyzer_creation() {
        let analyzer = QuorumAnalyzer::new(Duration::from_secs(10), 100);
        assert!(analyzer.cache.last_analysis.is_none());
    }

    #[test]
    fn test_fragility_score_bounds() {
        let analyzer = QuorumAnalyzer::new(Duration::from_secs(10), 100);
        
        // Test various scenarios
        let score1 = analyzer.calculate_fragility_score(0, 5, 0.0, 10);
        assert!(score1 >= 0.0 && score1 <= 1.0);

        let score2 = analyzer.calculate_fragility_score(5, 0, 100.0, 10);
        assert!(score2 >= 0.0 && score2 <= 1.0);

        let score3 = analyzer.calculate_fragility_score(10, 0, 1000.0, 10);
        assert!(score3 >= 0.0 && score3 <= 1.0);
    }

    #[test]
    fn test_fragility_score_empty_quorum() {
        let analyzer = QuorumAnalyzer::new(Duration::from_secs(10), 100);
        let score = analyzer.calculate_fragility_score(0, 0, 0.0, 0);
        assert_eq!(score, 1.0); // Maximum fragility
    }
}
