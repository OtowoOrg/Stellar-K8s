//! Sophisticated event processing system for Stellar-K8s.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │  EventProcessingSystem                                        │
//! │                                                               │
//! │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────┐  │
//! │  │ Stream   │  │  CEP     │  │Correlation│  │ Automation │  │
//! │  │Processor │→ │ Engine   │  │  Engine   │  │ Executor   │  │
//! │  └──────────┘  └──────────┘  └──────────┘  └────────────┘  │
//! │        │                                                      │
//! │  ┌─────▼────┐  ┌──────────┐  ┌──────────┐                  │
//! │  │  Replay  │  │Analytics │  │ Catalog  │                   │
//! │  │  Store   │  │ Engine   │  │(Registry)│                   │
//! │  └──────────┘  └──────────┘  └──────────┘                  │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Quick start
//!
//! ```rust,no_run
//! use stellar_k8s::event_processing::{EventProcessingSystem, EventProcessingConfig};
//!
//! #[tokio::main]
//! async fn main() {
//!     let system = EventProcessingSystem::new(EventProcessingConfig::default()).await.unwrap();
//!     system.start().await.unwrap();
//! }
//! ```

pub mod analytics;
pub mod automation;
pub mod catalog;
pub mod cep;
pub mod correlation;
pub mod replay;
pub mod schema;
pub mod stream;

pub use analytics::AnalyticsEngine;
pub use automation::{AutomationAction, AutomationExecutor, AutomationRule};
pub use catalog::EventCatalog;
pub use cep::{CepEngine, CepPattern, PatternKind, PatternMatch};
pub use correlation::{CorrelatedGroup, CorrelationConfig, CorrelationEngine, CorrelationStrategy};
pub use replay::{EventReplayStore, ReplayOptions};
pub use schema::{EventSchema, EventSeverity, EventSource, ProcessingEvent, SchemaVersion};
pub use stream::{EventStreamProcessor, StreamConfig};

use std::sync::Arc;
use tracing::info;

/// Top-level configuration
#[derive(Clone, Debug, Default)]
pub struct EventProcessingConfig {
    pub stream: StreamConfig,
    pub correlation: CorrelationConfig,
    /// Replay buffer capacity (number of events)
    pub replay_capacity: usize,
}

impl EventProcessingConfig {
    pub fn with_replay_capacity(mut self, cap: usize) -> Self {
        self.replay_capacity = cap;
        self
    }
}

/// The unified event processing system.
///
/// Owns all sub-systems and provides a single `publish` entry point that
/// fans out to every component.
pub struct EventProcessingSystem {
    pub stream: Arc<EventStreamProcessor>,
    pub cep: Arc<CepEngine>,
    pub correlation: Arc<CorrelationEngine>,
    pub automation: Arc<AutomationExecutor>,
    pub replay_store: Arc<EventReplayStore>,
    pub analytics: Arc<AnalyticsEngine>,
    pub catalog: Arc<EventCatalog>,
}

impl EventProcessingSystem {
    /// Create a new system. Does not start background tasks.
    pub async fn new(config: EventProcessingConfig) -> crate::error::Result<Arc<Self>> {
        let replay_capacity = if config.replay_capacity == 0 { 10_000 } else { config.replay_capacity };

        let stream = EventStreamProcessor::new(config.stream).await?;
        let cep = CepEngine::new();
        let correlation = CorrelationEngine::new(config.correlation);
        let analytics = AnalyticsEngine::new();
        let replay_store = EventReplayStore::new(replay_capacity);
        let catalog = EventCatalog::new();

        // Wire automation emit_fn → stream processor
        let stream_clone = stream.clone();
        let emit_fn: automation::EmitFn = Arc::new(move |ev| {
            let s = stream_clone.clone();
            tokio::spawn(async move {
                let _ = s.publish(ev).await;
            });
        });
        let automation = AutomationExecutor::new(Some(emit_fn));

        info!("EventProcessingSystem initialized");
        Ok(Arc::new(Self {
            stream,
            cep,
            correlation,
            automation,
            replay_store,
            analytics,
            catalog,
        }))
    }

    /// Publish an event through the entire pipeline.
    pub async fn publish(&self, event: ProcessingEvent) -> crate::error::Result<()> {
        let start = std::time::Instant::now();

        // Validate against catalog
        let validation = self.catalog.validate(&event).await;
        if !validation.valid {
            tracing::warn!(
                "event '{}' failed schema validation: {:?}",
                event.event_type,
                validation.errors
            );
        }

        // Fan out to all sub-systems concurrently
        let latency_us = start.elapsed().as_micros() as u64;
        self.analytics.record(&event, latency_us).await;
        self.replay_store.record(event.clone()).await;
        self.cep.process(&event).await;
        self.correlation.process(event.clone()).await;
        self.automation.process(&event).await;
        self.stream.publish(event).await?;

        Ok(())
    }

    /// Start background tasks (e.g. periodic correlation flush).
    pub async fn start(self: &Arc<Self>) -> crate::error::Result<()> {
        let correlation = self.correlation.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                correlation.flush().await;
            }
        });
        info!("EventProcessingSystem background tasks started");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn system_publishes_and_records() {
        let system = EventProcessingSystem::new(EventProcessingConfig {
            replay_capacity: 100,
            ..Default::default()
        })
        .await
        .unwrap();

        let ev = ProcessingEvent::new(
            "stellar.node.created",
            EventSource::Controller,
            "validator-1",
            "stellar",
            serde_json::json!({"name": "validator-1", "namespace": "stellar", "node_type": "Validator"}),
        );

        system.publish(ev).await.unwrap();

        assert_eq!(system.replay_store.len().await, 1);
        let snap = system.analytics.snapshot().await;
        assert_eq!(snap.total_events, 1);
    }

    #[tokio::test]
    async fn cep_pattern_detected_through_system() {
        use crate::event_processing::cep::{CepPattern, EventCondition, PatternKind};
        use std::collections::HashMap;

        let system = EventProcessingSystem::new(EventProcessingConfig::default())
            .await
            .unwrap();

        let mut rx = system.cep.subscribe_matches();

        system
            .cep
            .register_pattern(CepPattern {
                id: "test-threshold".into(),
                name: "3x disk warning".into(),
                kind: PatternKind::Threshold {
                    condition: EventCondition {
                        event_type_prefix: "stellar.disk".into(),
                        source_filter: None,
                        min_severity: None,
                        required_labels: HashMap::new(),
                    },
                    count: 3,
                },
                conditions: vec![],
                window_secs: 60,
                description: "disk warning 3 times".into(),
            })
            .await;

        for _ in 0..3 {
            let ev = ProcessingEvent::new(
                "stellar.disk.warning",
                EventSource::DiskScaler,
                "validator-1",
                "stellar",
                serde_json::json!({"usage_percent": 90}),
            );
            system.publish(ev).await.unwrap();
        }

        let m = rx.try_recv().unwrap();
        assert_eq!(m.pattern_id, "test-threshold");
    }
}
