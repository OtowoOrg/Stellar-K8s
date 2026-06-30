//! StellarDatabase CRD for advanced database management and optimization.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Custom Resource Definition for Stellar Database configurations
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarDatabase",
    namespaced,
    status = "StellarDatabaseStatus",
    shortname = "sdb"
)]
#[serde(rename_all = "camelCase")]
pub struct StellarDatabaseSpec {
    /// Number of HA PostgreSQL instances to run
    pub instances: i32,
    /// Storage class name for PostgreSQL persistent volumes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_class: Option<String>,
    /// Size of the PostgreSQL storage volumes (e.g. "100Gi")
    pub storage_size: String,
    /// Target PostgreSQL version (e.g. "16")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postgres_version: Option<String>,
    /// PgBouncer connection pooling configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_pooling: Option<ConnectionPoolingConfig>,
    /// Read replica auto-scaling configuration (1-10 replicas)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_replicas: Option<ReadReplicaConfig>,
    /// Failover and replication settings (e.g. Patroni or CNPG)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failover: Option<FailoverConfig>,
    /// Engine auto-tuning parameters based on workload profile
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_tuning: Option<AutoTuningConfig>,
    /// Slow query logging and optimization checks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_optimization: Option<QueryOptimizationConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionPoolingConfig {
    pub enabled: bool,
    /// PgBouncer pooling mode (session, transaction, statement)
    pub pool_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_client_connections: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_pool_size: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadReplicaConfig {
    pub enabled: bool,
    /// Minimum read replica nodes
    pub min_replicas: i32,
    /// Maximum read replica nodes (must be <= 10)
    pub max_replicas: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_cpu_utilization: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FailoverConfig {
    pub enabled: bool,
    /// Automatic failover manager (e.g. "patroni" or "cnpg")
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_timeout_seconds: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutoTuningConfig {
    pub enabled: bool,
    /// Workload profile type (oltp, batch, mixed)
    pub workload_profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_buffers: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_cache_size: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueryOptimizationConfig {
    pub enabled: bool,
    /// Threshold (ms) for identifying slow queries
    pub slow_query_threshold_ms: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert_channels: Option<Vec<String>>,
    /// Suggest/create indexes automatically
    pub auto_index_recommendations: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum DatabasePhase {
    #[default]
    Pending,
    Configuring,
    Scaling,
    Running,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct StellarDatabaseStatus {
    /// The current operational lifecycle phase
    pub phase: DatabasePhase,
    /// Active PG connections observed
    pub active_connections: i32,
    /// PgBouncer pooler connection efficiency percentage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_efficiency_percent: Option<i32>,
    /// Speedup factor from custom query indexing suggestions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_performance_improvement_factor: Option<f32>,
    /// Number of running read replicas
    pub read_replica_count: i32,
    /// Detailed condition tracking for Kubernetes status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<crate::crd::Condition>>,
}
