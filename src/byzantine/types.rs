//! Data types for Byzantine monitoring observations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single consensus observation from one watcher at one point in time.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsensusObservation {
    /// Watcher identity — unique per deployed instance.
    pub watcher_id: String,

    /// Cloud provider label (e.g. "aws", "gcp", "azure", "on-prem").
    pub cloud: String,

    /// Geographic region label (e.g. "us-east-1", "eu-west-1", "ap-south-1").
    pub region: String,

    /// The Stellar network being observed (e.g. "mainnet", "testnet").
    pub network: String,

    /// The Stellar Core HTTP endpoint being polled.
    pub node_endpoint: String,

    /// Latest externalized ledger sequence number.
    pub ledger_sequence: u64,

    /// Hex-encoded ledger close hash (32 bytes → 64 hex chars).
    /// Empty string if the node is not yet externalized.
    pub ledger_hash: String,

    /// Whether the node reports itself as EXTERNALIZED (in consensus).
    pub is_externalized: bool,

    /// Timestamp of this observation.
    pub observed_at: DateTime<Utc>,
}

/// Aggregated view across all watchers for a single network.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AggregatedConsensusView {
    /// The Stellar network.
    pub network: String,

    /// Total number of watchers that reported in the last evaluation window.
    pub total_watchers: usize,

    /// Number of watchers that agree on the majority ledger hash.
    pub agreeing_watchers: usize,

    /// Number of watchers that see a *different* ledger hash than the majority.
    pub diverging_watchers: usize,

    /// The majority ledger hash (most common hash among all watchers).
    pub majority_hash: String,

    /// The majority ledger sequence.
    pub majority_sequence: u64,

    /// Fraction of watchers that diverge from the majority (0.0 – 1.0).
    pub divergence_ratio: f64,

    /// Whether the divergence ratio exceeds the alert threshold (>20%).
    pub byzantine_alert: bool,

    /// Watchers that are diverging (for diagnostics).
    pub diverging_watcher_ids: Vec<String>,

    /// Timestamp of this aggregation.
    pub aggregated_at: DateTime<Utc>,
}

/// Response from Stellar Core `/info` endpoint (subset we care about).
#[derive(Debug, Deserialize)]
pub struct StellarCoreInfoResponse {
    pub info: StellarCoreInfo,
}

#[derive(Debug, Deserialize)]
pub struct StellarCoreInfo {
    /// Ledger state
    pub ledger: StellarCoreLedger,

    /// Sync state string, e.g. "Synced!", "Catching up", "Booting"
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct StellarCoreLedger {
    /// Latest ledger sequence number
    pub num: u64,

    /// Latest ledger close hash (hex)
    pub hash: String,
}

/// Configuration for a single Watcher instance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatcherConfig {
    /// Unique identifier for this watcher instance.
    pub watcher_id: String,

    /// Cloud provider (aws / gcp / azure / on-prem / …).
    pub cloud: String,

    /// Geographic region (us-east-1 / eu-west-1 / …).
    pub region: String,

    /// Stellar network name (mainnet / testnet / futurenet / custom).
    pub network: String,

    /// HTTP endpoint of the Stellar Core node to poll.
    /// Example: `http://stellar-core.stellar-system.svc.cluster.local:11626`
    pub node_endpoint: String,

    /// How often to poll the node (seconds). Default: 10.
    pub poll_interval_secs: u64,

    /// HTTP request timeout (seconds). Default: 5.
    pub request_timeout_secs: u64,

    /// Address to bind the Prometheus `/metrics` HTTP server on.
    /// Default: `0.0.0.0:9101`
    pub metrics_bind_addr: String,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            watcher_id: "watcher-default".to_string(),
            cloud: "unknown".to_string(),
            region: "unknown".to_string(),
            network: "mainnet".to_string(),
            node_endpoint: "http://localhost:11626".to_string(),
            poll_interval_secs: 10,
            request_timeout_secs: 5,
            metrics_bind_addr: "0.0.0.0:9101".to_string(),
        }
    }
}
