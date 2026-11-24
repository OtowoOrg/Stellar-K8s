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
//! Integration and regression test suite for backup/restore and Disaster Recovery (DR) command flows.
//!
//! Issue #1113: Add regression suite for backup/restore and DR command flows.

use std::fs;
use tempfile::TempDir;

use stellar_k8s::backup::{
    BackupSource, BackupVerificationConfig, VerificationResources, VerificationStrategy,
};
use stellar_k8s::commands::backup::{
    run_backup, run_cleanup, run_list, run_restore, BackupArgs, CleanupArgs, ListArgs, RestoreArgs,
};
use stellar_k8s::crd::{
    DRPeerHealth, DRRole, DRSyncStrategy, DisasterRecoveryConfig, DisasterRecoveryStatus,
};

#[tokio::test]
async fn test_backup_and_restore_command_regression_flow() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let source_dir = temp_dir.path().join("source_data");
    let backup_dir = temp_dir.path().join("backups");
    let restore_dir = temp_dir.path().join("restored_data");

    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&backup_dir).unwrap();

    // Create sample data files
    fs::write(source_dir.join("ledger.db"), b"stellar-core-ledger-data-v1").unwrap();
    fs::write(source_dir.join("config.txt"), b"network_passphrase=testnet").unwrap();

    // 1. Run Backup Create
    let backup_args = BackupArgs {
        source: source_dir.clone(),
        backend: "file".to_string(),
        destination: backup_dir.to_str().unwrap().to_string(),
        incremental: false,
        verify: true,
    };

    let backup_res = run_backup(backup_args).await;
    assert!(
        backup_res.is_ok(),
        "Backup creation failed: {:?}",
        backup_res
    );

    // 2. Run Backup List
    let list_args = ListArgs {
        backend: "file".to_string(),
        location: backup_dir.to_str().unwrap().to_string(),
    };
    let list_res = run_list(list_args).await;
    assert!(list_res.is_ok(), "Backup list failed: {:?}", list_res);

    // 3. Run Backup Restore
    let restore_args = RestoreArgs {
        backup: backup_dir.to_str().unwrap().to_string(),
        destination: restore_dir.clone(),
        backend: "file".to_string(),
        verify: true,
    };
    let restore_res = run_restore(restore_args).await;
    assert!(
        restore_res.is_ok(),
        "Backup restore failed: {:?}",
        restore_res
    );

    // Verify restored file contents match original
    assert!(restore_dir.join("ledger.db").exists());
    let restored_db = fs::read(restore_dir.join("ledger.db")).unwrap();
    assert_eq!(restored_db, b"stellar-core-ledger-data-v1");

    // 4. Run Backup Cleanup
    let cleanup_args = CleanupArgs {
        backend: "file".to_string(),
        location: backup_dir.to_str().unwrap().to_string(),
        keep: 1,
    };
    let cleanup_res = run_cleanup(cleanup_args).await;
    assert!(
        cleanup_res.is_ok(),
        "Backup cleanup failed: {:?}",
        cleanup_res
    );
}

#[tokio::test]
async fn test_backup_restore_error_modes_regression() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let non_existent_source = temp_dir.path().join("does_not_exist");
    let dest_dir = temp_dir.path().join("destination");

    // Test non-existent source path returns Error
    let invalid_backup_args = BackupArgs {
        source: non_existent_source,
        backend: "file".to_string(),
        destination: dest_dir.to_str().unwrap().to_string(),
        incremental: false,
        verify: false,
    };

    let res = run_backup(invalid_backup_args).await;
    assert!(
        res.is_err(),
        "Expected backup of non-existent source to fail"
    );
}

#[test]
fn test_dr_policy_and_status_regression() {
    let dr_config = DisasterRecoveryConfig {
        enabled: true,
        role: DRRole::Primary,
        peer_cluster_id: "us-east-standby".to_string(),
        sync_strategy: DRSyncStrategy::Consensus,
        failover_dns: None,
        health_check_interval: 30,
        drill_schedule: None,
        policy_ref: Some("global-dr-policy".to_string()),
        archive_integrity_config: None,
    };

    assert!(dr_config.enabled);
    assert_eq!(dr_config.role, DRRole::Primary);

    let mut dr_status = DisasterRecoveryStatus {
        current_role: Some(DRRole::Primary),
        active_peer_cluster_id: Some("us-west-2".to_string()),
        peer_health: Some("Healthy".to_string()),
        peer_health_map: Some(vec![DRPeerHealth {
            cluster_id: "us-east-standby".to_string(),
            health: "Healthy".to_string(),
            last_contact: Some("2026-07-27T10:00:00Z".to_string()),
            priority: Some(1),
        }]),
        last_peer_contact: Some("2026-07-27T10:00:00Z".to_string()),
        sync_lag: Some(0),
        failover_active: false,
        last_failover_time: None,
        last_failover_reason: None,
        last_check_time: Some("2026-07-27T10:00:00Z".to_string()),
        last_drill_time: Some("2026-07-26T12:00:00Z".to_string()),
        last_drill_result: None,
    };

    assert_eq!(dr_status.current_role, Some(DRRole::Primary));

    // Simulate Failover Transition from Primary to Standby
    dr_status.current_role = Some(DRRole::Standby);
    dr_status.failover_active = true;
    dr_status.last_failover_time = Some("2026-07-27T11:00:00Z".to_string());
    dr_status.active_peer_cluster_id = Some("us-east-1".to_string());

    assert_eq!(dr_status.current_role, Some(DRRole::Standby));
    assert!(dr_status.failover_active);
    assert_eq!(
        dr_status.active_peer_cluster_id,
        Some("us-east-1".to_string())
    );
}

#[test]
fn test_backup_verification_config_regression() {
    let verification_config = BackupVerificationConfig {
        enabled: true,
        schedule: "0 4 * * *".to_string(),
        backup_source: BackupSource::S3 {
            bucket: "stellar-dr-backups".to_string(),
            region: "us-east-1".to_string(),
            prefix: "validators/".to_string(),
            credentials_secret: "s3-secret".to_string(),
        },
        strategy: VerificationStrategy::Full,
        timeout_minutes: 45,
        rpo_target_minutes: 180,
        retention_days: 30,
        point_in_time_restore: true,
        benchmark_enabled: true,
        notification_webhook: Some("https://alerts.stellar.example.com".to_string()),
        report_storage: None,
        resources: VerificationResources {
            cpu_limit: "4000m".to_string(),
            memory_limit: "8Gi".to_string(),
            storage_size: "200Gi".to_string(),
        },
    };

    assert!(verification_config.enabled);
    assert_eq!(verification_config.strategy, VerificationStrategy::Full);
    assert_eq!(verification_config.resources.cpu_limit, "4000m");
}
