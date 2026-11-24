// Copyright 2026 Stellar-K8s Contributors
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
//! Signed CEL policy-bundle Custom Resource for admission-time evaluation.

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::Condition;
use crate::crd::secret_policy::KmsProvider;

/// Cluster-scoped signed policy bundle evaluated at admission time.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarPolicyBundle",
    namespaced,
    status = "StellarPolicyBundleStatus",
    shortname = "spb",
    printcolumn = r#"{"name":"Version","type":"string","jsonPath":".spec.version"}"#,
    printcolumn = r#"{"name":"Verified","type":"boolean","jsonPath":".status.verified"}"#,
    printcolumn = r#"{"name":"Hash","type":"string","jsonPath":".status.activeHash"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarPolicyBundleSpec {
    /// Operator-independent bundle version (semver or monotonic id).
    pub version: String,
    /// CEL expressions; every expression must evaluate to true to admit.
    pub policies: Vec<CelPolicySpec>,
    /// Inclusive validity start (RFC3339).
    pub not_before: DateTime<Utc>,
    /// Exclusive validity end (RFC3339).
    pub not_after: DateTime<Utc>,
    /// Unique nonce used to reject replayed bundles.
    pub nonce: String,
    /// Trust-root key identifier (matches a configured verifying key).
    pub key_id: String,
    /// Signature algorithm. Only `ed25519` is accepted.
    #[serde(default = "default_algorithm")]
    pub algorithm: String,
    /// Base64-encoded Ed25519 signature over the canonical unsigned payload.
    #[serde(default)]
    pub signature: String,
    /// Optional SecretPolicy / KMS reference used to resolve the trust root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_root_ref: Option<PolicyTrustRootRef>,
    /// Optional rollback target (previous verified bundle version/hash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_target: Option<String>,
}

fn default_algorithm() -> String {
    "ed25519".to_string()
}

/// One CEL policy inside a signed bundle.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CelPolicySpec {
    /// Stable policy name used in deny messages and metrics.
    pub name: String,
    /// CEL expression evaluated against the admission request.
    pub expression: String,
}

/// How the verifying key is obtained from existing key-management infra.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyTrustRootRef {
    /// Secret name holding the Ed25519 public key (base64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_name: Option<String>,
    /// Key inside the Secret (default: `publicKey`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_key: Option<String>,
    /// Optional KMS provider that issued/wraps the signing key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_provider: Option<KmsProvider>,
    /// Provider-specific key identifier recorded for audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
}

/// Observed verification and activation state.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarPolicyBundleStatus {
    /// Whether the current spec verified against the trust root.
    #[serde(default)]
    pub verified: bool,
    /// SHA-256 hex digest of the active canonical payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_hash: Option<String>,
    /// Previously active hash (used for rollback).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_hash: Option<String>,
    /// Active bundle version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_version: Option<String>,
    /// Last verification or load error (fail-closed reason).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Standard conditions.
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

/// Dual-approval annotations reused from the existing ChangeRequest workflow.
pub mod annotations {
    /// Request emergency bypass of signed policy evaluation.
    pub const EMERGENCY_OVERRIDE: &str = "policy.stellar.org/emergency-override";
    /// First required approver identity (ChangeRequest-compatible).
    pub const APPROVER_1: &str = "approval.stellar.org/approver-1";
    /// Second required approver identity (ChangeRequest-compatible).
    pub const APPROVER_2: &str = "approval.stellar.org/approver-2";
    /// Comma-separated approver list (alternative to the two discrete keys).
    pub const APPROVERS: &str = "approval.stellar.org/approvers";
}
