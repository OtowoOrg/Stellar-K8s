//! Dynamic Quorum Set Optimizer
//!
//! Monitors validator peer health (latency, uptime, ledger lag) and
//! calculates trust scores to recommend or apply quorum set changes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::Utc;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::crd::types::{DynamicQuorumConfig, DynamicQuorumStatus, PeerHealthStatus};
use crate::error::{Error, Result};

/// Sample of peer performance data
#[derive(Debug, Clone)]
struct PerformanceSample {
    pub latency_ms: u32,
    pub is_up: bool,
    pub ledger_lag: u64,
}

/// History of performance for a specific peer
#[derive(Debug, Clone)]
struct PeerHistory {
    pub public_key: String,
    pub name: String,
    pub samples: Vec<PerformanceSample>,
}

impl PeerHistory {
    pub fn new(public_key: String, name: String) -> Self {
        Self {
            public_key,
            name,
            samples: Vec::new(),
        }
    }

    pub fn add_sample(&mut self, sample: PerformanceSample, window_size: usize) {
        self.samples.push(sample);
        if self.samples.len() > window_size {
            self.samples.remove(0);
        }
    }

    pub fn calculate_uptime_percent(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let up_count = self.samples.iter().filter(|s| s.is_up).count();
        (up_count as f32 / self.samples.len() as f32) * 100.0
    }

    pub fn calculate_avg_latency(&self) -> u32 {
        if self.samples.is_empty() {
            return 0;
        }
        let sum: u32 = self.samples.iter().map(|s| s.latency_ms).sum();
        sum / self.samples.len() as u32
    }

    pub fn calculate_avg_ledger_lag(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let sum: u64 = self.samples.iter().map(|s| s.ledger_lag).sum();
        sum / self.samples.len() as u64
    }

    pub fn calculate_trust_score(&self, config: &DynamicQuorumConfig) -> u32 {
        if self.samples.is_empty() {
            return 0;
        }

        let uptime = self.calculate_uptime_percent();
        let latency = self.calculate_avg_latency();
        let lag = self.calculate_avg_ledger_lag();

        let mut score: f32 = 100.0;

        // Uptime penalty
        if uptime < 100.0 {
            score -= (100.0 - uptime) * 2.0;
        }

        // Latency penalty
        if latency > config.latency_threshold_ms {
            let excess = (latency - config.latency_threshold_ms) as f32;
            score -= (excess / 100.0).min(50.0);
        }

        // Ledger lag penalty
        if lag > 10 {
            score -= (lag as f32 - 10.0) * 5.0;
        }

        score.clamp(0.0, 100.0) as u32
    }
}

/// Stellar Core /info response
#[derive(Debug, Deserialize)]
struct CoreInfo {
    pub info: CoreInfoDetails,
}

#[derive(Debug, Deserialize)]
struct CoreInfoDetails {
    pub ledger: CoreLedgerInfo,
    pub state: String,
}

#[derive(Debug, Deserialize)]
struct CoreLedgerInfo {
    pub age: u32,
}

// Removed CorePeersInfo as it was unused

/// Orchestrates quorum optimization
pub struct QuorumOptimizer {
    http_client: HttpClient,
    peer_histories: HashMap<String, PeerHistory>,
}

impl Default for QuorumOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

impl QuorumOptimizer {
    pub fn new() -> Self {
        Self {
            http_client: HttpClient::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("Failed to build HTTP client"),
            peer_histories: HashMap::new(),
        }
    }

    /// Update peer health data for a given node
    pub async fn update_node_health(
        &mut self,
        pod_ip: &str,
        public_key: &str,
        name: &str,
        config: &DynamicQuorumConfig,
    ) -> Result<()> {
        let start = Instant::now();
        let url = format!("http://{pod_ip}:11626/info");

        let response = match self.http_client.get(&url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                debug!("Failed to reach node {} at {}: {}", name, pod_ip, e);
                self.record_failure(public_key, name, config);
                return Err(Error::HttpError(e));
            }
        };

        if !response.status().is_success() {
            debug!(
                "Node {} at {} returned status {}",
                name,
                pod_ip,
                response.status()
            );
            self.record_failure(public_key, name, config);
            return Ok(());
        }

        let latency = start.elapsed().as_millis() as u32;

        match response.json::<CoreInfo>().await {
            Ok(core_info) => {
                let sample = PerformanceSample {
                    latency_ms: latency,
                    is_up: core_info.info.state == "Synced!",
                    ledger_lag: core_info.info.ledger.age as u64, // simplified lag measure
                };

                let history = self
                    .peer_histories
                    .entry(public_key.to_string())
                    .or_insert_with(|| PeerHistory::new(public_key.to_string(), name.to_string()));

                history.add_sample(sample, config.observation_window as usize);
            }
            Err(e) => {
                warn!("Failed to parse /info from {}: {}", name, e);
                self.record_failure(public_key, name, config);
            }
        }

        Ok(())
    }

    fn record_failure(&mut self, public_key: &str, name: &str, config: &DynamicQuorumConfig) {
        let history = self
            .peer_histories
            .entry(public_key.to_string())
            .or_insert_with(|| PeerHistory::new(public_key.to_string(), name.to_string()));

        history.add_sample(
            PerformanceSample {
                latency_ms: config.latency_threshold_ms * 2,
                is_up: false,
                ledger_lag: 100,
            },
            config.observation_window as usize,
        );
    }

    /// Generate status report for the CRD
    pub fn get_status(&self, config: &DynamicQuorumConfig) -> DynamicQuorumStatus {
        let mut peers = Vec::new();

        for history in self.peer_histories.values() {
            peers.push(PeerHealthStatus {
                public_key: history.public_key.clone(),
                name: history.name.clone(),
                latency_ms: history.calculate_avg_latency(),
                uptime_percent: history.calculate_uptime_percent(),
                ledger_lag: history.calculate_avg_ledger_lag(),
                trust_score: history.calculate_trust_score(config),
                last_seen: Utc::now().to_rfc3339(),
            });
        }

        DynamicQuorumStatus {
            peers,
            recommended_quorum_set: self.generate_recommended_vsl(config),
            last_optimized_at: Some(Utc::now().to_rfc3339()),
        }
    }

    fn generate_recommended_vsl(&self, config: &DynamicQuorumConfig) -> Option<String> {
        let trusted_peers: Vec<_> = self
            .peer_histories
            .values()
            .filter(|h| h.calculate_trust_score(config) >= config.min_trust_score)
            .collect();

        if trusted_peers.is_empty() {
            return None;
        }

        let mut toml = String::from("[QUORUM_SET]\n");
        let threshold = (trusted_peers.len() / 2) + 1;
        let pct = ((threshold as f64 / trusted_peers.len() as f64) * 100.0).ceil() as u32;

        toml.push_str(&format!("THRESHOLD_PERCENT={pct}\n"));

        let keys: Vec<String> = trusted_peers
            .iter()
            .map(|h| format!("\"{}\"", h.public_key))
            .collect();

        toml.push_str(&format!("VALIDATORS=[{}]\n", keys.join(", ")));

        Some(toml)
    }
}
