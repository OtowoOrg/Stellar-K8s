//! Quorum analyzer orchestrator
//!
//! This module orchestrates the quorum analysis process, calculating the fragility score
//! and coordinating between the SCP client, graph analysis, and latency tracking.

use super::error::{QuorumAnalysisError, Result};
use super::graph::QuorumGraph;
use super::latency::ConsensusLatencyTracker;
use super::scp_client::ScpClient;
use super::types::QuorumSetInfo;
use super::uptime::PeerUptimeTracker;
use crate::crd::types::Condition;
use crate::crd::StellarNode;
use chrono::{DateTime, Utc};
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Main quorum analyzer
pub struct QuorumAnalyzer {
    scp_client: ScpClient,
    latency_tracker: ConsensusLatencyTracker,
    uptime_tracker: PeerUptimeTracker,
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
    pub recommendation: Option<QuorumSetRecommendation>,
}

/// Safety-checked quorum-set recommendation based on peer health.
#[derive(Clone, Debug)]
pub struct QuorumSetRecommendation {
    pub recommended_validators: Vec<String>,
    pub recommended_threshold: u32,
    pub message: String,
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
    pub fn new(timeout: Duration, window_size: usize, max_attempts: u32) -> Self {
        Self {
            scp_client: ScpClient::new(timeout, max_attempts),
            latency_tracker: ConsensusLatencyTracker::new(window_size),
            uptime_tracker: PeerUptimeTracker::new(window_size),
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

        // Query quorum sets from all validators and record measured latency.
        let mut quorum_sets = Vec::new();

        for pod_ip in &pod_ips {
            let start = Instant::now();
            match self.scp_client.query_scp_state(pod_ip).await {
                Ok(scp_state) => {
                    let elapsed_ms = start.elapsed().as_millis() as u64;
                    let ledger_seq = scp_state.ballot_state.ballot_counter as u64;
                    self.latency_tracker
                        .record_latency(&scp_state.node_id, ledger_seq, elapsed_ms);

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
        let cached_analysis = if self.should_use_cache(topology_hash) {
            debug!("Using cached quorum topology analysis (health still re-sampled)");
            self.cache.last_analysis.clone()
        } else {
            None
        };

        // Determine the peer universe for uptime sampling.
        let all_quorum_peers: HashSet<String> = quorum_sets
            .iter()
            .flat_map(|(_, qset)| {
                let mut ids = Vec::new();
                ids.extend(qset.validators.iter().cloned());
                for inner in &qset.inner_sets {
                    ids.extend(inner.validators.iter().cloned());
                }
                ids
            })
            .collect();

        // Sample uptime: for each peer in the quorum set, count how many observed
        // validators currently report it as connected/authenticated.
        let is_connected = |state: &str| {
            let s = state.to_ascii_lowercase();
            s.contains("connected") || s.contains("authenticated") || s.contains("established")
        };

        let mut connected_count: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let total_pods = pod_ips.len().max(1);

        for pod_ip in &pod_ips {
            match self.scp_client.query_peers(pod_ip).await {
                Ok(peers) => {
                    for peer in peers {
                        if all_quorum_peers.contains(&peer.id) && is_connected(&peer.state) {
                            *connected_count.entry(peer.id).or_insert(0) += 1;
                        }
                    }
                }
                Err(e) => {
                    debug!("Failed to query peers from {}: {}", pod_ip, e);
                }
            }
        }

        for peer_id in &all_quorum_peers {
            let count = connected_count.get(peer_id).copied().unwrap_or(0);
            let uptime_ratio = count as f64 / total_pods as f64;
            self.uptime_tracker
                .record_uptime_ratio(peer_id, uptime_ratio);
        }

        // Determine baseline topology-derived metrics (possibly from cache).
        let (critical_nodes, baseline_min_overlap) = if let Some(cached) = cached_analysis {
            (cached.critical_nodes, cached.min_overlap)
        } else {
            // Build quorum graph (expensive slice enumeration).
            let graph = QuorumGraph::from_quorum_sets(quorum_sets.clone());
            let critical_analysis = graph.find_critical_nodes();
            let overlap_analysis = graph.calculate_overlaps();
            (
                critical_analysis.critical_nodes,
                overlap_analysis.min_overlap,
            )
        };

        // Get latency variance
        let latency_variance = self.latency_tracker.get_variance_across_validators();

        let total_validators = quorum_sets.len();

        // Calculate fragility score
        let fragility_score = self.calculate_fragility_score(
            critical_nodes.len(),
            baseline_min_overlap,
            latency_variance,
            total_validators,
        );

        // Recommend a safer quorum set by replacing unhealthy peers.
        let recommendation =
            self.recommend_quorum_set(&all_quorum_peers, &quorum_sets, baseline_min_overlap);

        let result = QuorumAnalysisResult {
            critical_nodes,
            min_overlap: baseline_min_overlap,
            latency_variance,
            fragility_score,
            timestamp: Utc::now(),
            recommendation,
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
        score.clamp(0.0, 1.0)
    }

    /// Recommend a quorum set update based on rolling peer uptime/latency.
    ///
    /// This is a conservative heuristic:
    /// - Only triggers when baseline quorum peers have low uptime and/or high latency.
    /// - Picks replacement validators from the union of observed peers.
    /// - Refuses to recommend if the resulting topology loses quorum intersection or
    ///   reduces overlap vs the baseline topology (best-effort safety guard).
    fn recommend_quorum_set(
        &self,
        all_quorum_peers: &HashSet<String>,
        quorum_sets: &[(String, QuorumSetInfo)],
        baseline_min_overlap: usize,
    ) -> Option<QuorumSetRecommendation> {
        const MIN_UPTIME_RATIO: f64 = 0.80;
        const MAX_LATENCY_MS: f64 = 500.0;
        const LATENCY_SCORE_SCALE_MS: f64 = 200.0;

        let baseline_qset = quorum_sets.iter().map(|(_, q)| q).max_by_key(|q| {
            let inner = q
                .inner_sets
                .iter()
                .map(|i| i.validators.len())
                .sum::<usize>();
            q.validators.len() + inner
        })?;

        // Flatten direct validators + inner-set validators into a single peer list.
        let mut baseline_validators: Vec<String> = baseline_qset.validators.clone();
        for inner in &baseline_qset.inner_sets {
            baseline_validators.extend(inner.validators.clone());
        }
        baseline_validators.sort_unstable();
        baseline_validators.dedup();

        if baseline_validators.is_empty() {
            return None;
        }

        // Identify baseline unhealthy peers.
        let mut unhealthy_peers = Vec::new();
        for peer_id in &baseline_validators {
            let uptime_mean = self.uptime_tracker.get_mean_uptime(peer_id).unwrap_or(0.0);

            let latency_mean = self
                .latency_tracker
                .get_stats(peer_id)
                .map(|s| s.mean_ms)
                .unwrap_or(MAX_LATENCY_MS * 2.0);

            if uptime_mean < MIN_UPTIME_RATIO || latency_mean > MAX_LATENCY_MS {
                unhealthy_peers.push(peer_id.clone());
            }
        }

        if unhealthy_peers.is_empty() {
            return None;
        }

        // Score candidate peers by health.
        let mut scored: Vec<(String, f64)> = all_quorum_peers
            .iter()
            .map(|peer_id| {
                let uptime_mean = self.uptime_tracker.get_mean_uptime(peer_id).unwrap_or(0.0);

                let latency_mean = self
                    .latency_tracker
                    .get_stats(peer_id)
                    .map(|s| s.mean_ms)
                    .unwrap_or(MAX_LATENCY_MS * 2.0);

                // Convert latency into a [0,1] score (higher is better).
                let latency_score = 1.0 / (1.0 + (latency_mean / LATENCY_SCORE_SCALE_MS).max(0.0));

                // Weighted health: favor uptime, but still account for latency.
                let health_score = 0.6 * uptime_mean + 0.4 * latency_score;
                (peer_id.clone(), health_score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Replace unhealthy peers while keeping the quorum peer count stable.
        let new_validators_count = baseline_validators.len();
        let new_validators: Vec<String> = scored
            .into_iter()
            .take(new_validators_count)
            .map(|(id, _)| id)
            .collect();

        if new_validators.len() < 2 {
            return None;
        }

        // Preserve relative threshold ratio (best-effort) when changing validator count.
        let threshold_ratio = baseline_qset.threshold as f64 / baseline_validators.len() as f64;
        let mut recommended_threshold =
            (threshold_ratio * new_validators.len() as f64).ceil() as u32;
        recommended_threshold = recommended_threshold
            .max(1)
            .min(new_validators.len() as u32);

        // Build a recommended quorum set candidate and validate it retains quorum intersection.
        let recommended_qset = QuorumSetInfo {
            threshold: recommended_threshold,
            validators: new_validators.clone(),
            inner_sets: vec![],
        };

        let graph = QuorumGraph::from_quorum_sets(
            new_validators
                .iter()
                .cloned()
                .map(|v| (v, recommended_qset.clone()))
                .collect(),
        );

        let critical_analysis = graph.find_critical_nodes();
        if !critical_analysis.quorum_intersection_valid {
            return None;
        }

        let overlap_analysis = graph.calculate_overlaps();
        if overlap_analysis.min_overlap < baseline_min_overlap {
            return None;
        }

        let unhealthy_sample = unhealthy_peers
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");

        let message = format!(
            "Quorum optimization: replace {} low-health peer(s) (e.g. {}) with healthier peers; recommended threshold={} validators={}",
            unhealthy_peers.len(),
            unhealthy_sample,
            recommended_threshold,
            new_validators.len()
        );

        Some(QuorumSetRecommendation {
            recommended_validators: new_validators,
            recommended_threshold,
            message,
        })
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
        self.latency_tracker
            .record_latency(validator, ledger, latency_ms);
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

        let name =
            node.metadata.name.as_ref().ok_or_else(|| {
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
            status
                .conditions
                .retain(|c| c.type_ != "Degraded" || c.reason != "QuorumFragile");
        }

        // Add quorum optimization recommendation condition (health-based; safe-checked).
        if let Some(rec) = &result.recommendation {
            let recommendation_condition = Condition {
                type_: "QuorumOptimizationRecommended".to_string(),
                status: "True".to_string(),
                last_transition_time: Utc::now().to_rfc3339(),
                reason: "PeerHealthRebalance".to_string(),
                message: rec.message.clone(),
                observed_generation: None,
            };

            if let Some(pos) = status
                .conditions
                .iter()
                .position(|c| c.type_ == "QuorumOptimizationRecommended")
            {
                status.conditions[pos] = recommendation_condition;
            } else {
                status.conditions.push(recommendation_condition);
            }
        } else {
            status
                .conditions
                .retain(|c| c.type_ != "QuorumOptimizationRecommended");
        }

        // Patch the status
        let patch = serde_json::json!({
            "status": status
        });

        let _: StellarNode = api
            .patch_status(name, &PatchParams::default(), &Patch::Merge(&patch))
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

        let name =
            node.metadata.name.as_ref().ok_or_else(|| {
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
        let analyzer = QuorumAnalyzer::new(Duration::from_secs(10), 100, 3);
        assert!(analyzer.cache.last_analysis.is_none());
    }

    #[test]
    fn test_fragility_score_bounds() {
        let analyzer = QuorumAnalyzer::new(Duration::from_secs(10), 100, 3);

        // Test various scenarios
        let score1 = analyzer.calculate_fragility_score(0, 5, 0.0, 10);
        assert!((0.0..=1.0).contains(&score1));

        let score2 = analyzer.calculate_fragility_score(5, 0, 100.0, 10);
        assert!((0.0..=1.0).contains(&score2));

        let score3 = analyzer.calculate_fragility_score(10, 0, 1000.0, 10);
        assert!((0.0..=1.0).contains(&score3));
    }

    #[test]
    fn test_fragility_score_empty_quorum() {
        let analyzer = QuorumAnalyzer::new(Duration::from_secs(10), 100, 3);
        let score = analyzer.calculate_fragility_score(0, 0, 0.0, 0);
        assert_eq!(score, 1.0); // Maximum fragility
    }
}
