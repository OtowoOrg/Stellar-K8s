use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Watch list for monitoring issued Stellar assets.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarAssetMonitor",
    namespaced
)]
#[serde(rename_all = "camelCase")]
pub struct StellarAssetMonitorSpec {
    /// Whether monitoring is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Network name used to label exported metrics.
    pub network: String,

    /// Watched classic assets and Soroban Asset Contracts.
    #[serde(default)]
    pub watch_list: Vec<AssetWatch>,

    /// Absolute supply change percentage that sets the large-change alert gauge.
    #[serde(default = "default_supply_change_threshold")]
    pub large_supply_change_percent: f64,
}

fn default_enabled() -> bool {
    true
}

fn default_supply_change_threshold() -> f64 {
    10.0
}

/// Asset identifiers used to match TrustLine and ContractData ledger changes.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssetWatch {
    /// Stellar asset code.
    pub asset_code: String,

    /// Issuer account for a classic asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,

    /// SAC contract ID for Soroban asset events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_id: Option<String>,
}
