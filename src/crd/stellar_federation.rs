//! StellarFederation CRD for multi-region federation support

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Multi-region federation configuration
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarFederation",
    namespaced,
    status = "StellarFederationStatus",
    shortname = "sfed"
)]
#[serde(rename_all = "camelCase")]
pub struct StellarFederationSpec {
    /// Federated clusters configuration
    pub clusters: Vec<FederationCluster>,
    /// Health check interval in seconds
    #[serde(default = "default_health_check_interval")]
    pub health_check_interval_secs: u32,
    /// Failover detection threshold in seconds
    #[serde(default = "default_failover_threshold")]
    pub failover_detection_secs: u32,
    /// Cross-region replication configuration
    pub replication: ReplicationConfig,
    /// Traffic routing policy
    pub traffic_routing: TrafficRoutingPolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederationCluster {
    pub name: String,
    pub region: String,
    pub api_endpoint: String,
    pub kubeconfig_secret_ref: String,
    #[serde(default)]
    pub weight: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationConfig {
    pub enabled: bool,
    pub mode: ReplicationMode,
    /// PostgreSQL replication lag threshold in seconds
    pub replication_lag_threshold_secs: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ReplicationMode {
    Asynchronous,
    Synchronous,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrafficRoutingPolicy {
    pub strategy: RoutingStrategy,
    pub health_check_timeout_secs: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum RoutingStrategy {
    Geographic,
    RoundRobin,
    LeastConnections,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct StellarFederationStatus {
    pub phase: String,
    pub active_regions: Vec<String>,
    pub failed_regions: Vec<String>,
    pub last_sync_time: Option<String>,
    pub replication_lag_ms: u32,
}

fn default_health_check_interval() -> u32 {
    30
}

fn default_failover_threshold() -> u32 {
    30
}
