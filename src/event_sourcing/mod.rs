//! Advanced Event Sourcing System with CQRS Pattern
//!
//! Provides comprehensive event sourcing and Command Query Responsibility Segregation (CQRS)
//! implementation for audit trails, state reconstruction, and event replay.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  Event Sourcing & CQRS System                            │
//! ├─────────────────────────────────────────────────────────┤
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐   │
//! │  │ Command      │  │ Event Store  │  │ Projections  │   │
//! │  │ Handler      │  │ (Append-only)│  │ (Read Model) │   │
//! │  └──────────────┘  └──────────────┘  └──────────────┘   │
//! │         │                 │                 │             │
//! │         └─────────────────┴─────────────────┘             │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Event Bus               │                      │
//! │         │ (Pub/Sub)               │                      │
//! │         └────────────┬────────────┘                      │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Snapshot Manager        │                      │
//! │         │ (Performance)           │                      │
//! │         └────────────┬────────────┘                      │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Event Replay Engine     │                      │
//! │         │ (Audit & Recovery)      │                      │
//! │         └────────────────────────┘                      │
//! └─────────────────────────────────────────────────────────┘
//! ```

pub mod command;
pub mod event;
pub mod event_store;
pub mod projection;
pub mod snapshot;
pub mod replay;
pub mod bus;

pub use command::{Command, CommandHandler, CommandResult};
pub use event::{DomainEvent, EventMetadata, EventVersion};
pub use event_store::{EventStore, EventStoreConfig};
pub use projection::{Projection, ProjectionManager};
pub use snapshot::{Snapshot, SnapshotManager};
pub use replay::{EventReplayEngine, ReplayOptions};
pub use bus::{EventBus, EventSubscriber};

use std::sync::Arc;
use tracing::info;

/// Event Sourcing System Configuration
#[derive(Clone, Debug)]
pub struct EventSourcingConfig {
    pub event_store_config: event_store::EventStoreConfig,
    pub snapshot_config: snapshot::SnapshotConfig,
    pub projection_config: projection::ProjectionConfig,
}

impl Default for EventSourcingConfig {
    fn default() -> Self {
        Self {
            event_store_config: Default::default(),
            snapshot_config: Default::default(),
            projection_config: Default::default(),
        }
    }
}

/// Main Event Sourcing System
pub struct EventSourcingSystem {
    /// Event store for persisting events
    event_store: Arc<EventStore>,
    /// Event bus for pub/sub
    event_bus: Arc<EventBus>,
    /// Projection manager for read models
    projection_manager: Arc<ProjectionManager>,
    /// Snapshot manager for performance
    snapshot_manager: Arc<SnapshotManager>,
    /// Event replay engine
    replay_engine: Arc<EventReplayEngine>,
}

impl EventSourcingSystem {
    /// Create a new event sourcing system
    pub async fn new(config: EventSourcingConfig) -> crate::error::Result<Self> {
        info!("Initializing Event Sourcing System");

        let event_store = Arc::new(EventStore::new(config.event_store_config).await?);
        let event_bus = Arc::new(EventBus::new());
        let projection_manager = Arc::new(ProjectionManager::new(config.projection_config).await?);
        let snapshot_manager = Arc::new(SnapshotManager::new(config.snapshot_config).await?);
        let replay_engine = Arc::new(EventReplayEngine::new(event_store.clone()));

        Ok(Self {
            event_store,
            event_bus,
            projection_manager,
            snapshot_manager,
            replay_engine,
        })
    }

    /// Get event store
    pub fn event_store(&self) -> Arc<EventStore> {
        self.event_store.clone()
    }

    /// Get event bus
    pub fn event_bus(&self) -> Arc<EventBus> {
        self.event_bus.clone()
    }

    /// Get projection manager
    pub fn projection_manager(&self) -> Arc<ProjectionManager> {
        self.projection_manager.clone()
    }

    /// Get snapshot manager
    pub fn snapshot_manager(&self) -> Arc<SnapshotManager> {
        self.snapshot_manager.clone()
    }

    /// Get replay engine
    pub fn replay_engine(&self) -> Arc<EventReplayEngine> {
        self.replay_engine.clone()
    }

    /// Append event to store and publish to bus
    pub async fn append_event(&self, event: DomainEvent) -> crate::error::Result<()> {
        // Store event
        self.event_store.append(event.clone()).await?;

        // Publish to bus
        self.event_bus.publish(event).await?;

        Ok(())
    }

    /// Get event stream for aggregate
    pub async fn get_event_stream(&self, aggregate_id: &str) -> crate::error::Result<Vec<DomainEvent>> {
        self.event_store.get_events(aggregate_id).await
    }

    /// Get current state by replaying events
    pub async fn get_current_state(&self, aggregate_id: &str) -> crate::error::Result<serde_json::Value> {
        // Try to get snapshot first
        if let Ok(Some(snapshot)) = self.snapshot_manager.get_snapshot(aggregate_id).await {
            return Ok(snapshot.state);
        }

        // Replay events to reconstruct state
        let events = self.event_store.get_events(aggregate_id).await?;
        let mut state = serde_json::json!({});

        for event in events {
            state = self.apply_event(&state, &event)?;
        }

        Ok(state)
    }

    /// Apply event to state
    fn apply_event(&self, state: &serde_json::Value, event: &DomainEvent) -> crate::error::Result<serde_json::Value> {
        // This is a simplified implementation
        // In production, you'd have specific handlers for each event type
        let mut new_state = state.clone();
        
        if let Some(obj) = new_state.as_object_mut() {
            obj.insert("last_event".to_string(), serde_json::json!(event.event_type));
            obj.insert("last_event_at".to_string(), serde_json::json!(event.metadata.timestamp));
        }

        Ok(new_state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_sourcing_system_creation() {
        let config = EventSourcingConfig::default();
        let system = EventSourcingSystem::new(config).await.unwrap();
        
        assert!(system.event_store().is_some());
        assert!(system.event_bus().is_some());
    }
}
