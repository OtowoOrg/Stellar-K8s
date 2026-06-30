//! Event Bus for pub/sub event distribution

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

use crate::error::Result;
use super::event::DomainEvent;

/// Event subscriber trait
#[async_trait]
pub trait EventSubscriber: Send + Sync {
    /// Handle event
    async fn handle_event(&self, event: &DomainEvent) -> Result<()>;

    /// Get subscriber name
    fn name(&self) -> &str;
}

/// Event Bus for pub/sub
pub struct EventBus {
    subscribers: tokio::sync::RwLock<HashMap<String, Vec<Arc<dyn EventSubscriber>>>>,
}

impl EventBus {
    /// Create a new event bus
    pub fn new() -> Self {
        Self {
            subscribers: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Subscribe to events
    pub async fn subscribe(
        &self,
        event_type: String,
        subscriber: Arc<dyn EventSubscriber>,
    ) -> Result<()> {
        debug!(
            "Subscribing {} to event type: {}",
            subscriber.name(),
            event_type
        );

        let mut subscribers = self.subscribers.write().await;
        subscribers
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(subscriber);

        Ok(())
    }

    /// Unsubscribe from events
    pub async fn unsubscribe(&self, event_type: &str, subscriber_name: &str) -> Result<()> {
        debug!(
            "Unsubscribing {} from event type: {}",
            subscriber_name, event_type
        );

        let mut subscribers = self.subscribers.write().await;
        if let Some(subs) = subscribers.get_mut(event_type) {
            subs.retain(|s| s.name() != subscriber_name);
        }

        Ok(())
    }

    /// Publish event
    pub async fn publish(&self, event: DomainEvent) -> Result<()> {
        debug!("Publishing event: {}", event.event_type);

        let subscribers = self.subscribers.read().await;

        // Get subscribers for this event type
        if let Some(subs) = subscribers.get(&event.event_type) {
            for subscriber in subs {
                if let Err(e) = subscriber.handle_event(&event).await {
                    // Log error but continue with other subscribers
                    tracing::warn!(
                        "Error handling event in {}: {}",
                        subscriber.name(),
                        e
                    );
                }
            }
        }

        // Also publish to wildcard subscribers
        if let Some(subs) = subscribers.get("*") {
            for subscriber in subs {
                if let Err(e) = subscriber.handle_event(&event).await {
                    tracing::warn!(
                        "Error handling event in {}: {}",
                        subscriber.name(),
                        e
                    );
                }
            }
        }

        Ok(())
    }

    /// Get subscriber count for event type
    pub async fn get_subscriber_count(&self, event_type: &str) -> Result<usize> {
        let subscribers = self.subscribers.read().await;
        Ok(subscribers.get(event_type).map(|v| v.len()).unwrap_or(0))
    }

    /// Get all subscriptions
    pub async fn get_subscriptions(&self) -> Result<HashMap<String, usize>> {
        let subscribers = self.subscribers.read().await;
        Ok(subscribers
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect())
    }

    /// Clear all subscriptions
    pub async fn clear_subscriptions(&self) -> Result<()> {
        debug!("Clearing all event subscriptions");
        let mut subscribers = self.subscribers.write().await;
        subscribers.clear();
        Ok(())
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSubscriber {
        name: String,
    }

    #[async_trait]
    impl EventSubscriber for TestSubscriber {
        async fn handle_event(&self, _event: &DomainEvent) -> Result<()> {
            Ok(())
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    #[tokio::test]
    async fn test_event_bus_creation() {
        let bus = EventBus::new();
        let subs = bus.get_subscriptions().await.unwrap();
        assert_eq!(subs.len(), 0);
    }

    #[tokio::test]
    async fn test_subscribe() {
        let bus = EventBus::new();
        let subscriber = Arc::new(TestSubscriber {
            name: "test".to_string(),
        });

        bus.subscribe("TestEvent".to_string(), subscriber)
            .await
            .unwrap();

        let count = bus.get_subscriber_count("TestEvent").await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_publish() {
        let bus = EventBus::new();
        let subscriber = Arc::new(TestSubscriber {
            name: "test".to_string(),
        });

        bus.subscribe("TestEvent".to_string(), subscriber)
            .await
            .unwrap();

        let event = DomainEvent::builder(
            "agg123".to_string(),
            "Test".to_string(),
            "user@example.com".to_string(),
        )
        .event_type("TestEvent")
        .build();

        bus.publish(event).await.unwrap();
    }
}
