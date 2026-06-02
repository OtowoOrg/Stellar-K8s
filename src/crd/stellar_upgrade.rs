//! StellarUpgrade CRD for zero-downtime canary deployments

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Zero-downtime upgrade with canary deployments
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarUpgrade",
    namespaced,
    status = "StellarUpgradeStatus",
    shortname = "sup"
)]
#[serde(rename_all = "camelCase")]
pub struct StellarUpgradeSpec {
    /// Target StellarNode name
    pub target_node: String,
    /// New version to upgrade to
    pub target_version: String,
    /// Canary deployment strategy
    pub canary_strategy: CanaryStrategy,
    /// Health validation rules
    pub health_validation: HealthValidation,
    /// Rollback policy
    pub rollback_policy: RollbackPolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanaryStrategy {
    pub initial_traffic_percent: u32,
    pub traffic_increment_percent: u32,
    pub increment_interval_secs: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthValidation {
    pub consensus_healthy: bool,
    pub sync_state_healthy: bool,
    pub api_responding: bool,
    pub error_rate_threshold_percent: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RollbackPolicy {
    pub automatic_rollback: bool,
    pub rollback_on_error_rate: bool,
    pub error_rate_threshold_percent: f64,
    pub rollback_timeout_secs: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct StellarUpgradeStatus {
    pub phase: UpgradePhase,
    pub canary_ready: bool,
    pub traffic_percent: u32,
    pub health_checks_passed: u32,
    pub health_checks_failed: u32,
    pub last_update_time: Option<String>,
    pub rollback_in_progress: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum UpgradePhase {
    Pending,
    CanaryDeployed,
    ProgressivRollout,
    Completed,
    RolledBack,
    Failed,
}

impl Default for UpgradePhase {
    fn default() -> Self {
        UpgradePhase::Pending
    }
}
