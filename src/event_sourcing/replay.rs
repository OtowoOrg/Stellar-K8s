//! Event Replay Engine for audit and recovery

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tracing::{debug, info};

use crate::error::Result;
use super::event::DomainEvent;
use super::event_store::EventStore;

/// Replay options
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayOptions {
    /// Start from this sequence number
    pub from_sequence: Option<u64>,
    /// Replay until this sequence number
    pub to_sequence: Option<u64>,
    /// Only replay events of this type
    pub event_type_filter: Option<String>,
    /// Only replay events for this aggregate
    pub aggregate_id_filter: Option<String>,
    /// Replay events in this time range
    pub time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
}

impl Default for ReplayOptions {
    fn default() -> Self {
        Self {
            from_sequence: None,
            to_sequence: None,
            event_type_filter: None,
            aggregate_id_filter: None,
            time_range: None,
        }
    }
}

/// Event Replay Engine
pub struct EventReplayEngine {
    event_store: Arc<EventStore>,
}

impl EventReplayEngine {
    /// Create a new replay engine
    pub fn new(event_store: Arc<EventStore>) -> Self {
        Self { event_store }
    }

    /// Replay events
    pub async fn replay(&self, options: ReplayOptions) -> Result<ReplayResult> {
        debug!("Starting event replay with options: {:?}", options);

        let all_events = self.event_store.get_all_events().await?;

        let mut replayed_events = Vec::new();
        let mut state_snapshots = std::collections::HashMap::new();

        for event in all_events {
            // Apply filters
            if let Some(ref filter) = options.event_type_filter {
                if event.event_type != *filter {
                    continue;
                }
            }

            if let Some(ref filter) = options.aggregate_id_filter {
                if event.metadata.aggregate_id != *filter {
                    continue;
                }
            }

            if let Some((start, end)) = options.time_range {
                if event.metadata.timestamp < start || event.metadata.timestamp > end {
                    continue;
                }
            }

            replayed_events.push(event.clone());

            // Build state snapshot
            let agg_id = &event.metadata.aggregate_id;
            let state = state_snapshots
                .entry(agg_id.clone())
                .or_insert_with(|| serde_json::json!({}));

            *state = apply_event_to_state(state, &event)?;
        }

        info!(
            "Replay completed: {} events replayed, {} aggregates",
            replayed_events.len(),
            state_snapshots.len()
        );

        Ok(ReplayResult {
            events_replayed: replayed_events.len(),
            aggregates_affected: state_snapshots.len(),
            final_states: state_snapshots,
            timestamp: Utc::now(),
        })
    }

    /// Replay events for specific aggregate
    pub async fn replay_aggregate(&self, aggregate_id: &str) -> Result<ReplayResult> {
        let options = ReplayOptions {
            aggregate_id_filter: Some(aggregate_id.to_string()),
            ..Default::default()
        };

        self.replay(options).await
    }

    /// Replay events since timestamp
    pub async fn replay_since(&self, since: DateTime<Utc>) -> Result<ReplayResult> {
        let now = Utc::now();
        let options = ReplayOptions {
            time_range: Some((since, now)),
            ..Default::default()
        };

        self.replay(options).await
    }

    /// Verify event consistency
    pub async fn verify_consistency(&self) -> Result<ConsistencyReport> {
        debug!("Verifying event store consistency");

        let all_events = self.event_store.get_all_events().await?;

        let mut aggregates = std::collections::HashMap::new();
        let mut issues = Vec::new();

        for event in all_events {
            let agg_id = &event.metadata.aggregate_id;
            let agg_events = aggregates
                .entry(agg_id.clone())
                .or_insert_with(Vec::new);

            agg_events.push(event);
        }

        // Check for consistency issues
        for (agg_id, events) in aggregates.iter() {
            // Check for duplicate event IDs
            let mut event_ids = std::collections::HashSet::new();
            for event in events {
                if !event_ids.insert(&event.metadata.event_id) {
                    issues.push(format!("Duplicate event ID in aggregate {}", agg_id));
                }
            }

            // Check for timestamp ordering
            for i in 1..events.len() {
                if events[i].metadata.timestamp < events[i - 1].metadata.timestamp {
                    issues.push(format!(
                        "Out-of-order timestamps in aggregate {}",
                        agg_id
                    ));
                }
            }
        }

        Ok(ConsistencyReport {
            total_aggregates: aggregates.len(),
            total_events: all_events.len(),
            issues,
            timestamp: Utc::now(),
        })
    }
}

/// Apply event to state
fn apply_event_to_state(state: &serde_json::Value, event: &DomainEvent) -> Result<serde_json::Value> {
    let mut new_state = state.clone();

    if let Some(obj) = new_state.as_object_mut() {
        obj.insert("last_event".to_string(), serde_json::json!(event.event_type));
        obj.insert("last_event_at".to_string(), serde_json::json!(event.metadata.timestamp));
        
        // Merge event payload into state
        if let Some(payload_obj) = event.payload.as_object() {
            for (key, value) in payload_obj {
                obj.insert(key.clone(), value.clone());
            }
        }
    }

    Ok(new_state)
}

/// Replay result
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayResult {
    pub events_replayed: usize,
    pub aggregates_affected: usize,
    pub final_states: std::collections::HashMap<String, serde_json::Value>,
    pub timestamp: DateTime<Utc>,
}

/// Consistency report
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsistencyReport {
    pub total_aggregates: usize,
    pub total_events: usize,
    pub issues: Vec<String>,
    pub timestamp: DateTime<Utc>,
}

impl ConsistencyReport {
    pub fn is_consistent(&self) -> bool {
        self.issues.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_replay_options_default() {
        let options = ReplayOptions::default();
        assert!(options.from_sequence.is_none());
        assert!(options.event_type_filter.is_none());
    }
}
