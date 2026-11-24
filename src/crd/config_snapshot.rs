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

//! Configuration snapshot CRD for versioned, content-addressed dataplane configs.
//!
//! Bundles all dataplane configuration (ConfigMaps, routing tables, peer lists, etc.)
//! into a single versioned artifact, content-addressed by Merkle root (SHA-256).
//! Agents fetch and verify the snapshot atomically before applying — eliminating
//! partial-apply failure modes.

use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Marker type for StellarConfigSnapshot CRD
#[derive(
    CustomResource,
    Clone,
    Debug,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[kube(
    group = "stellar.io",
    version = "v1alpha1",
    kind = "StellarConfigSnapshot",
    plural = "stellarconfigsnapshots",
    shortname = "scs",
    namespaced
)]
#[kube(status = "StellarConfigSnapshotStatus")]
pub struct StellarConfigSnapshotSpec {
    /// Label selector identifying nodes to snapshot. Empty = all nodes.
    pub target_node_selector: Option<BTreeMap<String, String>>,

    /// Complete bundled dataplane configuration (content-addressed by merkle_root).
    pub dataplane_config: serde_json::Value,

    /// SHA-256 hash of serialized dataplane_config (content address).
    pub merkle_root: String,

    /// Optional reference to prior snapshot for delta compression.
    pub delta_from: Option<DeltaReference>,

    /// ECDSA signature (base64) over merkle_root for verification before apply.
    pub signature: String,

    /// Timestamp when snapshot was generated.
    pub generation_time: DateTime<Utc>,

    /// Byte size of serialized dataplane_config.
    pub config_size: i64,

    /// Optional user-defined metadata (change reason, context, etc.).
    pub metadata_fields: Option<BTreeMap<String, String>>,
}

/// Reference to a prior snapshot for delta-compression.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DeltaReference {
    /// Name of prior StellarConfigSnapshot in same namespace
    pub name: String,
    /// Expected merkle_root of prior snapshot (verification)
    pub merkle_root: String,
    /// Timestamp of prior snapshot
    pub snapshot_time: DateTime<Utc>,
}

/// Status of a configuration snapshot
#[derive(Clone, Debug, Serialize, Deserialize, Default, JsonSchema)]
pub struct StellarConfigSnapshotStatus {
    /// Snapshot phase: Pending, Generated, Signed, Applied, Failed
    pub phase: String,

    /// Agents (nodes) that have successfully applied this snapshot
    pub applied_by: Vec<AgentApplyRecord>,

    /// Signature and content verification results
    pub verification_status: Option<VerificationStatus>,

    /// Latest spec generation observed by controller
    pub observed_generation: Option<i64>,

    /// Conditions describing snapshot lifecycle
    pub conditions: Vec<Condition>,
}

/// Record of an agent applying this snapshot
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct AgentApplyRecord {
    /// Name of the agent (typically pod name or node name)
    pub agent_name: String,
    /// Name of the node where agent runs
    pub node_name: Option<String>,
    /// Time when snapshot was applied
    pub applied_time: DateTime<Utc>,
    /// Success or Failed
    pub status: String,
}

/// Verification status of a snapshot
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct VerificationStatus {
    /// Whether signature validation passed
    pub signature_valid: bool,
    /// Whether merkle_root matches content
    pub merkle_root_verified: bool,
    /// Last time verification was performed
    pub last_verified_time: DateTime<Utc>,
}

/// Condition tracking snapshot state
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Condition {
    /// Type of condition (e.g., "Ready", "SignatureValid")
    pub type_: String,
    /// Status: True, False, Unknown
    pub status: String,
    /// Reason code
    pub reason: Option<String>,
    /// Human-readable message
    pub message: Option<String>,
    /// Last time condition changed
    pub last_transition_time: DateTime<Utc>,
    /// Generation when condition was observed
    pub observed_generation: Option<i64>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Configuration snapshot helper functions
// ─────────────────────────────────────────────────────────────────────────────

/// Compute SHA-256 Merkle root of a configuration object.
///
/// Returns hex-encoded SHA-256 digest of the canonical JSON representation
/// (sorted keys, compact output).
pub fn compute_merkle_root(config: &serde_json::Value) -> Result<String> {
    let canonical_json = serde_json::to_string(config)
        .map_err(|e| Error::ConfigError(format!("Failed to serialize config: {}", e)))?;

    let mut hasher = Sha256::new();
    hasher.update(canonical_json.as_bytes());
    let digest = hasher.finalize();

    Ok(format!("{:x}", digest))
}

/// Verify that a computed Merkle root matches the expected value.
pub fn verify_merkle_root(computed: &str, expected: &str) -> bool {
    computed.eq_ignore_ascii_case(expected)
}

/// Extract the delta between two configs (prior → current).
///
/// Returns a new JSON object containing only fields that differ.
/// Used to reduce bandwidth when snapshot_size > threshold.
pub fn compute_delta(prior: &serde_json::Value, current: &serde_json::Value) -> serde_json::Value {
    let mut delta = serde_json::json!({});

    match (prior, current) {
        (serde_json::Value::Object(prior_map), serde_json::Value::Object(current_map)) => {
            // Fields added or changed
            for (key, current_val) in current_map.iter() {
                if !prior_map.contains_key(key) || prior_map[key] != *current_val {
                    delta[key] = current_val.clone();
                }
            }
            // Note: Deleted fields can be indicated via a separate "deletions" array
        }
        _ => {
            // If types differ or not objects, replace entirely
            delta = current.clone();
        }
    }

    delta
}

/// Merge delta into a prior config to reconstruct the full current config.
///
/// Inverse of compute_delta(). Used by agents to assemble full config
/// from prior snapshot + delta.
pub fn apply_delta(
    prior: &serde_json::Value,
    delta: &serde_json::Value,
) -> Result<serde_json::Value> {
    let mut result = prior.clone();

    match (&mut result, delta) {
        (serde_json::Value::Object(result_map), serde_json::Value::Object(delta_map)) => {
            for (key, delta_val) in delta_map.iter() {
                result_map.insert(key.clone(), delta_val.clone());
            }
        }
        _ => {
            result = delta.clone();
        }
    }

    Ok(result)
}

/// Format snapshot size in human-readable form (bytes, KB, MB).
pub fn format_config_size(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let size = bytes as f64;
    if size >= GB {
        format!("{:.2} GB", size / GB)
    } else if size >= MB {
        format!("{:.2} MB", size / MB)
    } else if size >= KB {
        format!("{:.2} KB", size / KB)
    } else {
        format!("{} bytes", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_merkle_root_deterministic() {
        let config = serde_json::json!({
            "foo": "bar",
            "nested": { "key": "value" }
        });

        let root1 = compute_merkle_root(&config).expect("merkle root should compute");
        let root2 = compute_merkle_root(&config).expect("merkle root should compute");

        assert_eq!(root1, root2, "Merkle root must be deterministic");
        assert_eq!(root1.len(), 64, "SHA-256 hex digest should be 64 chars");
    }

    #[test]
    fn test_verify_merkle_root() {
        let root = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6";
        assert!(verify_merkle_root(root, root));
        assert!(verify_merkle_root(
            root.to_uppercase().as_str(),
            root
        ));
        assert!(!verify_merkle_root(root, "different"));
    }

    #[test]
    fn test_compute_delta() {
        let prior = serde_json::json!({
            "a": 1,
            "b": 2,
            "c": { "d": 4 }
        });

        let current = serde_json::json!({
            "a": 1,
            "b": 3,
            "c": { "d": 4 },
            "e": 5
        });

        let delta = compute_delta(&prior, &current);

        // b changed, e added
        assert_eq!(delta.get("a"), None, "Unchanged field should not appear");
        assert_eq!(delta["b"], 3, "Changed field should appear");
        assert_eq!(delta["e"], 5, "New field should appear");
    }

    #[test]
    fn test_apply_delta() {
        let prior = serde_json::json!({
            "a": 1,
            "b": 2
        });

        let delta = serde_json::json!({
            "b": 20,
            "c": 3
        });

        let result = apply_delta(&prior, &delta).expect("apply_delta should succeed");

        assert_eq!(result["a"], 1);
        assert_eq!(result["b"], 20);
        assert_eq!(result["c"], 3);
    }

    #[test]
    fn test_format_config_size() {
        assert_eq!(format_config_size(500), "500 bytes");
        assert_eq!(format_config_size(1024), "1.00 KB");
        assert_eq!(format_config_size(1024 * 1024), "1.00 MB");
        assert_eq!(format_config_size(2 * 1024 * 1024 + 512 * 1024), "2.50 MB");
    }
}
