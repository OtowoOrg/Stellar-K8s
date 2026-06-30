//! Disaster Recovery Custom Resource Definitions
//!
//! The StellarBackup, StellarRestore, and StellarDRDrill CRDs define
//! comprehensive disaster recovery automation for Stellar K8s.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::Condition;

// ============================================================================
// StellarBackup CRD
// ============================================================================

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarBackup",
    namespaced,
    status = "StellarBackupStatus",
    shortname = "sb",
    printcolumn = r#"{"name":"Schedule","type":"string","jsonPath":".spec.schedule"}"#,
    printcolumn = r#"{"name":"LastBackup","type":"date","jsonPath":".status.lastBackupTime"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarBackupSpec {
    /// Cron schedule for automated backups
    pub schedule: String,

    /// Backup destination configuration
    pub destination: BackupDestination,

    /// Retention policy for backups
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_policy: Option<RetentionPolicy>,

    /// Encryption settings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EncryptionConfig>,

    /// Cross-region replication
    #[serde(default)]
    pub cross_region_replication: bool,

    /// Backup verification enabled
    #[serde(default)]
    pub verify_backups: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackupDestination {
    /// Destination type (S3, GCS, AzureBlob)
    pub dest_type: String,

    /// Bucket or container name
    pub bucket: String,

    /// Region or location
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,

    /// Credentials secret reference
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_secret: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPolicy {
    /// Daily backup retention in days
    #[serde(default = "default_daily_retention")]
    pub daily_retention_days: u32,

    /// Weekly backup retention in weeks
    #[serde(default = "default_weekly_retention")]
    pub weekly_retention_weeks: u32,

    /// Monthly backup retention in months
    #[serde(default = "default_monthly_retention")]
    pub monthly_retention_months: u32,
}

fn default_daily_retention() -> u32 {
    7
}
fn default_weekly_retention() -> u32 {
    4
}
fn default_monthly_retention() -> u32 {
    12
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionConfig {
    /// Enable encryption
    #[serde(default)]
    pub enabled: bool,

    /// Encryption algorithm
    #[serde(default = "default_encryption_algo")]
    pub algorithm: String,
}

fn default_encryption_algo() -> String {
    "AES256".to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarBackupStatus {
    /// Conditions of this resource
    #[serde(default)]
    pub conditions: Vec<Condition>,

    /// Last backup time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup_time: Option<String>,

    /// Total backup count
    #[serde(default)]
    pub backup_count: u32,

    /// Total backed up size
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_size: Option<String>,

    /// Backup status
    pub backup_status: Option<String>,

    /// Next scheduled backup time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_backup_time: Option<String>,
}

impl Default for StellarBackupStatus {
    fn default() -> Self {
        Self {
            conditions: Vec::new(),
            last_backup_time: None,
            backup_count: 0,
            total_size: None,
            backup_status: Some("Idle".to_string()),
            next_backup_time: None,
        }
    }
}

// ============================================================================
// StellarRestore CRD
// ============================================================================

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarRestore",
    namespaced,
    status = "StellarRestoreStatus",
    shortname = "sr",
    printcolumn = r#"{"name":"TargetLedger","type":"integer","jsonPath":".spec.targetLedger"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarRestoreSpec {
    /// Reference to backup to restore from
    pub backup_ref: String,

    /// Target ledger number for point-in-time restore
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_ledger: Option<u64>,

    /// Target timestamp for point-in-time restore
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_timestamp: Option<String>,

    /// Parallelism level for restore
    #[serde(default = "default_parallelism")]
    pub parallelism: u32,

    /// Verify restore integrity
    #[serde(default)]
    pub verify_restore: bool,
}

fn default_parallelism() -> u32 {
    4
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum RestorePhase {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarRestoreStatus {
    /// Current restore phase
    pub phase: Option<String>,

    /// Conditions of this resource
    #[serde(default)]
    pub conditions: Vec<Condition>,

    /// Restore progress (0-100)
    #[serde(default)]
    pub restore_progress: u32,

    /// Start time of restore
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,

    /// Completion time of restore
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_time: Option<String>,

    /// Estimated remaining time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_remaining_time: Option<String>,
}

impl Default for StellarRestoreStatus {
    fn default() -> Self {
        Self {
            phase: Some("Pending".to_string()),
            conditions: Vec::new(),
            restore_progress: 0,
            start_time: None,
            completion_time: None,
            estimated_remaining_time: None,
        }
    }
}

// ============================================================================
// StellarDRDrill CRD
// ============================================================================

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarDRDrill",
    namespaced,
    status = "StellarDRDrillStatus",
    shortname = "sdd",
    printcolumn = r#"{"name":"Schedule","type":"string","jsonPath":".spec.schedule"}"#,
    printcolumn = r#"{"name":"LastDrill","type":"date","jsonPath":".status.lastDrillTime"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarDRDrillSpec {
    /// Cron schedule for DR drills
    pub schedule: String,

    /// Target DR region for failover
    pub dr_region: String,

    /// Automatically revert after drill
    #[serde(default)]
    pub auto_revert: bool,

    /// Revert timeout duration
    #[serde(default = "default_revert_timeout")]
    pub revert_timeout: String,

    /// Email for drill results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification_email: Option<String>,

    /// Slack channel for notifications
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification_slack: Option<String>,
}

fn default_revert_timeout() -> String {
    "1h".to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum DrillStatus {
    Passed,
    Failed,
    Running,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarDRDrillStatus {
    /// Conditions of this resource
    #[serde(default)]
    pub conditions: Vec<Condition>,

    /// Last drill time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_drill_time: Option<String>,

    /// Last drill status
    pub last_drill_status: Option<String>,

    /// Total drill count
    #[serde(default)]
    pub drill_count: u32,

    /// Average recovery time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_recovery_time: Option<String>,

    /// Next scheduled drill time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_drill_time: Option<String>,

    /// Current drill RTO (Recovery Time Objective)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_rto: Option<String>,
}

impl Default for StellarDRDrillStatus {
    fn default() -> Self {
        Self {
            conditions: Vec::new(),
            last_drill_time: None,
            last_drill_status: Some("Pending".to_string()),
            drill_count: 0,
            average_recovery_time: None,
            next_drill_time: None,
            current_rto: None,
        }
    }
}
