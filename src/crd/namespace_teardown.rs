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
//! Namespace teardown CRD (#1499).
//!
//! Models a secure namespace teardown as an explicit state machine so that a
//! stuck step is visible as a status condition rather than a hidden shell
//! pipeline failure.
//!
//! Phase pipeline (success path):
//!   Pending -> Archiving -> RevokingCredentials -> CleaningDns -> Attesting
//!   -> Deleting -> Complete
//!
//! Any step may fail; the phase moves to `Failed` and the corresponding
//! `TeardownCondition` carries the reason and message.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Archive target for a tenant namespace before deletion.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSpec {
    /// Destination URI (e.g. `s3://backups/acme`, `gs://...`).
    pub destination: String,

    /// Optional expected SHA-256 of the archive. When set, the controller
    /// verifies it before allowing the teardown to proceed past `Archiving`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_checksum: Option<String>,

    /// When true (default), the controller refuses to advance if the archive
    /// cannot be verified.
    #[serde(default = "default_true")]
    pub verify: bool,
}

fn default_true() -> bool {
    true
}

/// Namespace teardown specification.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceTeardownSpec {
    /// The namespace to be torn down.
    pub target_namespace: String,

    /// Optional pre-delete archive step. When omitted, the `Archiving` phase
    /// is skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveSpec>,

    /// Credential identifiers to revoke. Each must be confirmed revoked
    /// before the teardown moves past `RevokingCredentials`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_refs: Vec<String>,

    /// DNS record names to remove before deletion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns_records: Vec<String>,

    /// Optional sink where the final attestation is recorded (e.g. a
    /// ConfigMap name, an object-store URI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_sink: Option<String>,

    /// Optional per-step timeout in seconds; 0 or unset means the cluster
    /// default applies.
    #[serde(default)]
    pub step_timeout_seconds: u32,
}

/// Explicit teardown phase. The success path is linear; `Failed` is terminal
/// and set when a step errors out.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum TeardownPhase {
    #[default]
    Pending,
    Archiving,
    RevokingCredentials,
    CleaningDns,
    Attesting,
    Deleting,
    Complete,
    Failed,
}

/// Per-step status carried in `NamespaceTeardownStatus::conditions`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum TeardownStepStatus {
    #[default]
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

/// Condition reporting the status of one step in the pipeline.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeardownCondition {
    /// Step identifier, e.g. `Archiving` or `CleaningDns`.
    pub step: String,
    pub status: TeardownStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// RFC3339 timestamp of the last transition for this step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_transition: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceTeardownStatus {
    pub phase: TeardownPhase,
    #[serde(default)]
    pub conditions: Vec<TeardownCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// Checksum recorded after the archive completed and verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_checksum: Option<String>,
}

/// NamespaceTeardown CRD.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "NamespaceTeardown",
    namespaced,
    status = "NamespaceTeardownStatus",
    shortname = "nsteardown"
)]
pub struct NamespaceTeardownCrd {
    pub spec: NamespaceTeardownSpec,
    pub status: Option<NamespaceTeardownStatus>,
}

impl NamespaceTeardownSpec {
    /// The ordered pipeline of phases this teardown will pass through,
    /// skipping steps whose inputs are absent.
    pub fn step_order(&self) -> Vec<TeardownPhase> {
        let mut steps = vec![TeardownPhase::Pending];
        if self.archive.is_some() {
            steps.push(TeardownPhase::Archiving);
        }
        if !self.credential_refs.is_empty() {
            steps.push(TeardownPhase::RevokingCredentials);
        }
        if !self.dns_records.is_empty() {
            steps.push(TeardownPhase::CleaningDns);
        }
        if self.attestation_sink.is_some() {
            steps.push(TeardownPhase::Attesting);
        }
        steps.push(TeardownPhase::Deleting);
        steps.push(TeardownPhase::Complete);
        steps
    }

    /// Validate the teardown spec, returning the first violation found.
    pub fn validate(&self) -> Result<(), String> {
        if self.target_namespace.trim().is_empty() {
            return Err("targetNamespace must not be empty".to_string());
        }

        if let Some(archive) = &self.archive {
            if archive.destination.trim().is_empty() {
                return Err("archive.destination must not be empty".to_string());
            }
            if let Some(checksum) = &archive.expected_checksum {
                if checksum.len() != 64 || !checksum.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(
                        "archive.expectedChecksum must be a 64-character hex SHA-256".to_string(),
                    );
                }
            }
            if archive.verify && archive.expected_checksum.is_none() {
                return Err(
                    "archive.verify is true but no expectedChecksum was provided".to_string(),
                );
            }
        }

        if self.credential_refs.iter().any(|r| r.trim().is_empty()) {
            return Err("credentialRefs must not contain empty identifiers".to_string());
        }
        if self.dns_records.iter().any(|r| r.trim().is_empty()) {
            return Err("dnsRecords must not contain empty names".to_string());
        }

        Ok(())
    }

    /// True when the given phase is terminal.
    pub fn is_terminal(phase: &TeardownPhase) -> bool {
        matches!(phase, TeardownPhase::Complete | TeardownPhase::Failed)
    }
}
