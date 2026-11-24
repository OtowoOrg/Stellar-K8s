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
//! MultiSigOperation Custom Resource Definition (#1578)
//!
//! Coordinates multi-signature administrative operations (settings upgrades, signer changes,
//! protocol upgrades) requiring M-of-N approvals from validator operators, sidecars, or KMS signers.

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Custom Resource Definition for M-of-N Multi-Signature Administrative Operations
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "MultiSigOperation",
    namespaced,
    status = "MultiSigOperationStatus",
    shortname = "msig",
    printcolumn = r#"{"name":"Operation","type":"string","jsonPath":".spec.operationType"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Quorum","type":"string","jsonPath":".status.quorumProgress"}"#,
    printcolumn = r#"{"name":"Signatures","type":"integer","jsonPath":".status.signaturesCollected"}"#,
    printcolumn = r#"{"name":"Threshold","type":"integer","jsonPath":".status.signaturesRequired"}"#,
    printcolumn = r#"{"name":"ExpiresAt","type":"date","jsonPath":".status.expiresAt"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct MultiSigOperationSpec {
    /// Type of administrative operation being proposed.
    pub operation_type: AdminOperationType,

    /// Human-readable description / RFC reference for this proposal.
    pub description: String,

    /// Target network ("Public", "Testnet", "Futurenet", "Standalone").
    #[serde(default = "default_network")]
    pub target_network: String,

    /// Stellar transaction envelope XDR payload (base64 or hex encoded).
    pub transaction_xdr_payload: String,

    /// SHA-256 hash of the Stellar transaction payload.
    pub transaction_hash: String,

    /// Required threshold M (number of valid signatures required before submission).
    pub threshold_m: u32,

    /// Total eligible signers N.
    pub total_signers_n: u32,

    /// List of authorized signer pods, sidecars, or secret references.
    pub signers: Vec<SignerEndpointSpec>,

    /// Timeout in seconds before an incomplete operation expires (default: 7200 = 2 hours).
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,

    /// Automatically submit transaction to network once quorum M is satisfied.
    #[serde(default = "default_true")]
    pub auto_submit_on_quorum: bool,

    /// Horizon or Stellar Core submission endpoint URL.
    #[serde(default)]
    pub submission_endpoint: Option<String>,
}

fn default_network() -> String {
    "Testnet".to_string()
}

fn default_timeout() -> u64 {
    7200
}

fn default_true() -> bool {
    true
}

/// Administrative operation categories
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum AdminOperationType {
    SettingsUpgrade,
    SignerUpdate,
    ProtocolUpgrade,
    FeePoolChange,
    AnchorTrustline,
    Custom(String),
}

/// Signer endpoint specification
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignerEndpointSpec {
    /// Identifier of the signer (operator, organization, or validator name).
    pub signer_id: String,

    /// Stellar public key (G...) or ed25519 public key.
    pub public_key: String,

    /// URL of the signer sidecar service (e.g., http://validator-signer.stellar:8080).
    #[serde(default)]
    pub sidecar_endpoint: Option<String>,

    /// Secret reference containing private key or token if locally managed.
    #[serde(default)]
    pub secret_ref: Option<String>,

    /// Weight of this signer (default: 1).
    #[serde(default = "default_weight")]
    pub weight: u32,
}

fn default_weight() -> u32 {
    1
}

/// Lifecycle phase of multi-signature operation
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "PascalCase")]
pub enum MultiSigPhase {
    #[default]
    Pending,
    Collecting,
    QuorumReached,
    Submitting,
    Submitted,
    Expired,
    Failed,
}

/// Observed status of multi-sig coordination
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MultiSigOperationStatus {
    /// Current lifecycle phase.
    pub phase: MultiSigPhase,

    /// Quorum progress summary (e.g. "2/3").
    #[serde(default)]
    pub quorum_progress: String,

    /// Total valid signatures collected so far.
    pub signatures_collected: u32,

    /// Target threshold M required.
    pub signatures_required: u32,

    /// Whether quorum M has been achieved.
    pub quorum_reached: bool,

    /// Collected cryptographic signatures.
    #[serde(default)]
    pub collected_signatures: Vec<CollectedSignature>,

    /// Signer IDs that have not yet signed.
    #[serde(default)]
    pub missing_signers: Vec<String>,

    /// Timestamp when operation expires if quorum is not reached.
    pub expires_at: DateTime<Utc>,

    /// Transaction submission outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submission_result: Option<SubmissionResult>,

    /// Immutable audit trail of signer approvals and state transitions.
    #[serde(default)]
    pub audit_trail: Vec<MultiSigAuditEntry>,

    /// Kubernetes conditions.
    #[serde(default)]
    pub conditions: Vec<MultiSigCondition>,
}

/// Record of an individual collected signature
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectedSignature {
    pub signer_id: String,
    pub public_key: String,
    pub signature: String,
    pub signed_at: DateTime<Utc>,
    pub source: String,
}

/// Transaction submission receipt
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionResult {
    pub submitted_at: DateTime<Utc>,
    pub transaction_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_sequence: Option<u64>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Audit trail event
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MultiSigAuditEntry {
    pub timestamp: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub details: String,
}

/// Condition
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MultiSigCondition {
    pub type_: String,
    pub status: String,
    pub reason: String,
    pub message: String,
    pub last_transition_time: Option<DateTime<Utc>>,
}
