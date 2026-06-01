//! Event stream processor with NATS (and optional Kafka) integration.
//!
//! Provides publish/subscribe over NATS subjects, with an in-process
//! fallback channel when the `nats` feature is disabled.

use crate::event_processing::schema::ProcessingEvent;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

#[cfg(feature = "nats")]
use async_nats;

const CHANNEL_CAPACITY: usize = 1_024;

/// Configuration for the stream processor
#[derive(Clone, Debug)]
pub struct StreamConfig {
    /// NATS server URL, e.g. "nats://localhost:4222"
    pub nats_url: String,
    /// Subject prefix for all events (e.g. "stellar.events")
    pub subject_prefix: String,
    /// Maximum in-flight messages before back-pressure
    pub buffer_size: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            nats_url: "nats://localhost:4222".into(),
            subject_prefix: "stellar.events".into(),
            buffer_size: CHANNEL_CAPACITY,
        }
    }
}

/// Subscriber handle returned by [`EventStreamProcessor::subscribe`]
pub struct EventSubscription {
    pub(crate) rx: broadcast::Receiver<ProcessingEvent>,
}

impl EventSubscription {
    /// Receive the next event (blocks until one arrives or the channel closes)
    pub async fn recv(&mut self) -> Option<ProcessingEvent> {
        loop {
            match self.rx.recv().await {
                Ok(ev) => return Some(ev),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("event subscription lagged, skipped {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

/// Core event stream processor.
///
/// When compiled with the `nats` feature the processor connects to a NATS
/// server and bridges the in-process broadcast channel to NATS subjects.
/// Without the feature it operates purely in-process.
pub struct EventStreamProcessor {
    config: StreamConfig,
    tx: broadcast::Sender<ProcessingEvent>,
    #[cfg(feature = "nats")]
    nats_client: Option<async_nats::Client>,
}

impl EventStreamProcessor {
    /// Create and (optionally) connect to NATS.
    pub async fn new(config: StreamConfig) -> crate::error::Result<Arc<Self>> {
        let (tx, _) = broadcast::channel(config.buffer_size);

        #[cfg(feature = "nats")]
        let nats_client = match async_nats::connect(&config.nats_url).await {
            Ok(c) => {
                info!("Connected to NATS at {}", config.nats_url);
                Some(c)
            }
            Err(e) => {
                warn!("NATS unavailable ({}), running in-process only", e);
                None
            }
        };

        Ok(Arc::new(Self {
            config,
            tx,
            #[cfg(feature = "nats")]
            nats_client,
        }))
    }

    /// Publish an event to all subscribers (and NATS if connected).
    pub async fn publish(&self, event: ProcessingEvent) -> crate::error::Result<()> {
        debug!("publishing event {} type={}", event.id, event.event_type);

        // Broadcast in-process
        let _ = self.tx.send(event.clone()); // ignore "no receivers" error

        // Forward to NATS
        #[cfg(feature = "nats")]
        if let Some(client) = &self.nats_client {
            let subject = format!("{}.{}", self.config.subject_prefix, event.event_type.replace('.', "_"));
            let payload = serde_json::to_vec(&event)
                .map_err(|e| crate::error::Error::SerializationError(e))?;
            if let Err(e) = client.publish(subject, payload.into()).await {
                error!("NATS publish error: {e}");
            }
        }

        Ok(())
    }

    /// Subscribe to all events (in-process broadcast).
    pub fn subscribe(&self) -> EventSubscription {
        EventSubscription { rx: self.tx.subscribe() }
    }

    /// Subscribe to events matching a type prefix (in-process filter).
    pub fn subscribe_filtered(&self, type_prefix: String) -> FilteredSubscription {
        FilteredSubscription {
            rx: self.tx.subscribe(),
            type_prefix,
        }
    }
}

/// A subscription that only yields events whose `event_type` starts with a prefix.
pub struct FilteredSubscription {
    rx: broadcast::Receiver<ProcessingEvent>,
    type_prefix: String,
}

impl FilteredSubscription {
    pub async fn recv(&mut self) -> Option<ProcessingEvent> {
        loop {
            match self.rx.recv().await {
                Ok(ev) if ev.event_type.starts_with(&self.type_prefix) => return Some(ev),
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_processing::schema::EventSource;

    #[tokio::test]
    async fn publish_and_receive() {
        let proc = EventStreamProcessor::new(StreamConfig::default()).await.unwrap();
        let mut sub = proc.subscribe();

        let ev = ProcessingEvent::new(
            "stellar.node.created",
            EventSource::Controller,
            "agg1",
            "default",
            serde_json::json!({}),
        );
        proc.publish(ev.clone()).await.unwrap();

        let received = sub.recv().await.unwrap();
        assert_eq!(received.event_type, "stellar.node.created");
    }

    #[tokio::test]
    async fn filtered_subscription() {
        let proc = EventStreamProcessor::new(StreamConfig::default()).await.unwrap();
        let mut sub = proc.subscribe_filtered("stellar.node".into());

        let ev = ProcessingEvent::new(
            "stellar.node.deleted",
            EventSource::Controller,
            "agg2",
            "default",
            serde_json::json!({}),
        );
        proc.publish(ev).await.unwrap();

        let received = sub.recv().await.unwrap();
        assert_eq!(received.event_type, "stellar.node.deleted");
    }
}
