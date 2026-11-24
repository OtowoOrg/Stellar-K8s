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
//! ChangeRequest CRD definitions for multi-party approval workflow

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Privileged operation classes that require multi-party approval
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum ChangeClass {
    /// Cluster-wide configuration changes
    ClusterConfig,
    /// Security policy modifications (PSS, NetworkPolicy, RBAC)
    SecurityPolicy,
    /// Namespace creation/deletion in restricted environments
    NamespaceManagement,
    /// Node version upgrades affecting quorum
    NodeUpgrade,
    /// Disaster recovery operations (failover, restore)
    DisasterRecovery,
    /// Certificate rotation affecting multiple nodes
    CertificateRotation,
    /// Custom privileged operation with user-defined approvers
    Custom(String),
}

impl std::fmt::Display for ChangeClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeClass::ClusterConfig => write!(f, "ClusterConfig"),
            ChangeClass::SecurityPolicy => write!(f, "SecurityPolicy"),
            ChangeClass::NamespaceManagement => write!(f, "NamespaceManagement"),
            ChangeClass::NodeUpgrade => write!(f, "NodeUpgrade"),
            ChangeClass::DisasterRecovery => write!(f, "DisasterRecovery"),
            ChangeClass::CertificateRotation => write!(f, "CertificateRotation"),
            ChangeClass::Custom(s) => write!(f, "Custom({})", s),
        }
    }
}

/// Specific privileged operation being requested
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrivilegedOperation {
    /// The class of change
    pub class: ChangeClass,
    /// Target resource (group/version/kind/name/namespace)
    pub target_ref: TargetReference,
    /// Human-readable description of the change
    pub description: String,
    /// JSON patch or manifest representing the desired change
    #[serde(default)]
    pub change_payload: serde_json::Value,
    /// Risk assessment level
    pub risk_level: RiskLevel,
}

/// Risk level of the privileged operation
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Reference to the target Kubernetes resource
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TargetReference {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// Approver identity and signature
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Approver {
    /// Unique identifier (user principal, service account, or group)
    pub identity: String,
    /// Human-readable name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether this approver is required (vs optional)
    #[serde(default)]
    pub required: bool,
}

/// Individual approval record
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    /// Approver identity
    pub approver: Approver,
    /// Approval decision
    pub decision: ApprovalDecision,
    /// Timestamp of the approval
    pub timestamp: DateTime<Utc>,
    /// Optional justification/comment
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
    /// Cryptographic signature of the approval (for non-repudiation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// Approval decision
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum ApprovalDecision {
    Approve,
    Reject,
}

/// Current approval state
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum ApprovalState {
    /// Waiting for approvals
    Pending,
    /// Sufficient approvals received
    Approved,
    /// Explicitly rejected by a required approver
    Rejected,
    /// Expired without reaching quorum
    Expired,
    /// Executed successfully
    Executed,
    /// Execution failed
    Failed,
}

/// Phase of the ChangeRequest lifecycle
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "PascalCase")]
pub enum ChangeRequestPhase {
    #[default]
    Pending,
    UnderReview,
    Approved,
    Rejected,
    Expired,
    Executing,
    Executed,
    Failed,
}

/// ChangeRequest specification
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestSpec {
    /// The privileged operation being requested
    pub operation: PrivilegedOperation,
    /// Required approvers for this change class
    pub required_approvers: Vec<Approver>,
    /// Optional additional approvers
    #[serde(default)]
    pub optional_approvers: Vec<Approver>,
    /// Minimum number of approvals required (N-of-M)
    pub min_approvals: u32,
    /// Time-to-live for the approval request
    #[serde(default = "default_ttl")]
    pub ttl: String,
    /// Whether to auto-execute on approval
    #[serde(default)]
    pub auto_execute: bool,
    /// Optional execution window (cron expression)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_window: Option<String>,
    /// Metadata for audit trail
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

fn default_ttl() -> String { "24h".to_string() }

/// ChangeRequest status
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestStatus {
    /// Current phase
    pub phase: ChangeRequestPhase,
    /// Current approval state
    pub approval_state: ApprovalState,
    /// All approvals received
    #[serde(default)]
    pub approvals: Vec<Approval>,
    /// Number of approvals received
    pub approvals_received: u32,
    /// Number of approvals required
    pub approvals_required: u32,
    /// Whether quorum has been reached
    pub quorum_reached: bool,
    /// Expiration timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Execution timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executed_at: Option<DateTime<Utc>>,
    /// Execution result
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_result: Option<String>,
    /// Conditions for Kubernetes status
    #[serde(default)]
    pub conditions: Vec<ChangeRequestCondition>,
    /// Last updated timestamp
    pub last_updated: DateTime<Utc>,
}

/// Condition for Kubernetes status subresource
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestCondition {
    pub type_: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_transition_time: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
}

/// ChangeRequest custom resource
#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "approval.stellar.org",
    version = "v1alpha1",
    kind = "ChangeRequest",
    plural = "changerequests",
    shortname = "cr",
    namespaced,
    status = "ChangeRequestStatus",
    derive = "PartialEq",
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Approval","type":"string","jsonPath":".status.approvalState"}"#,
    printcolumn = r#"{"name":"Approvals","type":"string","jsonPath":".status.approvalsReceived"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
pub struct ChangeRequestSpecWrap(pub ChangeRequestSpec);

/// Duration serialization helper
pub mod duration_serde {
    use chrono::Duration;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{}s", duration.num_seconds()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s.ends_with('s') {
            let secs = s.trim_end_matches('s').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::seconds(secs))
        } else if s.ends_with('m') {
            let mins = s.trim_end_matches('m').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::minutes(mins))
        } else if s.ends_with('h') {
            let hours = s.trim_end_matches('h').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::hours(hours))
        } else if s.ends_with('d') {
            let days = s.trim_end_matches('d').parse::<i64>().map_err(serde::de::Error::custom)?;
            Ok(Duration::days(days))
        } else {
            Err(serde::de::Error::custom("duration must end with s, m, h, or d"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_change_class_display() {
        assert_eq!(ChangeClass::ClusterConfig.to_string(), "ClusterConfig");
        assert_eq!(ChangeClass::Custom("test".to_string()).to_string(), "Custom(test)");
    }

    #[test]
    fn test_duration_serde() {
        let dur = Duration::hours(24);
        let serialized = serde_json::to_string(&dur).unwrap();
        assert!(serialized.contains("86400s") || serialized.contains("24h"));
    }
}