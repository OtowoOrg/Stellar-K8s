//! Data lineage tracking and architecture documentation

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

/// A single lineage event recording how a record was processed
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineageEvent {
    pub record_sequence: u64,
    pub stage: String,
    pub action: String,
    pub source: String,
    pub destination: String,
    pub transform_applied: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

/// Data lineage tracker — records the full processing history of ledger records
pub struct DataLineage {
    events: Arc<RwLock<VecDeque<LineageEvent>>>,
    max_events: usize,
}

impl DataLineage {
    pub fn new(max_events: usize) -> Self {
        Self { events: Arc::new(RwLock::new(VecDeque::new())), max_events }
    }

    pub async fn record(&self, event: LineageEvent) {
        let mut e = self.events.write().await;
        if e.len() >= self.max_events {
            e.pop_front();
        }
        e.push_back(event);
    }

    pub async fn events_for_ledger(&self, sequence: u64) -> Vec<LineageEvent> {
        self.events
            .read()
            .await
            .iter()
            .filter(|e| e.record_sequence == sequence)
            .cloned()
            .collect()
    }

    pub async fn total_events(&self) -> usize {
        self.events.read().await.len()
    }

    /// Export lineage as JSON for external consumption
    pub async fn export_json(&self) -> serde_json::Value {
        let events: Vec<LineageEvent> = self.events.read().await.iter().cloned().collect();
        serde_json::json!({ "lineage_events": events, "exported_at": Utc::now() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lineage_record_and_query() {
        let lineage = DataLineage::new(100);
        lineage
            .record(LineageEvent {
                record_sequence: 42,
                stage: "etl".into(),
                action: "transform".into(),
                source: "ingestion".into(),
                destination: "quality".into(),
                transform_applied: Some("normalize_fees".into()),
                timestamp: Utc::now(),
                metadata: serde_json::Value::Null,
            })
            .await;

        let events = lineage.events_for_ledger(42).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].stage, "etl");
    }
}
