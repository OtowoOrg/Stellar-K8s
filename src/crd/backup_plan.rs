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

//! Declarative BackupPlan CRD for intent-driven backups, PITR targeting,
//! and blocking restore verification.

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Declarative BackupPlan custom resource specification.
#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "BackupPlan",
    plural = "backupplans",
    shortname = "bp",
    namespaced
)]
#[kube(status = "BackupPlanStatus")]
#[serde(rename_all = "camelCase")]
pub struct BackupPlanSpec {
    /// Target StellarNode or datastore resource name.
    pub target_node_ref: String,

    /// Cron schedule for base snapshots.
    pub schedule: String,

    /// Declared Recovery Point Objective in seconds.
    pub rpo_target_seconds: u64,

    /// Declared Recovery Time Objective in seconds.
    pub rto_target_seconds: u64,

    /// Point-in-time recovery configuration.
    pub pitr: PitrConfig,

    /// Restore verification policy (blocking by default).
    #[serde(default)]
    pub verification: RestoreVerificationPolicy,

    /// Optional cross-region replication with checksum verification.
    pub cross_region_copy: Option<CrossRegionCopyConfig>,
}

/// Point-in-time recovery configuration.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PitrConfig {
    pub enabled: bool,
    pub wal_archive_uri: String,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

fn default_retention_days() -> u32 {
    30
}

/// Restore verification policy treating verification as a blocking condition.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreVerificationPolicy {
    /// A backup cannot transition to Complete until test restore passes.
    #[serde(default = "default_true")]
    pub blocking: bool,
    #[serde(default = "default_true")]
    pub verify_checksum: bool,
    #[serde(default)]
    pub sample_queries: Vec<String>,
}

impl Default for RestoreVerificationPolicy {
    fn default() -> Self {
        Self {
            blocking: true,
            verify_checksum: true,
            sample_queries: vec!["SELECT 1".to_string()],
        }
    }
}

fn default_true() -> bool {
    true
}

/// Cross-region backup copy configuration.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CrossRegionCopyConfig {
    pub destination_region: String,
    pub destination_bucket: String,
    #[serde(default = "default_checksum_alg")]
    pub checksum_algorithm: String,
}

fn default_checksum_alg() -> String {
    "sha256".to_string()
}

/// Backup execution lifecycle phase.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
pub enum BackupExecutionPhase {
    #[default]
    Pending,
    Snapshotting,
    VerifyingRestore,
    CrossRegionSyncing,
    Complete,
    VerificationFailed,
}

/// Status of a verified backup artifact.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedBackupRecord {
    pub backup_id: String,
    pub completed_at: DateTime<Utc>,
    pub sha256_checksum: String,
    pub restore_verified: bool,
    pub pitr_earliest_timestamp: Option<DateTime<Utc>>,
    pub pitr_latest_timestamp: Option<DateTime<Utc>>,
}

/// Cross-region copy verification status.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CrossRegionCopyStatus {
    pub destination_region: String,
    pub source_checksum: String,
    pub remote_checksum: String,
    pub checksum_verified: bool,
    pub synced_at: DateTime<Utc>,
}

/// Status subresource for BackupPlan.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct BackupPlanStatus {
    pub phase: BackupExecutionPhase,
    pub latest_verified_backup: Option<VerifiedBackupRecord>,
    pub achieved_rpo_seconds: Option<u64>,
    pub consecutive_rpo_compliant_days: u32,
    pub last_drill_rto_seconds: Option<u64>,
    pub cross_region_status: Option<CrossRegionCopyStatus>,
    pub last_error: Option<String>,
}
