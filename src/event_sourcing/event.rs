//! Domain Events for Event Sourcing

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Event version for schema evolution
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl EventVersion {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    pub fn v1() -> Self {
        Self { major: 1, minor: 0, patch: 0 }
    }
}

impl std::fmt::Display for EventVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Event metadata
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventMetadata {
    /// Event ID (unique)
    pub event_id: String,
    /// Aggregate ID (what this event is about)
    pub aggregate_id: String,
    /// Aggregate type
    pub aggregate_type: String,
    /// Event timestamp
    pub timestamp: DateTime<Utc>,
    /// Event version for schema evolution
    pub version: EventVersion,
    /// User/service that triggered the event
    pub actor: String,
    /// Correlation ID for tracing
    pub correlation_id: String,
    /// Causation ID (what caused this event)
    pub causation_id: Option<String>,
    /// Custom metadata
    pub custom_metadata: HashMap<String, serde_json::Value>,
}

impl EventMetadata {
    pub fn new(aggregate_id: String, aggregate_type: String, actor: String) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            aggregate_id,
            aggregate_type,
            timestamp: Utc::now(),
            version: EventVersion::v1(),
            actor,
            correlation_id: uuid::Uuid::new_v4().to_string(),
            causation_id: None,
            custom_metadata: HashMap::new(),
        }
    }
}

/// Domain Event
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DomainEvent {
    /// Event metadata
    pub metadata: EventMetadata,
    /// Event type
    pub event_type: String,
    /// Event payload
    pub payload: serde_json::Value,
}

impl DomainEvent {
    pub fn new(
        metadata: EventMetadata,
        event_type: String,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            metadata,
            event_type,
            payload,
        }
    }

    /// Create a new event with builder pattern
    pub fn builder(aggregate_id: String, aggregate_type: String, actor: String) -> EventBuilder {
        EventBuilder {
            metadata: EventMetadata::new(aggregate_id, aggregate_type, actor),
            event_type: String::new(),
            payload: serde_json::json!({}),
        }
    }
}

/// Event builder
pub struct EventBuilder {
    metadata: EventMetadata,
    event_type: String,
    payload: serde_json::Value,
}

impl EventBuilder {
    pub fn event_type(mut self, event_type: impl Into<String>) -> Self {
        self.event_type = event_type.into();
        self
    }

    pub fn payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = payload;
        self
    }

    pub fn causation_id(mut self, causation_id: String) -> Self {
        self.metadata.causation_id = Some(causation_id);
        self
    }

    pub fn correlation_id(mut self, correlation_id: String) -> Self {
        self.metadata.correlation_id = correlation_id;
        self
    }

    pub fn version(mut self, version: EventVersion) -> Self {
        self.metadata.version = version;
        self
    }

    pub fn custom_metadata(mut self, key: String, value: serde_json::Value) -> Self {
        self.metadata.custom_metadata.insert(key, value);
        self
    }

    pub fn build(self) -> DomainEvent {
        DomainEvent::new(self.metadata, self.event_type, self.payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_version() {
        let version = EventVersion::new(1, 2, 3);
        assert_eq!(version.to_string(), "1.2.3");
    }

    #[test]
    fn test_event_metadata_creation() {
        let metadata = EventMetadata::new(
            "agg123".to_string(),
            "StellarNode".to_string(),
            "user@example.com".to_string(),
        );

        assert_eq!(metadata.aggregate_id, "agg123");
        assert_eq!(metadata.aggregate_type, "StellarNode");
        assert_eq!(metadata.actor, "user@example.com");
    }

    #[test]
    fn test_domain_event_builder() {
        let event = DomainEvent::builder(
            "agg123".to_string(),
            "StellarNode".to_string(),
            "user@example.com".to_string(),
        )
        .event_type("NodeCreated")
        .payload(serde_json::json!({"name": "validator-1"}))
        .build();

        assert_eq!(event.event_type, "NodeCreated");
        assert_eq!(event.metadata.aggregate_id, "agg123");
    }
}
