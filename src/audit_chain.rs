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

//! # Immutable Audit Event Pipeline with Tamper-Evident Chain Hashing (Epic #1479)
//!
//! Provides an append-only, cryptographically linked audit event log.
//! Each record includes the SHA-256 hash of its predecessor, creating a
//! tamper-evident blockchain-style hash chain. Any retroactive modification,
//! deletion, or reordering breaks the chain. Periodic signed Merkle checkpoints
//! provide anchoring against truncation attacks.
//!
//! ## Architecture
//!
//! ```text
//! AuditEvent (Admin / Webhook / Controller / Quota)
//!    │
//!    ▼
//! AuditPipeline (Non-blocking async ingestion)
//!    │
//!    ▼
//! AuditChain (Append-only storage)
//!    ├── Record #0: Genesis (H_0 = SHA256("GENESIS" || cluster_id))
//!    ├── Record #1: H_1 = SHA256(H_0 || Seq_1 || Time_1 || Event_1)
//!    ├── Record #2: H_2 = SHA256(H_1 || Seq_2 || Time_2 || Event_2)
//!    └── ...
//!    │
//!    ├──> Periodic Checkpointing (Merkle Root + Signature)
//!    └──> ChainVerificationEngine (Detects modified, deleted, or inserted records)
//! ```

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};

/// Domain separation string for the audit chain genesis block.
pub const GENESIS_DOMAIN_PREFIX: &str = "STELLAR_K8S_AUDIT_GENESIS_v1:";

/// Type of audit event recorded in the tamper-evident chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    AdmissionDecision,
    ConfigChange,
    SecretRotation,
    QuotaEnforcement,
    AutoscalingAction,
    TenantAction,
    NodeLifecycle,
    SecurityScan,
    PolicyEvaluation,
    OperatorStartup,
    /// Feature-flag evaluation decision affecting user-visible behavior (Epic #1984).
    FeatureFlagEvaluation,
    Custom(String),
}

/// Severity classification of an audit event.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum AuditSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Actor who initiated the action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuditActor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

impl AuditActor {
    pub fn system(component: &str) -> Self {
        Self {
            name: format!("system:{component}"),
            namespace: Some("stellar-system".to_string()),
            client_ip: None,
            user_agent: Some("stellar-operator".to_string()),
        }
    }
}

/// Target resource affected by the event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TargetResource {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

/// Outcome of the audited action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AuditOutcome {
    Success,
    Allowed,
    Denied,
    Failed,
}

/// Structured audit event payload before chain framing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub event_id: String,
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub severity: AuditSeverity,
    pub actor: AuditActor,
    pub target: TargetResource,
    pub outcome: AuditOutcome,
    pub action: String,
    #[serde(default)]
    pub details: Value,
}

/// A cryptographically chained audit record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    /// Monotonically increasing sequence index (0, 1, 2, ...).
    pub sequence: u64,
    /// Record creation timestamp.
    pub timestamp: DateTime<Utc>,
    /// Underlying audit event.
    pub event: AuditEvent,
    /// Hex-encoded SHA-256 hash of the preceding record (H_{i-1}).
    pub prev_hash: String,
    /// Hex-encoded SHA-256 hash of this record (H_i).
    pub record_hash: String,
    /// Cryptographic signature over record_hash (HMAC or digital signature).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// Signed checkpoint summarizing a batch of records with a Merkle root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignedCheckpoint {
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub count: u64,
    pub merkle_root: String,
    pub head_hash: String,
    pub created_at: DateTime<Utc>,
    pub signature: String,
}

/// Detailed verification result for an audit chain segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainVerificationSummary {
    pub valid: bool,
    pub total_records: usize,
    pub genesis_hash: String,
    pub head_hash: String,
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub message: String,
}

/// Tamper detection error identifying where and why a chain broke.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditTamperError {
    pub sequence: u64,
    pub error_type: TamperType,
    pub expected_hash: String,
    pub actual_hash: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TamperType {
    BrokenHashLink,
    PayloadMutation,
    SequenceMismatch,
    NonMonotonicTimestamp,
    GenesisMismatch,
    InvalidSignature,
}

impl std::fmt::Display for AuditTamperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Audit tamper detected at seq {}: {:?} - {}",
            self.sequence, self.error_type, self.message
        )
    }
}

/// Core append-only audit chain with cryptographic linking.
pub struct AuditChain {
    cluster_id: String,
    genesis_hash: String,
    records: Vec<AuditRecord>,
    signing_secret: Option<String>,
}

impl AuditChain {
    /// Initialize a new audit chain for a given cluster ID.
    pub fn new(cluster_id: String, signing_secret: Option<String>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(GENESIS_DOMAIN_PREFIX.as_bytes());
        hasher.update(cluster_id.as_bytes());
        let genesis_hash = hex::encode(hasher.finalize());

        Self {
            cluster_id,
            genesis_hash,
            records: Vec::new(),
            signing_secret,
        }
    }

    /// Return the genesis hash for this chain.
    pub fn genesis_hash(&self) -> &str {
        &self.genesis_hash
    }

    /// Return the current head hash (or genesis if empty).
    pub fn head_hash(&self) -> String {
        self.records
            .last()
            .map(|r| r.record_hash.clone())
            .unwrap_or_else(|| self.genesis_hash.clone())
    }

    /// Append a new audit event to the chain, computing its cryptographic hash pointer.
    pub fn append(&mut self, event: AuditEvent) -> AuditRecord {
        let sequence = self.records.len() as u64;
        let prev_hash = self.head_hash();
        let timestamp = Utc::now();

        let record_hash = compute_record_hash(&prev_hash, sequence, timestamp, &event);
        let signature = self
            .signing_secret
            .as_ref()
            .map(|key| compute_hmac_signature(&record_hash, key));

        let record = AuditRecord {
            sequence,
            timestamp,
            event,
            prev_hash,
            record_hash,
            signature,
        };

        self.records.push(record.clone());
        record
    }

    /// Get all records currently in the chain.
    pub fn records(&self) -> &[AuditRecord] {
        &self.records
    }

    /// Number of records in the chain.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Query records with optional filters.
    pub fn query(
        &self,
        start_seq: Option<u64>,
        limit: usize,
        event_type: Option<AuditEventType>,
        tenant_id: Option<&str>,
    ) -> Vec<AuditRecord> {
        let start = start_seq.unwrap_or(0) as usize;
        self.records
            .iter()
            .skip(start)
            .filter(|r| {
                if let Some(ref et) = event_type {
                    if &r.event.event_type != et {
                        return false;
                    }
                }
                if let Some(tid) = tenant_id {
                    if r.event.target.tenant_id.as_deref() != Some(tid) {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .cloned()
            .collect()
    }

    /// Create a signed Merkle checkpoint over the current chain state.
    pub fn create_checkpoint(&self) -> Option<SignedCheckpoint> {
        if self.records.is_empty() {
            return None;
        }

        let start_sequence = 0;
        let end_sequence = (self.records.len() - 1) as u64;
        let count = self.records.len() as u64;
        let head_hash = self.head_hash();

        let leaf_hashes: Vec<String> = self.records.iter().map(|r| r.record_hash.clone()).collect();
        let merkle_root = compute_merkle_root(&leaf_hashes);

        let created_at = Utc::now();
        let signing_material = format!("{start_sequence}:{end_sequence}:{merkle_root}:{head_hash}");
        let signature = self
            .signing_secret
            .as_ref()
            .map(|key| compute_hmac_signature(&signing_material, key))
            .unwrap_or_else(|| "unsigned".to_string());

        Some(SignedCheckpoint {
            start_sequence,
            end_sequence,
            count,
            merkle_root,
            head_hash,
            created_at,
            signature,
        })
    }
}

/// Compute the deterministic SHA-256 hash for an audit record.
pub fn compute_record_hash(
    prev_hash: &str,
    sequence: u64,
    timestamp: DateTime<Utc>,
    event: &AuditEvent,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(sequence.to_be_bytes());
    hasher.update(timestamp.to_rfc3339().as_bytes());

    let event_json = serde_json::to_string(event).unwrap_or_default();
    hasher.update(event_json.as_bytes());

    hex::encode(hasher.finalize())
}

/// Compute HMAC-SHA256 signature for record or checkpoint integrity.
fn compute_hmac_signature(data: &str, secret_key: &str) -> String {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<Sha256>;

    let mut mac =
        HmacSha256::new_from_slice(secret_key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(data.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Compute a Merkle tree root from a list of leaf hashes.
pub fn compute_merkle_root(leaves: &[String]) -> String {
    if leaves.is_empty() {
        return "0000000000000000000000000000000000000000000000000000000000000000".to_string();
    }
    if leaves.len() == 1 {
        return leaves[0].clone();
    }

    let mut current_level: Vec<Vec<u8>> = leaves
        .iter()
        .map(|h| hex::decode(h).unwrap_or_default())
        .collect();

    while current_level.len() > 1 {
        let mut next_level = Vec::new();
        for chunk in current_level.chunks(2) {
            let mut hasher = Sha256::new();
            hasher.update(&chunk[0]);
            if chunk.len() > 1 {
                hasher.update(&chunk[1]);
            } else {
                hasher.update(&chunk[0]); // Duplicate odd leaf
            }
            next_level.push(hasher.finalize().to_vec());
        }
        current_level = next_level;
    }

    hex::encode(&current_level[0])
}

/// Verifies an audit record slice from start to finish.
/// Detects payload modifications, broken hash links, dropped records, and timestamp regressions.
pub fn verify_audit_chain(
    records: &[AuditRecord],
    expected_genesis: &str,
) -> std::result::Result<ChainVerificationSummary, AuditTamperError> {
    if records.is_empty() {
        return Ok(ChainVerificationSummary {
            valid: true,
            total_records: 0,
            genesis_hash: expected_genesis.to_string(),
            head_hash: expected_genesis.to_string(),
            start_sequence: 0,
            end_sequence: 0,
            message: "Empty chain is valid".to_string(),
        });
    }

    let mut prev_hash = expected_genesis.to_string();
    let mut prev_timestamp = records[0].timestamp - chrono::Duration::seconds(1);

    for (idx, record) in records.iter().enumerate() {
        let expected_seq = idx as u64;

        // 1. Sequence continuity
        if record.sequence != expected_seq {
            return Err(AuditTamperError {
                sequence: record.sequence,
                error_type: TamperType::SequenceMismatch,
                expected_hash: format!("seq:{expected_seq}"),
                actual_hash: format!("seq:{}", record.sequence),
                message: format!(
                    "Sequence jump detected: expected {}, found {}",
                    expected_seq, record.sequence
                ),
            });
        }

        // 2. Previous hash link check
        if record.prev_hash != prev_hash {
            return Err(AuditTamperError {
                sequence: record.sequence,
                error_type: if idx == 0 {
                    TamperType::GenesisMismatch
                } else {
                    TamperType::BrokenHashLink
                },
                expected_hash: prev_hash.clone(),
                actual_hash: record.prev_hash.clone(),
                message: format!(
                    "Broken hash link: expected prev_hash {}, found {}",
                    prev_hash, record.prev_hash
                ),
            });
        }

        // 3. Timestamp sanity check (must not move backwards in time)
        if record.timestamp < prev_timestamp {
            return Err(AuditTamperError {
                sequence: record.sequence,
                error_type: TamperType::NonMonotonicTimestamp,
                expected_hash: prev_timestamp.to_rfc3339(),
                actual_hash: record.timestamp.to_rfc3339(),
                message: format!(
                    "Timestamp retroactivity detected: record time {} is before previous {}",
                    record.timestamp, prev_timestamp
                ),
            });
        }

        // 4. Cryptographic integrity check: recompute hash
        let computed_hash = compute_record_hash(
            &record.prev_hash,
            record.sequence,
            record.timestamp,
            &record.event,
        );

        if computed_hash != record.record_hash {
            return Err(AuditTamperError {
                sequence: record.sequence,
                error_type: TamperType::PayloadMutation,
                expected_hash: computed_hash,
                actual_hash: record.record_hash.clone(),
                message:
                    "Payload mutation detected: record hash does not match computed event content"
                        .to_string(),
            });
        }

        prev_hash = record.record_hash.clone();
        prev_timestamp = record.timestamp;
    }

    let start_sequence = records.first().unwrap().sequence;
    let end_sequence = records.last().unwrap().sequence;
    let head_hash = records.last().unwrap().record_hash.clone();

    Ok(ChainVerificationSummary {
        valid: true,
        total_records: records.len(),
        genesis_hash: expected_genesis.to_string(),
        head_hash,
        start_sequence,
        end_sequence,
        message: format!(
            "Successfully verified {} audit records with intact hash chain",
            records.len()
        ),
    })
}

/// Thread-safe audit event pipeline providing asynchronous non-blocking submission.
#[derive(Clone)]
pub struct AuditPipeline {
    chain: Arc<RwLock<AuditChain>>,
}

impl AuditPipeline {
    pub fn new(cluster_id: String, signing_secret: Option<String>) -> Self {
        Self {
            chain: Arc::new(RwLock::new(AuditChain::new(cluster_id, signing_secret))),
        }
    }

    /// Asynchronously record an event into the immutable chain.
    pub async fn record(
        &self,
        event_type: AuditEventType,
        severity: AuditSeverity,
        actor: AuditActor,
        target: TargetResource,
        action: &str,
        outcome: AuditOutcome,
        details: Value,
    ) -> AuditRecord {
        let event = AuditEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            event_type,
            severity,
            actor,
            target,
            outcome,
            action: action.to_string(),
            details,
        };

        let mut chain = self.chain.write().await;
        let record = chain.append(event);
        debug!(
            seq = record.sequence,
            hash = %record.record_hash,
            "Recorded immutable audit event"
        );
        record
    }

    /// Query the chain.
    pub async fn query(
        &self,
        start_seq: Option<u64>,
        limit: usize,
        event_type: Option<AuditEventType>,
        tenant_id: Option<&str>,
    ) -> Vec<AuditRecord> {
        let chain = self.chain.read().await;
        chain.query(start_seq, limit, event_type, tenant_id)
    }

    /// Verify the current chain.
    pub async fn verify(&self) -> std::result::Result<ChainVerificationSummary, AuditTamperError> {
        let chain = self.chain.read().await;
        verify_audit_chain(chain.records(), chain.genesis_hash())
    }

    /// Create signed checkpoint.
    pub async fn checkpoint(&self) -> Option<SignedCheckpoint> {
        let chain = self.chain.read().await;
        chain.create_checkpoint()
    }

    /// Record a feature-flag evaluation decision that affects user-visible behavior.
    ///
    /// Every evaluation routed through this method becomes part of the
    /// tamper-evident chain, satisfying the "every user-affecting decision
    /// auditable" acceptance criterion of Epic #1984.
    pub async fn record_flag_evaluation(
        &self,
        flag_key: &str,
        variant: &str,
        reason: &str,
        targeting_key: Option<&str>,
        details: Value,
    ) -> AuditRecord {
        let mut merged = details;
        if let Value::Object(ref mut map) = merged {
            map.insert("flagKey".to_string(), Value::String(flag_key.to_string()));
            map.insert("variant".to_string(), Value::String(variant.to_string()));
            map.insert("reason".to_string(), Value::String(reason.to_string()));
            if let Some(tk) = targeting_key {
                map.insert("targetingKey".to_string(), Value::String(tk.to_string()));
            }
        } else {
            merged = serde_json::json!({
                "flagKey": flag_key,
                "variant": variant,
                "reason": reason,
                "targetingKey": targeting_key,
                "details": merged,
            });
        }

        self.record(
            AuditEventType::FeatureFlagEvaluation,
            AuditSeverity::Info,
            AuditActor::system("feature-flags"),
            TargetResource {
                kind: "FeatureFlag".to_string(),
                name: flag_key.to_string(),
                namespace: None,
                tenant_id: None,
            },
            "evaluate",
            AuditOutcome::Success,
            merged,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event(name: &str, action: &str) -> AuditEvent {
        AuditEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            event_type: AuditEventType::AdmissionDecision,
            severity: AuditSeverity::Info,
            actor: AuditActor::system("webhook"),
            target: TargetResource {
                kind: "StellarNode".to_string(),
                name: name.to_string(),
                namespace: Some("default".to_string()),
                tenant_id: Some("tenant-1".to_string()),
            },
            outcome: AuditOutcome::Allowed,
            action: action.to_string(),
            details: serde_json::json!({ "image": "stellar-core:v21.0.0" }),
        }
    }

    #[test]
    fn test_audit_chain_append_and_verify() {
        let mut chain = AuditChain::new(
            "cluster-us-east-1".to_string(),
            Some("secret-key".to_string()),
        );
        let e1 = sample_event("node-1", "create");
        let e2 = sample_event("node-2", "scale");
        let e3 = sample_event("node-1", "delete");

        let r1 = chain.append(e1);
        let r2 = chain.append(e2);
        let r3 = chain.append(e3);

        assert_eq!(r1.sequence, 0);
        assert_eq!(r2.sequence, 1);
        assert_eq!(r3.sequence, 2);

        assert_eq!(r2.prev_hash, r1.record_hash);
        assert_eq!(r3.prev_hash, r2.record_hash);

        let verification = verify_audit_chain(chain.records(), chain.genesis_hash());
        assert!(verification.is_ok());
        let summary = verification.unwrap();
        assert_eq!(summary.total_records, 3);
        assert!(summary.valid);
    }

    #[test]
    fn test_detect_payload_tampering() {
        let mut chain = AuditChain::new("cluster-test".to_string(), None);
        chain.append(sample_event("node-1", "create"));
        chain.append(sample_event("node-2", "update"));
        chain.append(sample_event("node-3", "delete"));

        let mut tampered_records = chain.records().to_vec();
        // Maliciously modify the payload of record #1
        tampered_records[1].event.action = "malicious_unauthorized_action".to_string();

        let result = verify_audit_chain(&tampered_records, chain.genesis_hash());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.sequence, 1);
        assert_eq!(err.error_type, TamperType::PayloadMutation);
    }

    #[test]
    fn test_detect_dropped_record_omission() {
        let mut chain = AuditChain::new("cluster-test".to_string(), None);
        chain.append(sample_event("node-1", "create"));
        chain.append(sample_event("node-2", "update"));
        chain.append(sample_event("node-3", "delete"));

        let mut tampered_records = chain.records().to_vec();
        // Maliciously delete record #1 from history
        tampered_records.remove(1);

        let result = verify_audit_chain(&tampered_records, chain.genesis_hash());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error_type, TamperType::SequenceMismatch);
    }

    #[tokio::test]
    async fn test_flag_evaluation_is_audited() {
        let pipeline = AuditPipeline::new("cluster-test".to_string(), Some("key".to_string()));

        pipeline
            .record_flag_evaluation(
                "checkout.new-flow",
                "on",
                "targeting_match",
                Some("user-42"),
                serde_json::json!({ "bundleVersion": 7 }),
            )
            .await;

        let records = pipeline
            .query(None, 10, Some(AuditEventType::FeatureFlagEvaluation), None)
            .await;
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.event.target.kind, "FeatureFlag");
        assert_eq!(record.event.target.name, "checkout.new-flow");
        assert_eq!(record.event.details["variant"], "on");
        assert_eq!(record.event.details["targetingKey"], "user-42");
        assert_eq!(record.event.details["bundleVersion"], 7);

        let summary = pipeline.verify().await.expect("chain must verify");
        assert_eq!(summary.total_records, 1);
    }

    #[test]
    fn test_merkle_checkpoint_generation() {
        let mut chain = AuditChain::new("cluster-test".to_string(), Some("key".to_string()));
        for i in 0..10 {
            chain.append(sample_event(&format!("node-{i}"), "tick"));
        }

        let checkpoint = chain.create_checkpoint().unwrap();
        assert_eq!(checkpoint.count, 10);
        assert_eq!(checkpoint.start_sequence, 0);
        assert_eq!(checkpoint.end_sequence, 9);
        assert!(!checkpoint.merkle_root.is_empty());
        assert_ne!(checkpoint.signature, "unsigned");
    }
}
