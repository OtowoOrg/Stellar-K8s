//! Topological Health Consumer for SCP Analytics
//!
//! This module implements a Kafka consumer that processes SCP messages
//! and computes real-time topological health metrics for the Stellar network.
//!
//! # Metrics Computed
//!
//! - **Health Score**: Overall network health (0.0 = unhealthy, 1.0 = healthy)
//! - **Active Validators**: Number of validators actively participating
//! - **Stalled Validators**: Validators with no phase change in 30+ seconds
//! - **Critical Nodes**: Validators whose removal would break consensus
//! - **Quorum Intersection**: Whether the network has quorum intersection
//! - **Consensus Latency**: Average time to reach consensus
//! - **Partition Detection**: Whether network partitions are detected

use super::error::{QuorumAnalysisError, Result};
use super::scp_kafka_stream::ScpMessage;
use chrono::{DateTime, Duration, Utc};
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::message::Message;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Topological health metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologicalHealth {
    /// Timestamp of analysis
    pub timestamp: DateTime<Utc>,

    /// Network identifier
    pub network: String,

    /// Overall quorum health score (0.0 = unhealthy, 1.0 = healthy)
    pub health_score: f64,

    /// Number of active validators
    pub active_validators: u32,

    /// Number of stalled validators (no phase change in 30s)
    pub stalled_validators: u32,

    /// Number of critical nodes (removal breaks consensus)
    pub critical_nodes: u32,

    /// Quorum intersection status
    pub has_quorum_intersection: bool,

    /// Average consensus latency in milliseconds
    pub avg_consensus_latency_ms: f64,

    /// Network partition detected
    pub partition_detected: bool,

    /// List of validator health statuses
    pub validator_health: Vec<ValidatorHealth>,
}

/// Individual validator health status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorHealth {
    /// Validator public key
    pub node_id: String,

    /// Current SCP phase
    pub phase: String,

    /// Time since last phase change in seconds
    pub time_since_last_change_secs: u32,

    /// Is this validator critical for consensus?
    pub is_critical: bool,

    /// Is this validator stalled?
    pub is_stalled: bool,

    /// Number of peers connected
    pub peer_count: u32,

    /// Last seen timestamp
    pub last_seen: DateTime<Utc>,
}

/// Validator state tracker
#[derive(Debug, Clone)]
struct ValidatorState {
    node_id: String,
    phase: String,
    ballot_counter: u32,
    last_phase_change: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    quorum_threshold: u32,
    quorum_validators: Vec<String>,
}

/// Topological health consumer configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyHealthConfig {
    /// Kafka bootstrap servers
    pub bootstrap_servers: String,

    /// Topic to consume from
    #[serde(default = "default_topic")]
    pub topic: String,

    /// Consumer group ID
    #[serde(default = "default_group_id")]
    pub group_id: String,

    /// Stall threshold in seconds
    #[serde(default = "default_stall_threshold")]
    pub stall_threshold_secs: u32,

    /// Analysis window in seconds
    #[serde(default = "default_analysis_window")]
    pub analysis_window_secs: u32,

    /// Enable auto-commit
    #[serde(default = "default_auto_commit")]
    pub enable_auto_commit: bool,
}

fn default_topic() -> String {
    "stellar-scp-messages".to_string()
}

fn default_group_id() -> String {
    "topology-health-consumer".to_string()
}

fn default_stall_threshold() -> u32 {
    30 // 30 seconds
}

fn default_analysis_window() -> u32 {
    60 // 60 seconds
}

fn default_auto_commit() -> bool {
    true
}

impl Default for TopologyHealthConfig {
    fn default() -> Self {
        Self {
            bootstrap_servers: "localhost:9092".to_string(),
            topic: default_topic(),
            group_id: default_group_id(),
            stall_threshold_secs: default_stall_threshold(),
            analysis_window_secs: default_analysis_window(),
            enable_auto_commit: default_auto_commit(),
        }
    }
}

/// Topological health consumer
pub struct TopologyHealthConsumer {
    consumer: StreamConsumer,
    config: TopologyHealthConfig,
    validator_states: Arc<RwLock<HashMap<String, ValidatorState>>>,
    latest_health: Arc<RwLock<Option<TopologicalHealth>>>,
}

impl TopologyHealthConsumer {
    /// Create a new topology health consumer
    pub fn new(config: TopologyHealthConfig) -> Result<Self> {
        let mut client_config = ClientConfig::new();
        client_config
            .set("bootstrap.servers", &config.bootstrap_servers)
            .set("group.id", &config.group_id)
            .set("enable.auto.commit", config.enable_auto_commit.to_string())
            .set("auto.offset.reset", "latest")
            .set("session.timeout.ms", "30000")
            .set("heartbeat.interval.ms", "3000");

        let consumer: StreamConsumer = client_config
            .create()
            .map_err(|e| QuorumAnalysisError::KafkaError(e.to_string()))?;

        consumer
            .subscribe(&[&config.topic])
            .map_err(|e| QuorumAnalysisError::KafkaError(e.to_string()))?;

        Ok(Self {
            consumer,
            config,
            validator_states: Arc::new(RwLock::new(HashMap::new())),
            latest_health: Arc::new(RwLock::new(None)),
        })
    }

    /// Start consuming and analyzing SCP messages
    pub async fn start(&self) -> Result<()> {
        info!(
            "Starting topology health consumer for topic {}",
            self.config.topic
        );

        loop {
            match self.consumer.recv().await {
                Ok(message) => {
                    if let Some(payload) = message.payload() {
                        match self.process_message(payload).await {
                            Ok(()) => {}
                            Err(e) => {
                                warn!("Error processing message: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Kafka consumer error: {}", e);
                }
            }
        }
    }

    /// Process a single SCP message
    async fn process_message(&self, payload: &[u8]) -> Result<()> {
        // Deserialize message
        let scp_message: ScpMessage = serde_json::from_slice(payload)?;

        debug!(
            "Processing SCP message from {} (phase: {}, ballot: {})",
            scp_message.node_id, scp_message.phase, scp_message.ballot_counter
        );

        // Update validator state
        let mut states = self.validator_states.write().await;

        let now = Utc::now();
        let last_phase_change = if let Some(existing) = states.get(&scp_message.node_id) {
            if existing.phase != scp_message.phase
                || existing.ballot_counter != scp_message.ballot_counter
            {
                now
            } else {
                existing.last_phase_change
            }
        } else {
            now
        };

        let state = ValidatorState {
            node_id: scp_message.node_id.clone(),
            phase: scp_message.phase.clone(),
            ballot_counter: scp_message.ballot_counter,
            last_phase_change,
            last_seen: now,
            quorum_threshold: scp_message.quorum_set.threshold,
            quorum_validators: scp_message.quorum_set.validators.clone(),
        };

        states.insert(scp_message.node_id.clone(), state);

        // Clean up old states (older than analysis window)
        let cutoff = now - Duration::seconds(self.config.analysis_window_secs as i64);
        states.retain(|_, state| state.last_seen > cutoff);

        drop(states); // Release lock

        // Compute health metrics
        self.compute_health_metrics().await?;

        Ok(())
    }

    /// Compute topological health metrics
    async fn compute_health_metrics(&self) -> Result<()> {
        let states = self.validator_states.read().await;
        let now = Utc::now();

        if states.is_empty() {
            return Ok(());
        }

        // Count active validators
        let active_validators = states.len() as u32;

        // Identify stalled validators
        let stall_threshold = Duration::seconds(self.config.stall_threshold_secs as i64);
        let stalled_validators: Vec<_> = states
            .values()
            .filter(|state| now - state.last_phase_change > stall_threshold)
            .collect();
        let stalled_count = stalled_validators.len() as u32;

        // Identify critical nodes (simplified - nodes with high quorum threshold)
        let critical_nodes: Vec<_> = states
            .values()
            .filter(|state| state.quorum_threshold >= (active_validators / 2))
            .collect();
        let critical_count = critical_nodes.len() as u32;

        // Check quorum intersection (simplified - check if all validators share common validators)
        let has_quorum_intersection = self.check_quorum_intersection(&states);

        // Calculate average consensus latency (time between phase changes)
        let avg_consensus_latency_ms = self.calculate_avg_latency(&states);

        // Detect network partitions (simplified - check for validators in different phases)
        let partition_detected = self.detect_partition(&states);

        // Calculate health score
        let health_score = self.calculate_health_score(
            active_validators,
            stalled_count,
            critical_count,
            has_quorum_intersection,
            partition_detected,
        );

        // Build validator health list
        let validator_health: Vec<ValidatorHealth> = states
            .values()
            .map(|state| {
                let time_since_last_change = (now - state.last_phase_change).num_seconds() as u32;
                let is_stalled = time_since_last_change > self.config.stall_threshold_secs;
                let is_critical = state.quorum_threshold >= (active_validators / 2);

                ValidatorHealth {
                    node_id: state.node_id.clone(),
                    phase: state.phase.clone(),
                    time_since_last_change_secs: time_since_last_change,
                    is_critical,
                    is_stalled,
                    peer_count: state.quorum_validators.len() as u32,
                    last_seen: state.last_seen,
                }
            })
            .collect();

        // Determine network (assume all validators are on same network)
        let network = states
            .values()
            .next()
            .map(|_| "mainnet".to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let health = TopologicalHealth {
            timestamp: now,
            network,
            health_score,
            active_validators,
            stalled_validators: stalled_count,
            critical_nodes: critical_count,
            has_quorum_intersection,
            avg_consensus_latency_ms,
            partition_detected,
            validator_health,
        };

        // Update latest health
        let mut latest = self.latest_health.write().await;
        *latest = Some(health.clone());

        info!(
            "Topological health: score={:.2}, active={}, stalled={}, critical={}, intersection={}",
            health.health_score,
            health.active_validators,
            health.stalled_validators,
            health.critical_nodes,
            health.has_quorum_intersection
        );

        Ok(())
    }

    /// Check if quorum intersection exists (simplified)
    fn check_quorum_intersection(&self, states: &HashMap<String, ValidatorState>) -> bool {
        if states.len() < 2 {
            return true;
        }

        // Simplified: check if there's at least one common validator across all quorum sets
        let mut common_validators: Option<Vec<String>> = None;

        for state in states.values() {
            if let Some(ref common) = common_validators {
                let intersection: Vec<String> = common
                    .iter()
                    .filter(|v| state.quorum_validators.contains(v))
                    .cloned()
                    .collect();

                if intersection.is_empty() {
                    return false;
                }

                common_validators = Some(intersection);
            } else {
                common_validators = Some(state.quorum_validators.clone());
            }
        }

        true
    }

    /// Calculate average consensus latency
    fn calculate_avg_latency(&self, states: &HashMap<String, ValidatorState>) -> f64 {
        if states.is_empty() {
            return 0.0;
        }

        let now = Utc::now();
        let total_latency: i64 = states
            .values()
            .map(|state| (now - state.last_phase_change).num_milliseconds())
            .sum();

        (total_latency as f64) / (states.len() as f64)
    }

    /// Detect network partition
    fn detect_partition(&self, states: &HashMap<String, ValidatorState>) -> bool {
        if states.len() < 2 {
            return false;
        }

        // Check if validators are in significantly different phases
        let phases: Vec<&str> = states.values().map(|s| s.phase.as_str()).collect();

        let prepare_count = phases.iter().filter(|&&p| p == "PREPARE").count();
        let externalize_count = phases.iter().filter(|&&p| p == "EXTERNALIZE").count();

        // If more than 30% are in different phases, consider it a partition
        let max_phase_count = prepare_count.max(externalize_count);
        let min_phase_count = prepare_count.min(externalize_count);

        if min_phase_count > 0 {
            let ratio = (min_phase_count as f64) / (max_phase_count as f64);
            ratio > 0.3
        } else {
            false
        }
    }

    /// Calculate overall health score
    fn calculate_health_score(
        &self,
        active: u32,
        stalled: u32,
        critical: u32,
        has_intersection: bool,
        partition: bool,
    ) -> f64 {
        let mut score = 1.0;

        // Penalize for stalled validators
        if active > 0 {
            let stalled_ratio = (stalled as f64) / (active as f64);
            score -= stalled_ratio * 0.3;
        }

        // Penalize for high number of critical nodes
        if active > 0 {
            let critical_ratio = (critical as f64) / (active as f64);
            if critical_ratio > 0.5 {
                score -= 0.2;
            }
        }

        // Penalize for no quorum intersection
        if !has_intersection {
            score -= 0.3;
        }

        // Penalize for network partition
        if partition {
            score -= 0.2;
        }

        score.max(0.0).min(1.0)
    }

    /// Get the latest health metrics
    pub async fn get_latest_health(&self) -> Option<TopologicalHealth> {
        self.latest_health.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topology_health_config_defaults() {
        let config = TopologyHealthConfig::default();
        assert_eq!(config.topic, "stellar-scp-messages");
        assert_eq!(config.group_id, "topology-health-consumer");
        assert_eq!(config.stall_threshold_secs, 30);
        assert_eq!(config.analysis_window_secs, 60);
        assert!(config.enable_auto_commit);
    }

    #[test]
    fn test_health_score_calculation() {
        let consumer = TopologyHealthConsumer {
            consumer: unsafe { std::mem::zeroed() }, // Mock for testing
            config: TopologyHealthConfig::default(),
            validator_states: Arc::new(RwLock::new(HashMap::new())),
            latest_health: Arc::new(RwLock::new(None)),
        };

        // Perfect health
        let score = consumer.calculate_health_score(10, 0, 0, true, false);
        assert_eq!(score, 1.0);

        // Some stalled validators
        let score = consumer.calculate_health_score(10, 3, 0, true, false);
        assert!(score < 1.0 && score > 0.6);

        // No quorum intersection
        let score = consumer.calculate_health_score(10, 0, 0, false, false);
        assert!(score < 0.8);

        // Network partition
        let score = consumer.calculate_health_score(10, 0, 0, true, true);
        assert!(score < 0.9);
    }
}
