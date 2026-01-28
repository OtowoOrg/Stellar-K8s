// use kube::CustomResource; // Unused
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::ResourceRequirements;

/// Configuration for read-only replica pools
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadReplicaConfig {
    /// Number of read-only replicas
    #[serde(default = "default_read_replicas")]
    pub replicas: i32,

    /// Compute resource requirements for read replicas
    #[serde(default)]
    pub resources: ResourceRequirements,

    /// Load balancing strategy
    #[serde(default)]
    pub strategy: ReadReplicaStrategy,

    /// Enable history archive sharding
    /// When true, replicas serve different archives to balance bandwidth
    #[serde(default)]
    pub archive_sharding: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
pub enum ReadReplicaStrategy {
    RoundRobin,
    FreshnessPreferred,
}

impl Default for ReadReplicaStrategy {
    fn default() -> Self {
        ReadReplicaStrategy::RoundRobin
    }
}

fn default_read_replicas() -> i32 {
    1
}
