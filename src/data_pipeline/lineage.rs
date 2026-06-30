//! Data lineage tracking and audit trail for the pipeline.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// A single lineage event recorded as a record moves through the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageEvent {
    pub record_id: String,
    pub stage: String,
    pub status: LineageStatus,
    pub timestamp: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LineageStatus {
    Received,
    Transformed,
    ValidationFailed,
    SinkSuccess,
    SinkFailed,
    DeadLettered,
}

/// Thread-safe lineage tracker that stores an in-memory audit trail.
///
/// In production this would be persisted to a database or shipped to an
/// audit log service; the in-memory ring buffer here is sufficient for
/// operator-level observability.
#[derive(Clone)]
pub struct LineageTracker {
    /// Capped ring buffer — keeps the last `capacity` events.
    events: Arc<RwLock<Vec<LineageEvent>>>,
    capacity: usize,
}

impl LineageTracker {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::with_capacity(capacity))),
            capacity,
        }
    }

    /// Record a lineage event for `record_id` at `stage`.
    pub async fn record(
        &self,
        record_id: impl Into<String>,
        stage: impl Into<String>,
        status: LineageStatus,
        detail: Option<String>,
    ) {
        let event = LineageEvent {
            record_id: record_id.into(),
            stage: stage.into(),
            status,
            timestamp: Utc::now().to_rfc3339(),
            detail,
        };
        debug!(record_id = %event.record_id, stage = %event.stage, "lineage event");
        let mut guard = self.events.write().await;
        if guard.len() >= self.capacity {
            guard.remove(0);
        }
        guard.push(event);
    }

    /// Return a snapshot of all stored lineage events.
    pub async fn snapshot(&self) -> Vec<LineageEvent> {
        self.events.read().await.clone()
    }

    /// Return lineage events for a specific record ID.
    pub async fn for_record(&self, record_id: &str) -> Vec<LineageEvent> {
        self.events
            .read()
            .await
            .iter()
            .filter(|e| e.record_id == record_id)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_and_retrieves_events() {
        let tracker = LineageTracker::new(100);
        tracker
            .record("rec-1", "etl", LineageStatus::Transformed, None)
            .await;
        tracker
            .record("rec-1", "postgres_sink", LineageStatus::SinkSuccess, None)
            .await;
        let events = tracker.for_record("rec-1").await;
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn respects_capacity() {
        let tracker = LineageTracker::new(3);
        for i in 0..5u32 {
            tracker
                .record(format!("rec-{i}"), "etl", LineageStatus::Received, None)
                .await;
        }
        assert_eq!(tracker.snapshot().await.len(), 3);
    }
}
