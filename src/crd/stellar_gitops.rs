//! StellarGitOpsConfig Custom Resource Definition
//!
//! The StellarGitOpsConfig CRD enables GitOps integration with ArgoCD and Flux CD
//! for declarative infrastructure management and progressive delivery.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::types::Condition;

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarGitOpsConfig",
    namespaced,
    status = "StellarGitOpsConfigStatus",
    shortname = "sgc",
    printcolumn = r#"{"name":"Provider","type":"string","jsonPath":".spec.provider"}"#,
    printcolumn = r#"{"name":"Enabled","type":"boolean","jsonPath":".spec.enabled"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarGitOpsConfigSpec {
    /// GitOps provider to use (ArgoCD or FluxCD)
    pub provider: GitOpsProvider,

    /// Enable or disable GitOps
    #[serde(default)]
    pub enabled: bool,

    /// Git repository configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_repository: Option<GitRepositoryConfig>,

    /// ArgoCD-specific configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argocd_config: Option<ArgoCDConfig>,

    /// Flux CD-specific configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fluxcd_config: Option<FluxCDConfig>,

    /// Progressive delivery configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progressive_delivery: Option<ProgressiveDeliveryConfig>,

    /// Auto-sync policy labels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum GitOpsProvider {
    ArgoCD,
    FluxCD,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GitRepositoryConfig {
    /// Git repository URL
    pub url: String,

    /// Git branch to track
    #[serde(default = "default_branch")]
    pub branch: String,

    /// Sync interval (e.g., "1m", "5m", "1h")
    #[serde(default = "default_sync_interval")]
    pub interval: String,

    /// Credentials secret reference
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_secret: Option<String>,
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_sync_interval() -> String {
    "1m".to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArgoCDConfig {
    /// ArgoCD project name
    #[serde(default = "default_project")]
    pub project: String,

    /// Sync policy (Manual or Auto)
    pub sync_policy: Option<ArgoCDSyncPolicy>,

    /// Health assessment enabled
    #[serde(default)]
    pub auto_health_assessment: bool,
}

fn default_project() -> String {
    "default".to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum ArgoCDSyncPolicy {
    Manual,
    Auto,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FluxCDConfig {
    /// Kustomization interval
    pub kustomize_interval: Option<String>,

    /// HelmRelease interval
    pub helm_interval: Option<String>,

    /// Suspend reconciliation
    #[serde(default)]
    pub suspend: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProgressiveDeliveryConfig {
    /// Enable Flagger canary deployments
    #[serde(default)]
    pub canary_enabled: bool,

    /// Canary weight increment
    #[serde(default = "default_canary_increment")]
    pub canary_weight_increment: i32,

    /// Canary interval between steps
    pub canary_interval: Option<String>,
}

fn default_canary_increment() -> i32 {
    10
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarGitOpsConfigStatus {
    /// Conditions of this resource
    #[serde(default)]
    pub conditions: Vec<Condition>,

    /// Last sync time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_time: Option<String>,

    /// Sync status (Synced, OutOfSync, or Unknown)
    pub sync_status: Option<SyncStatus>,

    /// Drift detected
    #[serde(default)]
    pub drift_detected: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum SyncStatus {
    Synced,
    OutOfSync,
    Unknown,
}

impl Default for StellarGitOpsConfigStatus {
    fn default() -> Self {
        Self {
            conditions: Vec::new(),
            last_sync_time: None,
            sync_status: Some(SyncStatus::Unknown),
            drift_detected: false,
        }
    }
}
