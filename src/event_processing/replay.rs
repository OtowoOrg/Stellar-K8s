//! Event replay and debugging capabilities.
//!
//! Stores events in an in-memory ring buffer and allows replaying them
//! through the processing pipeline for debugging and root-cause analysis.

use crate::event_processing::schema::ProcessingEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Options controlling a replay run
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReplayOptions {
    /// Only replay events after this timestamp
    pub from: Option<DateTime<Utc>>,
    /// Only replay events before this timestamp
    pub until: Option<DateTime<Utc>>,
    /// Only replay events whose type starts with this prefix
    pub event_type_filter: Option<String>,
    /// Only replay events for this aggregate_id
    pub aggregate_filter: Option<String>,
    /// Maximum number of events to replay
    pub limit: Option<usize>,
}

/// A recorded event with replay metadata
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordedEvent {
    pub event: ProcessingEvent,
    pub recorded_at: DateTime<Utc>,
    pub sequence: u64,
}

/// Debug snapshot: a point-in-time view of the event store
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DebugSnapshot {
    pub taken_at: DateTime<Utc>,
    pub total_recorded: u64,
    pub events: Vec<RecordedEvent>,
    pub filter: ReplayOptions,
}

/// In-memory event store with replay support
pub struct EventReplayStore {
    buffer: RwLock<VecDeque<RecordedEvent>>,
    capacity: usize,
    sequence: RwLock<u64>,
}

impl EventReplayStore {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            buffer: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
            sequence: RwLock::new(0),
        })
    }

    /// Record an event into the ring buffer.
    pub async fn record(&self, event: ProcessingEvent) {
        let mut buf = self.buffer.write().await;
        let mut seq = self.sequence.write().await;
        *seq += 1;
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(RecordedEvent {
            event,
            recorded_at: Utc::now(),
            sequence: *seq,
        });
    }

    /// Replay events matching the given options, calling `handler` for each.
    pub async fn replay<F, Fut>(&self, opts: ReplayOptions, handler: F) -> usize
    where
        F: Fn(RecordedEvent) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let buf = self.buffer.read().await;
        let mut count = 0;

        for rec in buf.iter() {
            if let Some(from) = opts.from {
                if rec.event.timestamp < from {
                    continue;
                }
            }
            if let Some(until) = opts.until {
                if rec.event.timestamp > until {
                    continue;
                }
            }
            if let Some(ref prefix) = opts.event_type_filter {
                if !rec.event.event_type.starts_with(prefix.as_str()) {
                    continue;
                }
            }
            if let Some(ref agg) = opts.aggregate_filter {
                if rec.event.aggregate_id != *agg {
                    continue;
                }
            }
            if let Some(limit) = opts.limit {
                if count >= limit {
                    break;
                }
            }
            handler(rec.clone()).await;
            count += 1;
        }

        info!("replay: replayed {count} events");
        count
    }

    /// Take a debug snapshot of the current buffer.
    pub async fn snapshot(&self, filter: ReplayOptions) -> DebugSnapshot {
        let buf = self.buffer.read().await;
        let seq = *self.sequence.read().await;

        let events: Vec<_> = buf
            .iter()
            .filter(|rec| {
                if let Some(ref prefix) = filter.event_type_filter {
                    rec.event.event_type.starts_with(prefix.as_str())
                } else {
                    true
                }
            })
            .cloned()
            .collect();

        DebugSnapshot {
            taken_at: Utc::now(),
            total_recorded: seq,
            events,
            filter,
        }
    }

    /// Return the number of events currently in the buffer.
    pub async fn len(&self) -> usize {
        self.buffer.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_processing::schema::EventSource;

    fn ev(t: &str) -> ProcessingEvent {
        ProcessingEvent::new(t, EventSource::Controller, "agg", "ns", serde_json::json!({}))
    }

    #[tokio::test]
    async fn record_and_replay() {
        let store = EventReplayStore::new(100);
        store.record(ev("stellar.node.created")).await;
        store.record(ev("stellar.disk.warning")).await;
        store.record(ev("stellar.node.deleted")).await;

        let mut replayed = vec![];
        store
            .replay(
                ReplayOptions {
                    event_type_filter: Some("stellar.node".into()),
                    ..Default::default()
                },
                |rec| {
                    replayed.push(rec.event.event_type.clone());
                    async {}
                },
            )
            .await;

        assert_eq!(replayed, vec!["stellar.node.created", "stellar.node.deleted"]);
    }

    #[tokio::test]
    async fn ring_buffer_evicts_oldest() {
        let store = EventReplayStore::new(2);
        store.record(ev("a")).await;
        store.record(ev("b")).await;
        store.record(ev("c")).await; // evicts "a"
        assert_eq!(store.len().await, 2);
    }
}
