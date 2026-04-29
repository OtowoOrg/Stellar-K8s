//! Types for the fork detection sidecar.

use serde::{Deserialize, Serialize};

/// Configuration for the fork detector sidecar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForkDetectorConfig {
    /// HTTP endpoint of the local Stellar Core node.
    /// Default: `http://localhost:11626`
    pub local_endpoint: String,

    /// Public anchor node endpoints to compare against (minimum 3 recommended).
    pub anchor_endpoints: Vec<String>,

    /// How often to poll all nodes (seconds). Default: 5.
    pub poll_interval_secs: u64,

    /// HTTP request timeout per node (seconds). Default: 5.
    pub request_timeout_secs: u64,

    /// Number of consecutive diverging ledgers before alerting. Default: 3.
    pub divergence_threshold_ledgers: u64,

    /// Address to bind the Prometheus /metrics HTTP server. Default: `0.0.0.0:9102`
    pub metrics_bind_addr: String,

    /// Stellar network name (for labels). Default: `mainnet`
    pub network: String,

    /// Node identifier (for labels). Default: `local`
    pub node_id: String,
}

impl Default for ForkDetectorConfig {
    fn default() -> Self {
        Self {
            local_endpoint: "http://localhost:11626".to_string(),
            anchor_endpoints: vec![
                "https://horizon.stellar.org/ledgers?order=desc&limit=1".to_string(),
                "https://horizon-testnet.stellar.org/ledgers?order=desc&limit=1".to_string(),
                "https://stellar.expert/explorer/public/ledger".to_string(),
            ],
            poll_interval_secs: 5,
            request_timeout_secs: 5,
            divergence_threshold_ledgers: 3,
            metrics_bind_addr: "0.0.0.0:9102".to_string(),
            network: "mainnet".to_string(),
            node_id: "local".to_string(),
        }
    }
}

/// A single ledger observation from one node.
#[derive(Clone, Debug)]
pub struct LedgerObservation {
    /// Source identifier (local / anchor URL).
    pub source: String,
    /// Ledger sequence number.
    pub sequence: u64,
    /// Hex-encoded ledger close hash.
    pub hash: String,
    /// Whether the fetch succeeded.
    pub ok: bool,
}

/// Aggregated comparison result for one poll cycle.
#[derive(Clone, Debug)]
pub struct ForkCheckResult {
    /// Local ledger sequence.
    pub local_sequence: u64,
    /// Local ledger hash.
    pub local_hash: String,
    /// Number of anchors that agree with the local hash (at the same sequence).
    pub agreeing_anchors: usize,
    /// Total anchors that responded.
    pub responding_anchors: usize,
    /// Whether a fork divergence was detected this cycle.
    pub divergence_detected: bool,
    /// Sync confidence: agreeing_anchors / responding_anchors (0.0–1.0).
    pub sync_confidence: f64,
}

/// Response from Stellar Core `/info` endpoint (subset).
#[derive(Debug, Deserialize)]
pub struct StellarCoreInfoResponse {
    pub info: StellarCoreInfo,
}

#[derive(Debug, Deserialize)]
pub struct StellarCoreInfo {
    pub ledger: StellarCoreLedger,
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct StellarCoreLedger {
    pub num: u64,
    pub hash: String,
}

/// Response from Horizon `/ledgers?order=desc&limit=1`.
#[derive(Debug, Deserialize)]
pub struct HorizonLedgersResponse {
    #[serde(rename = "_embedded")]
    pub embedded: HorizonEmbedded,
}

#[derive(Debug, Deserialize)]
pub struct HorizonEmbedded {
    pub records: Vec<HorizonLedger>,
}

#[derive(Debug, Deserialize)]
pub struct HorizonLedger {
    pub sequence: u64,
    pub hash: String,
}
