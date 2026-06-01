//! Database backup and point-in-time recovery (PITR) management.
//!
//! Triggers pg_basebackup-style logical snapshots and tracks WAL positions
//! for PITR. Integrates with the existing backup scheduler patterns.

use crate::db_management::types::DbAlert;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::info;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub wal_start_lsn: String,
    pub wal_end_lsn: Option<String>,
    pub size_bytes: Option<i64>,
    pub status: BackupStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupStatus {
    InProgress,
    Completed,
    Failed(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PitrPoint {
    pub lsn: String,
    pub recorded_at: DateTime<Utc>,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupReport {
    pub analyzed_at: DateTime<Utc>,
    pub current_lsn: String,
    pub wal_level: String,
    pub archive_mode: String,
    pub recent_backups: Vec<BackupRecord>,
    pub alerts: Vec<DbAlert>,
}

pub struct BackupManager {
    /// In-memory log of backup records (production would persist to a table)
    records: tokio::sync::RwLock<Vec<BackupRecord>>,
}

impl BackupManager {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { records: tokio::sync::RwLock::new(vec![]) })
    }

    /// Collect WAL/archive configuration and recent backup metadata.
    pub async fn report(&self, pool: &PgPool) -> crate::error::Result<BackupReport> {
        let (lsn,): (String,) = sqlx::query_as("SELECT pg_current_wal_lsn()::text")
            .fetch_one(pool)
            .await
            .map_err(crate::error::Error::SqlxError)?;

        let (wal_level,): (String,) =
            sqlx::query_as("SELECT setting FROM pg_settings WHERE name = 'wal_level'")
                .fetch_one(pool)
                .await
                .map_err(crate::error::Error::SqlxError)?;

        let (archive_mode,): (String,) =
            sqlx::query_as("SELECT setting FROM pg_settings WHERE name = 'archive_mode'")
                .fetch_one(pool)
                .await
                .map_err(crate::error::Error::SqlxError)?;

        let mut alerts = vec![];
        if wal_level == "minimal" {
            alerts.push(DbAlert::warn("backup_pitr", "wal_level=minimal; PITR not possible"));
        }
        if archive_mode == "off" {
            alerts.push(DbAlert::warn("backup_pitr", "archive_mode=off; WAL archiving disabled"));
        }

        let records = self.records.read().await.clone();
        info!("backup report: lsn={lsn}, wal_level={wal_level}, archive_mode={archive_mode}");

        Ok(BackupReport {
            analyzed_at: Utc::now(),
            current_lsn: lsn,
            wal_level,
            archive_mode,
            recent_backups: records,
            alerts,
        })
    }

    /// Record the start of a backup and return its ID.
    pub async fn begin_backup(&self, pool: &PgPool, label: &str) -> crate::error::Result<String> {
        let (lsn,): (String,) =
            sqlx::query_as(&format!("SELECT pg_backup_start('{label}', true)::text"))
                .fetch_one(pool)
                .await
                .map_err(crate::error::Error::SqlxError)?;

        let id = format!("bkp-{}", Utc::now().timestamp());
        let record = BackupRecord {
            id: id.clone(),
            started_at: Utc::now(),
            completed_at: None,
            wal_start_lsn: lsn,
            wal_end_lsn: None,
            size_bytes: None,
            status: BackupStatus::InProgress,
        };
        self.records.write().await.push(record);
        info!("backup started: id={id}");
        Ok(id)
    }

    /// Mark a backup as complete.
    pub async fn finish_backup(&self, pool: &PgPool, id: &str) -> crate::error::Result<()> {
        let (lsn,): (String,) = sqlx::query_as("SELECT pg_backup_stop()::text")
            .fetch_one(pool)
            .await
            .map_err(crate::error::Error::SqlxError)?;

        let mut records = self.records.write().await;
        if let Some(r) = records.iter_mut().find(|r| r.id == id) {
            r.completed_at = Some(Utc::now());
            r.wal_end_lsn = Some(lsn);
            r.status = BackupStatus::Completed;
        }
        info!("backup completed: id={id}");
        Ok(())
    }

    /// Record a PITR recovery point at the current LSN.
    pub async fn create_recovery_point(pool: &PgPool, label: &str) -> crate::error::Result<PitrPoint> {
        let (lsn,): (String,) =
            sqlx::query_as(&format!("SELECT pg_create_restore_point('{label}')::text"))
                .fetch_one(pool)
                .await
                .map_err(crate::error::Error::SqlxError)?;

        info!("PITR recovery point '{label}' at LSN {lsn}");
        Ok(PitrPoint { lsn, recorded_at: Utc::now(), label: label.to_string() })
    }
}

impl Default for BackupManager {
    fn default() -> Self {
        Self { records: tokio::sync::RwLock::new(vec![]) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_status_eq() {
        assert_eq!(BackupStatus::Completed, BackupStatus::Completed);
        assert_ne!(BackupStatus::InProgress, BackupStatus::Completed);
    }
}
