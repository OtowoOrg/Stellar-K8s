//! Event Store - Append-only log of domain events

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use tracing::{debug, info};

use crate::error::Result;
use super::event::DomainEvent;

/// Event store configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EventStoreConfig {
    /// Maximum events per aggregate before requiring snapshot
    pub max_events_per_aggregate: usize,
    /// Enable event compression
    pub enable_compression: bool,
    /// Retention period in days (0 = unlimited)
    pub retention_days: u32,
}

impl Default for EventStoreConfig {
    fn default() -> Self {
        Self {
            max_events_per_aggregate: 10_000,
            enable_compression: true,
            retention_days: 0,
        }
    }
}

/// Event Store - Append-only log
pub struct EventStore {
    config: EventStoreConfig,
    // In-memory storage (in production, use database)
    events: tokio::sync::RwLock<Vec<StoredEvent>>,
    // Index by aggregate ID for fast lookup
    aggregate_index: tokio::sync::RwLock<HashMap<String, Vec<usize>>>,
}

/// Stored event with metadata
#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredEvent {
    event: DomainEvent,
    stored_at: DateTime<Utc>,
    sequence_number: u64,
}

impl EventStore {
    /// Create a new event store
    pub async fn new(config: EventStoreConfig) -> Result<Self> {
        debug!("Initializing Event Store");
        Ok(Self {
            config,
            events: tokio::sync::RwLock::new(Vec::new()),
            aggregate_index: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Append event to store
    pub async fn append(&self, event: DomainEvent) -> Result<u64> {
        let mut events = self.events.write().await;
        let sequence_number = events.len() as u64 + 1;

        let stored_event = StoredEvent {
            event: event.clone(),
            stored_at: Utc::now(),
            sequence_number,
        };

        events.push(stored_event);

        // Update index
        let mut index = self.aggregate_index.write().await;
        index
            .entry(event.metadata.aggregate_id.clone())
            .or_insert_with(Vec::new)
            .push(events.len() - 1);

        debug!(
            "Appended event {} for aggregate {}",
            event.event_type, event.metadata.aggregate_id
        );

        Ok(sequence_number)
    }

    /// Get events for aggregate
    pub async fn get_events(&self, aggregate_id: &str) -> Result<Vec<DomainEvent>> {
        let events = self.events.read().await;
        let index = self.aggregate_index.read().await;

        if let Some(indices) = index.get(aggregate_id) {
            let result = indices
                .iter()
                .filter_map(|&idx| events.get(idx).map(|se| se.event.clone()))
                .collect();
            Ok(result)
        } else {
            Ok(Vec::new())
        }
    }

    /// Get events since sequence number
    pub async fn get_events_since(&self, aggregate_id: &str, since: u64) -> Result<Vec<DomainEvent>> {
        let events = self.events.read().await;
        let index = self.aggregate_index.read().await;

        if let Some(indices) = index.get(aggregate_id) {
            let result = indices
                .iter()
                .filter_map(|&idx| {
                    events.get(idx).and_then(|se| {
                        if se.sequence_number > since {
                            Some(se.event.clone())
                        } else {
                            None
                        }
                    })
                })
                .collect();
            Ok(result)
        } else {
            Ok(Vec::new())
        }
    }

    /// Get all events
    pub async fn get_all_events(&self) -> Result<Vec<DomainEvent>> {
        let events = self.events.read().await;
        Ok(events.iter().map(|se| se.event.clone()).collect())
    }

    /// Get events by type
    pub async fn get_events_by_type(&self, event_type: &str) -> Result<Vec<DomainEvent>> {
        let events = self.events.read().await;
        Ok(events
            .iter()
            .filter(|se| se.event.event_type == event_type)
            .map(|se| se.event.clone())
            .collect())
    }

    /// Get events in time range
    pub async fn get_events_in_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<DomainEvent>> {
        let events = self.events.read().await;
        Ok(events
            .iter()
            .filter(|se| se.stored_at >= start && se.stored_at <= end)
            .map(|se| se.event.clone())
            .collect())
    }

    /// Get event count for aggregate
    pub async fn get_event_count(&self, aggregate_id: &str) -> Result<usize> {
        let index = self.aggregate_index.read().await;
        Ok(index.get(aggregate_id).map(|v| v.len()).unwrap_or(0))
    }

    /// Get total event count
    pub async fn get_total_event_count(&self) -> Result<usize> {
        let events = self.events.read().await;
        Ok(events.len())
    }

    /// Get store statistics
    pub async fn get_statistics(&self) -> Result<EventStoreStatistics> {
        let events = self.events.read().await;
        let index = self.aggregate_index.read().await;

        let total_events = events.len();
        let total_aggregates = index.len();
        let avg_events_per_aggregate = if total_aggregates > 0 {
            total_events / total_aggregates
        } else {
            0
        };

        let event_types: std::collections::HashSet<_> =
            events.iter().map(|e| e.event.event_type.clone()).collect();

        Ok(EventStoreStatistics {
            total_events,
            total_aggregates,
            avg_events_per_aggregate,
            unique_event_types: event_types.len(),
            timestamp: Utc::now(),
        })
    }

    /// Compact events (remove old events, keep snapshots)
    pub async fn compact(&self, keep_days: u32) -> Result<usize> {
        debug!("Compacting event store (keep {} days)", keep_days);

        let mut events = self.events.write().await;
        let initial_count = events.len();

        let cutoff = Utc::now() - chrono::Duration::days(keep_days as i64);
        events.retain(|e| e.stored_at > cutoff);

        let removed = initial_count - events.len();
        info!("Compacted event store: removed {} events", removed);

        Ok(removed)
    }
}

/// Event store statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventStoreStatistics {
    pub total_events: usize,
    pub total_aggregates: usize,
    pub avg_events_per_aggregate: usize,
    pub unique_event_types: usize,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_store_creation() {
        let config = EventStoreConfig::default();
        let store = EventStore::new(config).await.unwrap();
        
        let stats = store.get_statistics().await.unwrap();
        assert_eq!(stats.total_events, 0);
    }

    #[tokio::test]
    async fn test_append_event() {
        let config = EventStoreConfig::default();
        let store = EventStore::new(config).await.unwrap();

        let event = DomainEvent::builder(
            "agg123".to_string(),
            "StellarNode".to_string(),
            "user@example.com".to_string(),
        )
        .event_type("NodeCreated")
        .payload(serde_json::json!({"name": "validator-1"}))
        .build();

        let seq = store.append(event).await.unwrap();
        assert_eq!(seq, 1);
    }

    #[tokio::test]
    async fn test_get_events() {
        let config = EventStoreConfig::default();
        let store = EventStore::new(config).await.unwrap();

        let event = DomainEvent::builder(
            "agg123".to_string(),
            "StellarNode".to_string(),
            "user@example.com".to_string(),
        )
        .event_type("NodeCreated")
        .payload(serde_json::json!({"name": "validator-1"}))
        .build();

        store.append(event).await.unwrap();

        let events = store.get_events("agg123").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "NodeCreated");
    }
}
