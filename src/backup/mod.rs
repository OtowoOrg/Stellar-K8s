use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashSet;

pub mod providers;
pub mod scheduler;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DecentralizedBackupConfig {
    /// Enable decentralized backups
    pub enabled: bool,
    /// Storage provider configuration
    pub provider: StorageProvider,
    /// Backup schedule in cron format (default: every 6 hours)
    #[serde(default = "default_schedule")]
    pub schedule: String,
    /// Maximum number of concurrent uploads
    #[serde(default = "default_concurrency")]
    pub max_concurrent_uploads: usize,
    /// Enable compression before upload
    #[serde(default = "default_compression")]
    pub compression_enabled: bool,
    /// Retention policy (optional)
    pub retention: Option<RetentionPolicy>,
}

fn default_schedule() -> String {
    "0 */6 * * *".to_string() // Every 6 hours
}

fn default_concurrency() -> usize {
    3
}

fn default_compression() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StorageProvider {
    Arweave {
        /// Arweave wallet JWK (secret reference)
        wallet_secret: String,
        /// Gateway URL (default: arweave.net)
        #[serde(default = "default_arweave_gateway")]
        gateway: String,
        /// Tags to add to transactions
        #[serde(default)]
        tags: Vec<(String, String)>,
    },
    IPFS {
        /// IPFS API endpoint
        api_url: String,
        /// Pinning service (optional)
        pinning_service: Option<PinningService>,
    },
    Filecoin {
        /// Lotus API endpoint
        lotus_api: String,
        /// Wallet address for storage deals
        wallet_address: String,
        /// Storage deal parameters
        deal_params: FilecoinDealParams,
    },
}

fn default_arweave_gateway() -> String {
    "https://arweave.net".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PinningService {
    pub service_type: PinningServiceType,
    pub api_key_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PinningServiceType {
    Pinata,
    Web3Storage,
    Infura,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FilecoinDealParams {
    /// Price per epoch in attoFIL
    pub price_per_epoch: String,
    /// Duration in epochs
    pub duration: u64,
    /// Verified deal
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPolicy {
    /// Keep backups for this many days (0 = forever)
    pub days: u32,
    /// Minimum number of backups to keep
    pub min_backups: u32,
}
