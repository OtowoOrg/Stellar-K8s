//! Snapshot Management for performance optimization

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use tracing::debug;

use crate::error::Result;

/// Snapshot configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SnapshotConfig {
    /// Enable snapshots
    pub enabled: bool,
    /// Create snapshot every N events
    pub snapshot_interval: usize,
    /// Maximum snapshots per aggregate
    pub max_snapshots_per_aggregate: usize,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            snapshot_interval: 100,
            max_snapshots_per_aggregate: 5,
        }
    }
}

/// Snapshot
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Snapshot ID
    pub id: String,
    /// Aggregate ID
    pub aggregate_id: String,
    /// Aggregate state at snapshot
    pub state: serde_json::Value,
    /// Event sequence number at snapshot
    pub sequence_number: u64,
    /// When snapshot was created
    pub created_at: DateTime<Utc>,
}

impl Snapshot {
    pub fn new(
        aggregate_id: String,
        state: serde_json::Value,
        sequence_number: u64,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            aggregate_id,
            state,
            sequence_number,
            created_at: Utc::now(),
        }
    }
}

/// Snapshot Manager
pub struct SnapshotManager {
    config: SnapshotConfig,
    snapshots: tokio::sync::RwLock<HashMap<String, Vec<Snapshot>>>,
}

impl SnapshotManager {
    /// Create a new snapshot manager
    pub async fn new(config: SnapshotConfig) -> Result<Self> {
        debug!("Initializing Snapshot Manager");
        Ok(Self {
            config,
            snapshots: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Create snapshot
    pub async fn create_snapshot(
        &self,
        aggregate_id: String,
        state: serde_json::Value,
        sequence_number: u64,
    ) -> Result<Snapshot> {
        debug!(
            "Creating snapshot for aggregate {} at sequence {}",
            aggregate_id, sequence_number
        );

        let snapshot = Snapshot::new(aggregate_id.clone(), state, sequence_number);

        let mut snapshots = self.snapshots.write().await;
        let agg_snapshots = snapshots.entry(aggregate_id).or_insert_with(Vec::new);

        agg_snapshots.push(snapshot.clone());

        // Keep only max snapshots
        if agg_snapshots.len() > self.config.max_snapshots_per_aggregate {
            agg_snapshots.remove(0);
        }

        Ok(snapshot)
    }

    /// Get latest snapshot
    pub async fn get_snapshot(&self, aggregate_id: &str) -> Result<Option<Snapshot>> {
        let snapshots = self.snapshots.read().await;
        Ok(snapshots
            .get(aggregate_id)
            .and_then(|snaps| snaps.last().cloned()))
    }

    /// Get snapshot at sequence number
    pub async fn get_snapshot_at(
        &self,
        aggregate_id: &str,
        sequence_number: u64,
    ) -> Result<Option<Snapshot>> {
        let snapshots = self.snapshots.read().await;
        Ok(snapshots
            .get(aggregate_id)
            .and_then(|snaps| {
                snaps
                    .iter()
                    .rev()
                    .find(|s| s.sequence_number <= sequence_number)
                    .cloned()
            }))
    }

    /// Delete old snapshots
    pub async fn delete_old_snapshots(&self, keep_count: usize) -> Result<usize> {
        debug!("Deleting old snapshots (keep {})", keep_count);

        let mut snapshots = self.snapshots.write().await;
        let mut deleted = 0;

        for snaps in snapshots.values_mut() {
            if snaps.len() > keep_count {
                deleted += snaps.len() - keep_count;
                snaps.drain(0..snaps.len() - keep_count);
            }
        }

        Ok(deleted)
    }

    /// Get snapshot statistics
    pub async fn get_statistics(&self) -> Result<SnapshotStatistics> {
        let snapshots = self.snapshots.read().await;

        let total_aggregates = snapshots.len();
        let total_snapshots: usize = snapshots.values().map(|v| v.len()).sum();

        Ok(SnapshotStatistics {
            total_aggregates,
            total_snapshots,
            timestamp: Utc::now(),
        })
    }
}

/// Snapshot statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotStatistics {
    pub total_aggregates: usize,
    pub total_snapshots: usize,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_snapshot_manager_creation() {
        let config = SnapshotConfig::default();
        let manager = SnapshotManager::new(config).await.unwrap();
        
        let stats = manager.get_statistics().await.unwrap();
        assert_eq!(stats.total_snapshots, 0);
    }

    #[tokio::test]
    async fn test_create_snapshot() {
        let config = SnapshotConfig::default();
        let manager = SnapshotManager::new(config).await.unwrap();

        let snapshot = manager
            .create_snapshot(
                "agg123".to_string(),
                serde_json::json!({"state": "active"}),
                100,
            )
            .await
            .unwrap();

        assert_eq!(snapshot.aggregate_id, "agg123");
        assert_eq!(snapshot.sequence_number, 100);
    }

    #[tokio::test]
    async fn test_get_snapshot() {
        let config = SnapshotConfig::default();
        let manager = SnapshotManager::new(config).await.unwrap();

        manager
            .create_snapshot(
                "agg123".to_string(),
                serde_json::json!({"state": "active"}),
                100,
            )
            .await
            .unwrap();

        let snapshot = manager.get_snapshot("agg123").await.unwrap();
        assert!(snapshot.is_some());
        assert_eq!(snapshot.unwrap().sequence_number, 100);
    }
}
