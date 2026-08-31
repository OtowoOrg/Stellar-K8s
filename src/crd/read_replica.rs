// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
// use kube::CustomResource; // Unused
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::ResourceRequirements;

/// Configuration for read-only replica pools
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
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

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub enum ReadReplicaStrategy {
    #[default]
    RoundRobin,
    FreshnessPreferred,
}

fn default_read_replicas() -> i32 {
    1
}
