use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Multi-cluster disaster recovery and failover configuration
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "MultiRegionConfig",
    namespaced
)]
#[kube(status = "MultiRegionStatus")]
#[serde(rename_all = "camelCase")]
pub struct MultiRegionSpec {
    /// List of clusters participating in the multi-region deployment
    pub clusters: Vec<ClusterConfig>,
    /// Primary cluster name (must match one in clusters list)
    pub primary_cluster: String,
    /// Failover policy (Manual, Automated)
    pub failover_policy: FailoverPolicy,
    /// Health check configuration for failover detection
    pub health_check: MultiRegionHealthCheck,
    /// Secret synchronization configuration
    #[serde(default)]
    pub secret_sync: SecretSyncConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClusterConfig {
    pub name: String,
    pub region: String,
    /// API endpoint of the remote cluster (if accessible)
    pub api_endpoint: String,
    /// Reference to a secret containing Kubeconfig for the remote cluster
    pub kubeconfig_secret_ref: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum FailoverPolicy {
    Manual,
    Automated,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MultiRegionHealthCheck {
    pub interval_secs: u32,
    pub failure_threshold: u32,
    pub timeout_secs: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretSyncConfig {
    pub enabled: bool,
    /// Namespaces to sync secrets from
    pub namespaces: Vec<String>,
    /// Label selector for secrets to sync
    pub label_selector: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MultiRegionStatus {
    pub current_primary: String,
    pub last_failover_time: Option<chrono::DateTime<chrono::Utc>>,
    pub cluster_health: std::collections::BTreeMap<String, ClusterHealthStatus>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum ClusterHealthStatus {
    Healthy,
    Degraded,
    Unreachable,
}
