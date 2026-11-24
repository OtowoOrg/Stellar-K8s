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
//! Isolated fixture suite for backup verification types.
//!
//! Refactored in issue #1140: inline data builders replaced by shared fixtures;
//! cluster-dependent tests gated behind `#[ignore]` instead of silent returns.

mod common;

use common::fixtures::{
    backup_verification_defaults, backup_verification_quick, s3_backup_source,
    volume_snapshot_backup_source,
};
use stellar_k8s::backup::{
    BackupSource, BackupVerificationConfig, VerificationResources, VerificationStrategy,
};

// ── Default value contract ────────────────────────────────────────────────────

#[tokio::test]
async fn test_backup_verification_config_default() {
    let config = backup_verification_defaults();

    assert!(!config.enabled);
    assert_eq!(config.schedule, "0 2 * * 0");
    assert_eq!(config.timeout_minutes, 60);
    assert_eq!(config.rpo_target_minutes, 60);
    assert_eq!(config.retention_days, 30);
    assert!(!config.point_in_time_restore);
    assert!(!config.benchmark_enabled);
    assert_eq!(config.strategy, VerificationStrategy::Standard);
}

#[tokio::test]
async fn test_backup_verification_quick_fixture() {
    let config = backup_verification_quick();

    assert!(config.enabled);
    assert_eq!(config.strategy, VerificationStrategy::Quick);
    assert_eq!(config.timeout_minutes, 5);
}

// ── BackupSource serialisation round-trips ────────────────────────────────────

#[tokio::test]
async fn test_backup_source_s3_round_trip() {
    let source = s3_backup_source();

    let json = serde_json::to_string(&source).unwrap();
    let deserialized: BackupSource = serde_json::from_str(&json).unwrap();

    assert_eq!(source, deserialized);
}

#[tokio::test]
async fn test_backup_source_s3_custom_fields() {
    let source = BackupSource::S3 {
        bucket: "test-bucket".to_string(),
        region: "us-east-1".to_string(),
        prefix: "backups/".to_string(),
        credentials_secret: "aws-creds".to_string(),
    };

    let json = serde_json::to_string(&source).unwrap();
    let deserialized: BackupSource = serde_json::from_str(&json).unwrap();

    assert_eq!(source, deserialized);
}

#[tokio::test]
async fn test_backup_source_volume_snapshot_round_trip() {
    let source = volume_snapshot_backup_source();

    let json = serde_json::to_string(&source).unwrap();
    let deserialized: BackupSource = serde_json::from_str(&json).unwrap();

    assert_eq!(source, deserialized);
}

#[tokio::test]
async fn test_backup_source_volume_snapshot_custom_fields() {
    let source = BackupSource::VolumeSnapshot {
        snapshot_name: "my-snapshot".to_string(),
        storage_class: "fast-ssd".to_string(),
    };

    let json = serde_json::to_string(&source).unwrap();
    let deserialized: BackupSource = serde_json::from_str(&json).unwrap();

    assert_eq!(source, deserialized);
}

// ── VerificationStrategy enum ─────────────────────────────────────────────────

#[tokio::test]
async fn test_verification_strategy_variants_are_distinct() {
    let quick = VerificationStrategy::Quick;
    let standard = VerificationStrategy::Standard;
    let full = VerificationStrategy::Full;

    assert_ne!(quick, standard);
    assert_ne!(standard, full);
    assert_ne!(quick, full);
}

// ── VerificationResources defaults ───────────────────────────────────────────

#[tokio::test]
async fn test_verification_resources_default() {
    let resources = VerificationResources::default();

    assert_eq!(resources.cpu_limit, "2000m");
    assert_eq!(resources.memory_limit, "4Gi");
    assert_eq!(resources.storage_size, "100Gi");
}

// ── Full config serialisation ─────────────────────────────────────────────────

#[tokio::test]
async fn test_backup_verification_config_full_serialization() {
    let config = BackupVerificationConfig {
        enabled: true,
        schedule: "0 2 * * 0".to_string(),
        backup_source: s3_backup_source(),
        strategy: VerificationStrategy::Full,
        timeout_minutes: 120,
        rpo_target_minutes: 30,
        retention_days: 90,
        point_in_time_restore: true,
        benchmark_enabled: true,
        notification_webhook: Some("https://webhook.example.com".to_string()),
        report_storage: None,
        resources: VerificationResources::default(),
    };

    let json = serde_json::to_string(&config).unwrap();
    let deserialized: BackupVerificationConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(config, deserialized);
}

#[test]
fn test_backup_verification_config_rejects_invalid_recovery_objectives() {
    let mut config = BackupVerificationConfig {
        timeout_minutes: 0,
        ..Default::default()
    };
    assert!(config.validate().is_err());

    config.timeout_minutes = 60;
    config.rpo_target_minutes = 0;
    assert!(config.validate().is_err());

    config.rpo_target_minutes = 60;
    config.retention_days = 0;
    assert!(config.validate().is_err());
}

#[test]
fn test_backup_verification_config_accepts_pitr_schedule() {
    let config = BackupVerificationConfig {
        point_in_time_restore: true,
        schedule: "0 0 0 1 * *".to_string(),
        ..BackupVerificationConfig::default()
    };

    assert!(config.validate().is_ok());
}
