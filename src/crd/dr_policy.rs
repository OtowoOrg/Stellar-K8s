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
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Disaster Recovery Policy for Stellar infrastructure
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "DisasterRecoveryPolicy",
    namespaced
)]
#[kube(status = "DisasterRecoveryPolicyStatus")]
#[serde(rename_all = "camelCase")]
pub struct DisasterRecoveryPolicySpec {
    /// Recovery Time Objective (RTO) in seconds
    pub rto_seconds: u32,
    /// Recovery Point Objective (RPO) in seconds
    pub rpo_seconds: u32,
    /// Automated failover enabled for this policy
    #[serde(default = "default_true")]
    pub automated_failover: bool,
    /// Minimum health score required for failover (0-100)
    #[serde(default = "default_health_score")]
    pub min_health_score: u32,
    /// Notification channels for DR events
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notification_channels: Vec<String>,
    /// Continuous DR testing interval (e.g., "24h")
    #[serde(default = "default_test_interval")]
    pub testing_interval: String,
}

fn default_true() -> bool {
    true
}

fn default_health_score() -> u32 {
    80
}

fn default_test_interval() -> String {
    "24h".to_string()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DisasterRecoveryPolicyStatus {
    /// Whether current system state satisfies RTO/RPO requirements
    pub compliance_status: ComplianceStatus,
    /// Last RTO measurement in seconds
    pub last_rto_seconds: Option<u32>,
    /// Last RPO measurement in seconds
    pub last_rpo_seconds: Option<u32>,
    /// Last compliance check time
    pub last_check_time: Option<String>,
    /// Recent drill results summary
    pub recent_drills: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ComplianceStatus {
    #[default]
    Unknown,
    Compliant,
    NonCompliant,
}
