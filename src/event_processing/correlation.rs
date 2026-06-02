//! Event correlation engine.
//!
//! Groups related events from multiple sources into [`CorrelatedGroup`]s
//! using correlation IDs, aggregate IDs, or custom correlation keys.

use crate::event_processing::schema::ProcessingEvent;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info};

/// Strategy used to correlate events
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CorrelationStrategy {
    /// Group by the event's `correlation_id` field
    ByCorrelationId,
    /// Group by `aggregate_id`
    ByAggregateId,
    /// Group by a specific label value
    ByLabel(String),
}

/// Configuration for the correlation engine
#[derive(Clone, Debug)]
pub struct CorrelationConfig {
    pub strategy: CorrelationStrategy,
    /// How long to keep an open group before closing it (seconds)
    pub window_secs: u64,
    /// Minimum events to form a group worth emitting
    pub min_group_size: usize,
}

impl Default for CorrelationConfig {
    fn default() -> Self {
        Self {
            strategy: CorrelationStrategy::ByCorrelationId,
            window_secs: 30,
            min_group_size: 2,
        }
    }
}

/// A group of correlated events
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CorrelatedGroup {
    pub group_key: String,
    pub events: Vec<ProcessingEvent>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// Distinct sources that contributed events
    pub sources: Vec<String>,
}

impl CorrelatedGroup {
    fn new(key: String, first_event: ProcessingEvent) -> Self {
        let src = first_event.source.to_string();
        let ts = first_event.timestamp;
        Self {
            group_key: key,
            events: vec![first_event],
            first_seen: ts,
            last_seen: ts,
            sources: vec![src],
        }
    }

    fn add(&mut self, event: ProcessingEvent) {
        let src = event.source.to_string();
        self.last_seen = event.timestamp;
        if !self.sources.contains(&src) {
            self.sources.push(src);
        }
        self.events.push(event);
    }
}

struct OpenGroup {
    group: CorrelatedGroup,
    expires_at: DateTime<Utc>,
}

/// The correlation engine
pub struct CorrelationEngine {
    config: CorrelationConfig,
    open_groups: RwLock<HashMap<String, OpenGroup>>,
    group_tx: broadcast::Sender<CorrelatedGroup>,
}

impl CorrelationEngine {
    pub fn new(config: CorrelationConfig) -> Arc<Self> {
        let (group_tx, _) = broadcast::channel(256);
        Arc::new(Self {
            config,
            open_groups: RwLock::new(HashMap::new()),
            group_tx,
        })
    }

    fn correlation_key(&self, event: &ProcessingEvent) -> String {
        match &self.config.strategy {
            CorrelationStrategy::ByCorrelationId => event.correlation_id.clone(),
            CorrelationStrategy::ByAggregateId => event.aggregate_id.clone(),
            CorrelationStrategy::ByLabel(label) => {
                event.labels.get(label).cloned().unwrap_or_else(|| event.id.clone())
            }
        }
    }

    /// Feed an event into the engine. Emits completed groups when they expire.
    pub async fn process(&self, event: ProcessingEvent) {
        let key = self.correlation_key(&event);
        let now = Utc::now();
        let expires_at = now + Duration::seconds(self.config.window_secs as i64);

        let mut groups = self.open_groups.write().await;

        // Evict expired groups first
        let expired_keys: Vec<_> = groups
            .iter()
            .filter(|(_, og)| og.expires_at < now)
            .map(|(k, _)| k.clone())
            .collect();
        for k in expired_keys {
            if let Some(og) = groups.remove(&k) {
                if og.group.events.len() >= self.config.min_group_size {
                    debug!("correlation: emitting group '{}' ({} events)", k, og.group.events.len());
                    let _ = self.group_tx.send(og.group);
                }
            }
        }

        // Add event to existing or new group
        if let Some(og) = groups.get_mut(&key) {
            og.group.add(event);
            og.expires_at = expires_at; // extend window on activity
        } else {
            groups.insert(
                key.clone(),
                OpenGroup {
                    group: CorrelatedGroup::new(key, event),
                    expires_at,
                },
            );
        }
    }

    /// Flush all open groups immediately (useful for shutdown / testing).
    pub async fn flush(&self) {
        let mut groups = self.open_groups.write().await;
        for (_, og) in groups.drain() {
            if og.group.events.len() >= self.config.min_group_size {
                let _ = self.group_tx.send(og.group);
            }
        }
    }

    /// Subscribe to completed correlated groups.
    pub fn subscribe_groups(&self) -> broadcast::Receiver<CorrelatedGroup> {
        self.group_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_processing::schema::EventSource;

    fn ev(corr: &str) -> ProcessingEvent {
        ProcessingEvent::new(
            "stellar.test",
            EventSource::Controller,
            "agg",
            "ns",
            serde_json::json!({}),
        )
        .with_correlation_id(corr)
    }

    #[tokio::test]
    async fn groups_correlated_events() {
        let engine = CorrelationEngine::new(CorrelationConfig {
            strategy: CorrelationStrategy::ByCorrelationId,
            window_secs: 1,
            min_group_size: 2,
        });
        let mut rx = engine.subscribe_groups();

        let corr = "trace-abc";
        engine.process(ev(corr)).await;
        engine.process(ev(corr)).await;
        engine.flush().await;

        let group = rx.try_recv().unwrap();
        assert_eq!(group.group_key, corr);
        assert_eq!(group.events.len(), 2);
    }
}
