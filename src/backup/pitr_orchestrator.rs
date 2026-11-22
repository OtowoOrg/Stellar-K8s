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
//! Intent-driven BackupPlan & Point-in-Time Recovery (PITR) orchestration.
//!
//! Enforces restore verification as a blocking gate before any backup is marked
//! `Complete`, computes PITR recovery chains against declared RPO/RTO targets,
//! and verifies cross-region copies by SHA-256 checksum.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crd::backup_plan::{
    BackupExecutionPhase, BackupPlanSpec, BackupPlanStatus, CrossRegionCopyStatus,
    VerifiedBackupRecord,
};

/// Target point for Point-in-Time Recovery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PitrTarget {
    Timestamp(DateTime<Utc>),
    LedgerSequence(u64),
}

/// Backup artifact metadata tracked by the orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupArtifact {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub ledger_sequence: u64,
    pub sha256_checksum: String,
    pub wal_segments: Vec<String>,
    pub size_bytes: u64,
}

/// Outcome of the blocking test-restore verification gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreVerificationOutcome {
    pub backup_id: String,
    pub passed: bool,
    pub restore_duration_seconds: u64,
    pub checksum_matched: bool,
    pub queries_passed: usize,
    pub failure_reason: Option<String>,
}

/// Computed PITR execution plan for a target timestamp or ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitrRecoveryPlan {
    pub base_backup_id: String,
    pub target: PitrTarget,
    pub wal_segments_to_replay: Vec<String>,
    pub estimated_rto_seconds: u64,
    pub meets_declared_rto: bool,
}

/// Orchestrates BackupPlan lifecycle with blocking restore verification.
pub struct BackupPlanOrchestrator;

impl BackupPlanOrchestrator {
    /// Execute a backup lifecycle transition.
    ///
    /// A backup MUST NOT transition to `BackupExecutionPhase::Complete` unless
    /// `verification.passed` is true (when `spec.verification.blocking` is true).
    pub fn finalize_backup(
        spec: &BackupPlanSpec,
        artifact: &BackupArtifact,
        verification: &RestoreVerificationOutcome,
        remote_checksum: Option<&str>,
        previous_status: Option<&BackupPlanStatus>,
    ) -> BackupPlanStatus {
        let mut status = previous_status.cloned().unwrap_or_default();

        if spec.verification.blocking && (!verification.passed || !verification.checksum_matched) {
            status.phase = BackupExecutionPhase::VerificationFailed;
            status.last_error = verification.failure_reason.clone().or_else(|| {
                Some("Blocking restore verification failed; backup cannot be marked Complete".into())
            });
            return status;
        }

        if let Some(cross_region) = &spec.cross_region_copy {
            let remote = remote_checksum.unwrap_or("");
            let matched = !remote.is_empty() && remote == artifact.sha256_checksum;
            status.cross_region_status = Some(CrossRegionCopyStatus {
                destination_region: cross_region.destination_region.clone(),
                source_checksum: artifact.sha256_checksum.clone(),
                remote_checksum: remote.to_string(),
                checksum_verified: matched,
                synced_at: Utc::now(),
            });
            if !matched {
                status.phase = BackupExecutionPhase::CrossRegionSyncing;
                status.last_error = Some(format!(
                    "Cross-region checksum mismatch for region {}",
                    cross_region.destination_region
                ));
                return status;
            }
        }

        let achieved_rpo = (Utc::now() - artifact.created_at).num_seconds().max(0) as u64;
        status.achieved_rpo_seconds = Some(achieved_rpo);
        if achieved_rpo <= spec.rpo_target_seconds {
            status.consecutive_rpo_compliant_days =
                status.consecutive_rpo_compliant_days.saturating_add(1);
        } else {
            status.consecutive_rpo_compliant_days = 0;
        }

        status.last_drill_rto_seconds = Some(verification.restore_duration_seconds);
        status.phase = BackupExecutionPhase::Complete;
        status.last_error = None;
        status.latest_verified_backup = Some(VerifiedBackupRecord {
            backup_id: artifact.id.clone(),
            completed_at: Utc::now(),
            sha256_checksum: artifact.sha256_checksum.clone(),
            restore_verified: verification.passed,
            pitr_earliest_timestamp: Some(artifact.created_at),
            pitr_latest_timestamp: Some(Utc::now()),
        });
        status
    }

    /// Select the best base snapshot + WAL chain for a PITR target.
    pub fn plan_pitr_recovery(
        spec: &BackupPlanSpec,
        artifacts: &[BackupArtifact],
        target: PitrTarget,
    ) -> Result<PitrRecoveryPlan, String> {
        if !spec.pitr.enabled {
            return Err("PITR is not enabled on this BackupPlan".to_string());
        }

        let candidate = match &target {
            PitrTarget::Timestamp(ts) => artifacts
                .iter()
                .filter(|a| a.created_at <= *ts)
                .max_by_key(|a| a.created_at),
            PitrTarget::LedgerSequence(seq) => artifacts
                .iter()
                .filter(|a| a.ledger_sequence <= *seq)
                .max_by_key(|a| a.ledger_sequence),
        }
        .ok_or_else(|| "No eligible base backup found before PITR target".to_string())?;

        // Estimate 1s per 64 MiB base restore + 2s per WAL segment replay
        let base_secs = (candidate.size_bytes / (64 * 1024 * 1024)).max(1);
        let wal_secs = (candidate.wal_segments.len() as u64) * 2;
        let estimated_rto = base_secs + wal_secs;

        Ok(PitrRecoveryPlan {
            base_backup_id: candidate.id.clone(),
            target,
            wal_segments_to_replay: candidate.wal_segments.clone(),
            estimated_rto_seconds: estimated_rto,
            meets_declared_rto: estimated_rto <= spec.rto_target_seconds,
        })
    }

    /// Compute SHA-256 checksum for backup artifact verification.
    pub fn compute_checksum(payload: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(payload);
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::backup_plan::{
        CrossRegionCopyConfig, PitrConfig, RestoreVerificationPolicy,
    };
    use chrono::Duration;

    fn sample_spec() -> BackupPlanSpec {
        BackupPlanSpec {
            target_node_ref: "mainnet-validator".into(),
            schedule: "0 * * * *".into(),
            rpo_target_seconds: 300,
            rto_target_seconds: 600,
            pitr: PitrConfig {
                enabled: true,
                wal_archive_uri: "s3://stellar-wal/mainnet".into(),
                retention_days: 30,
            },
            verification: RestoreVerificationPolicy::default(),
            cross_region_copy: Some(CrossRegionCopyConfig {
                destination_region: "us-west-2".into(),
                destination_bucket: "stellar-backups-dr".into(),
                checksum_algorithm: "sha256".into(),
            }),
        }
    }

    #[test]
    fn failed_restore_verification_blocks_complete_status() {
        let spec = sample_spec();
        let checksum = BackupPlanOrchestrator::compute_checksum(b"snapshot-bytes");
        let artifact = BackupArtifact {
            id: "bk-001".into(),
            created_at: Utc::now() - Duration::seconds(30),
            ledger_sequence: 50_000_000,
            sha256_checksum: checksum.clone(),
            wal_segments: vec!["0001".into(), "0002".into()],
            size_bytes: 128 * 1024 * 1024,
        };
        let failed = RestoreVerificationOutcome {
            backup_id: "bk-001".into(),
            passed: false,
            restore_duration_seconds: 45,
            checksum_matched: true,
            queries_passed: 0,
            failure_reason: Some("Row count mismatch".into()),
        };

        let status =
            BackupPlanOrchestrator::finalize_backup(&spec, &artifact, &failed, Some(&checksum), None);
        assert_eq!(status.phase, BackupExecutionPhase::VerificationFailed);
        assert!(status.latest_verified_backup.is_none());
    }

    #[test]
    fn passing_verification_and_checksum_marks_complete_and_plans_pitr() {
        let spec = sample_spec();
        let checksum = BackupPlanOrchestrator::compute_checksum(b"snapshot-bytes");
        let now = Utc::now();
        let artifact = BackupArtifact {
            id: "bk-002".into(),
            created_at: now - Duration::seconds(60),
            ledger_sequence: 50_000_100,
            sha256_checksum: checksum.clone(),
            wal_segments: vec!["0001".into(), "0002".into(), "0003".into()],
            size_bytes: 256 * 1024 * 1024,
        };
        let passed = RestoreVerificationOutcome {
            backup_id: "bk-002".into(),
            passed: true,
            restore_duration_seconds: 42,
            checksum_matched: true,
            queries_passed: 1,
            failure_reason: None,
        };

        let status =
            BackupPlanOrchestrator::finalize_backup(&spec, &artifact, &passed, Some(&checksum), None);
        assert_eq!(status.phase, BackupExecutionPhase::Complete);
        assert!(status.cross_region_status.as_ref().unwrap().checksum_verified);

        let plan = BackupPlanOrchestrator::plan_pitr_recovery(
            &spec,
            &[artifact],
            PitrTarget::Timestamp(now - Duration::seconds(10)),
        )
        .unwrap();
        assert_eq!(plan.base_backup_id, "bk-002");
        assert!(plan.meets_declared_rto);
    }
}
