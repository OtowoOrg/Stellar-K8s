//! ReadOnlyPool Custom Resource Definition
//!
//! The ReadOnlyPool CRD represents a horizontally scalable pool of read-only Stellar nodes.
//! Unlike validators which are sensitive and read-only, read-only nodes can be scaled
//! horizontally and support weighted load balancing and shard balancing.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::{Condition, ResourceRequirements, StellarNetwork, StorageConfig};

/// The ReadOnlyPool CRD represents a pool of read-only Stellar nodes.
///
/// # Example
///
/// ```yaml
/// apiVersion: stellar.org/v1alpha1
/// kind: ReadOnlyPool
/// metadata:
///   name: mainnet-readonly-pool
///   namespace: stellar-nodes
/// spec:
///   network: Mainnet
///   version: "v21.0.0"
///   minReplicas: 3
///   maxReplicas: 20
///   targetReplicas: 5
///   loadBalancing:
///     enabled: true
///     freshNodeWeight: 100
///     laggingNodeWeight: 10
///     lagThreshold: 1000
///   shardBalancing:
///     enabled: true
///     shardCount: 4
///     historyArchiveUrls:
///       - "https://archive1.example.com"
///       - "https://archive2.example.com"
/// ```
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "ReadOnlyPool",
    namespaced,
    status = "ReadOnlyPoolStatus",
    shortname = "rop",
    printcolumn = r#"{"name":"Network","type":"string","jsonPath":".spec.network"}"#,
    printcolumn = r#"{"name":"Replicas","type":"integer","jsonPath":".status.currentReplicas"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ReadOnlyPoolSpec {
    /// Target Stellar network (Mainnet, Testnet, Futurenet, or Custom)
    pub network: StellarNetwork,

    /// Container image version to use (e.g., "v21.0.0")
    pub version: String,

    /// Minimum number of replicas in the pool
    #[serde(default = "default_min_replicas")]
    pub min_replicas: i32,

    /// Maximum number of replicas in the pool
    #[serde(default = "default_max_replicas")]
    pub max_replicas: i32,

    /// Target number of replicas (used for initial scaling)
    #[serde(default = "default_target_replicas")]
    pub target_replicas: i32,

    /// Compute resource requirements (CPU and memory)
    #[serde(default)]
    pub resources: ResourceRequirements,

    /// Storage configuration for persistent data
    #[serde(default)]
    pub storage: StorageConfig,

    /// Weighted load balancing configuration
    #[serde(default)]
    pub load_balancing: LoadBalancingConfig,

    /// Shard balancing configuration for large history archives
    #[serde(default)]
    pub shard_balancing: ShardBalancingConfig,

    /// History archive URLs to sync from
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history_archive_urls: Vec<String>,

    /// Stellar Core configuration overrides (TOML format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core_config_override: Option<String>,

    /// Enable alerting via PrometheusRule or ConfigMap
    #[serde(default)]
    pub alerting: bool,
}

fn default_min_replicas() -> i32 {
    3
}

fn default_max_replicas() -> i32 {
    20
}

fn default_target_replicas() -> i32 {
    5
}

impl ReadOnlyPoolSpec {
    /// Validate the spec
    pub fn validate(&self) -> Result<(), String> {
        if self.min_replicas < 1 {
            return Err("minReplicas must be at least 1".to_string());
        }
        if self.max_replicas < self.min_replicas {
            return Err("maxReplicas must be >= minReplicas".to_string());
        }
        if self.target_replicas < self.min_replicas || self.target_replicas > self.max_replicas {
            return Err("targetReplicas must be between minReplicas and maxReplicas".to_string());
        }
        if self.shard_balancing.enabled && self.shard_balancing.shard_count < 1 {
            return Err("shardBalancing.shardCount must be at least 1".to_string());
        }
        if self.shard_balancing.enabled && self.history_archive_urls.is_empty() {
            return Err("historyArchiveUrls must not be empty when shardBalancing is enabled".to_string());
        }
        Ok(())
    }

    /// Get the container image for read-only nodes
    pub fn container_image(&self) -> String {
        format!("stellar/stellar-core:{}", self.version)
    }
}

/// Weighted load balancing configuration
///
/// Enables intelligent traffic distribution between fresh (up-to-date) nodes
/// and lagging (catching up) nodes. Fresh nodes receive more traffic.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadBalancingConfig {
    /// Enable weighted load balancing
    #[serde(default)]
    pub enabled: bool,

    /// Weight for fresh nodes (nodes within lag threshold)
    /// Higher weight means more traffic
    #[serde(default = "default_fresh_weight")]
    pub fresh_node_weight: i32,

    /// Weight for lagging nodes (nodes beyond lag threshold)
    /// Lower weight means less traffic
    #[serde(default = "default_lagging_weight")]
    pub lagging_node_weight: i32,

    /// Ledger lag threshold in sequence numbers
    /// Nodes with lag > this threshold are considered "lagging"
    #[serde(default = "default_lag_threshold")]
    pub lag_threshold: u64,

    /// Update interval for weight recalculation (seconds)
    #[serde(default = "default_weight_update_interval")]
    pub update_interval_seconds: u64,
}

fn default_fresh_weight() -> i32 {
    100
}

fn default_lagging_weight() -> i32 {
    10
}

fn default_lag_threshold() -> u64 {
    1000
}

fn default_weight_update_interval() -> u64 {
    30
}

/// Shard balancing configuration for large history archives
///
/// Distributes history archive data across multiple shards to enable
/// parallel processing and reduce storage requirements per node.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ShardBalancingConfig {
    /// Enable shard balancing
    #[serde(default)]
    pub enabled: bool,

    /// Number of shards to distribute data across
    #[serde(default = "default_shard_count")]
    pub shard_count: i32,

    /// Shard assignment strategy
    #[serde(default)]
    pub strategy: ShardStrategy,

    /// Enable automatic rebalancing when nodes are added/removed
    #[serde(default = "default_true")]
    pub auto_rebalance: bool,
}

fn default_shard_count() -> i32 {
    4
}

fn default_true() -> bool {
    true
}

/// Shard assignment strategy
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ShardStrategy {
    /// Round-robin assignment (default)
    #[default]
    RoundRobin,
    /// Hash-based assignment (consistent hashing)
    HashBased,
    /// Manual assignment via annotations
    Manual,
}

/// Status subresource for ReadOnlyPool
///
/// Reports the current state of the read-only pool including replica counts,
/// load balancing weights, and shard assignments.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadOnlyPoolStatus {
    /// Current number of replicas
    #[serde(default)]
    pub current_replicas: i32,

    /// Number of ready replicas
    #[serde(default)]
    pub ready_replicas: i32,

    /// Number of fresh (up-to-date) replicas
    #[serde(default)]
    pub fresh_replicas: i32,

    /// Number of lagging replicas
    #[serde(default)]
    pub lagging_replicas: i32,

    /// Observed generation for status sync detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,

    /// Readiness conditions following Kubernetes conventions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,

    /// Current load balancing weights per replica
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replica_weights: Vec<ReplicaWeight>,

    /// Shard assignments per replica
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shard_assignments: Vec<ShardAssignment>,

    /// Average ledger sequence across all replicas
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_ledger_sequence: Option<u64>,

    /// Latest ledger sequence from the network
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_latest_ledger: Option<u64>,

    /// Average lag across all replicas
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_lag: Option<i64>,
}

/// Weight information for a single replica
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplicaWeight {
    /// Replica name (pod name)
    pub replica_name: String,

    /// Current weight (0-100)
    pub weight: i32,

    /// Ledger sequence for this replica
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_sequence: Option<u64>,

    /// Lag from network latest (in ledger sequences)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lag: Option<i64>,

    /// Whether this replica is considered "fresh"
    pub is_fresh: bool,

    /// Last update timestamp
    pub last_updated: String,
}

/// Shard assignment for a single replica
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ShardAssignment {
    /// Replica name (pod name)
    pub replica_name: String,

    /// Assigned shard ID (0-based)
    pub shard_id: i32,

    /// History archive URL for this shard
    pub archive_url: String,

    /// Ledger range for this shard (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_range: Option<LedgerRange>,
}

/// Ledger range for a shard
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LedgerRange {
    /// Start ledger sequence (inclusive)
    pub start: u64,

    /// End ledger sequence (inclusive, None means "latest")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
}

impl ReadOnlyPoolStatus {
    /// Check if the pool is ready
    pub fn is_ready(&self) -> bool {
        let has_ready_condition = self
            .conditions
            .iter()
            .any(|c| c.type_ == "Ready" && c.status == "True");

        has_ready_condition && self.ready_replicas >= self.current_replicas
    }

    /// Get a condition by type
    pub fn get_condition(&self, condition_type: &str) -> Option<&Condition> {
        self.conditions.iter().find(|c| c.type_ == condition_type)
    }
}
