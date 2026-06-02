//! Event schema types with versioning support for the event processing system.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Semantic version for event schemas
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SchemaVersion {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    pub fn v1() -> Self {
        Self::new(1, 0, 0)
    }

    /// Returns true if this version is backward-compatible with `other`
    pub fn is_compatible_with(&self, other: &SchemaVersion) -> bool {
        self.major == other.major && self.minor >= other.minor
    }
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Source of an event (which service/component produced it)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventSource {
    Controller,
    Reconciler,
    HealthCheck,
    DiskScaler,
    PeerDiscovery,
    Webhook,
    External(String),
}

impl std::fmt::Display for EventSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Controller => write!(f, "controller"),
            Self::Reconciler => write!(f, "reconciler"),
            Self::HealthCheck => write!(f, "health_check"),
            Self::DiskScaler => write!(f, "disk_scaler"),
            Self::PeerDiscovery => write!(f, "peer_discovery"),
            Self::Webhook => write!(f, "webhook"),
            Self::External(s) => write!(f, "external:{s}"),
        }
    }
}

/// Severity level of an event
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EventSeverity {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

/// Strongly-typed, versioned event envelope used throughout the processing pipeline
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessingEvent {
    /// Unique event identifier
    pub id: String,
    /// Schema version for this event type
    pub schema_version: SchemaVersion,
    /// Event type name (e.g. "stellar.node.created")
    pub event_type: String,
    /// Source component
    pub source: EventSource,
    /// Severity
    pub severity: EventSeverity,
    /// When the event occurred
    pub timestamp: DateTime<Utc>,
    /// Aggregate / resource this event relates to
    pub aggregate_id: String,
    /// Namespace of the resource
    pub namespace: String,
    /// Correlation ID for distributed tracing
    pub correlation_id: String,
    /// Causation ID (ID of the event that caused this one)
    pub causation_id: Option<String>,
    /// Typed payload
    pub payload: serde_json::Value,
    /// Arbitrary key-value labels
    pub labels: HashMap<String, String>,
}

impl ProcessingEvent {
    pub fn new(
        event_type: impl Into<String>,
        source: EventSource,
        aggregate_id: impl Into<String>,
        namespace: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: uuid(),
            schema_version: SchemaVersion::v1(),
            event_type: event_type.into(),
            source,
            severity: EventSeverity::Info,
            timestamp: Utc::now(),
            aggregate_id: aggregate_id.into(),
            namespace: namespace.into(),
            correlation_id: uuid(),
            causation_id: None,
            payload,
            labels: HashMap::new(),
        }
    }

    pub fn with_severity(mut self, severity: EventSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = id.into();
        self
    }

    pub fn with_causation_id(mut self, id: impl Into<String>) -> Self {
        self.causation_id = Some(id.into());
        self
    }

    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }
}

fn uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:016x}-{:08x}", rand_u64(), nanos)
}

fn rand_u64() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    h.finish()
}

/// Schema definition stored in the catalog
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventSchema {
    pub event_type: String,
    pub version: SchemaVersion,
    pub description: String,
    /// JSON Schema for the payload field
    pub payload_schema: serde_json::Value,
    pub deprecated: bool,
    pub created_at: DateTime<Utc>,
}

impl EventSchema {
    pub fn new(
        event_type: impl Into<String>,
        version: SchemaVersion,
        description: impl Into<String>,
        payload_schema: serde_json::Value,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            version,
            description: description.into(),
            payload_schema,
            deprecated: false,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_compatibility() {
        let v1 = SchemaVersion::new(1, 0, 0);
        let v1_1 = SchemaVersion::new(1, 1, 0);
        let v2 = SchemaVersion::new(2, 0, 0);
        assert!(v1_1.is_compatible_with(&v1));
        assert!(!v2.is_compatible_with(&v1));
    }

    #[test]
    fn processing_event_builder() {
        let ev = ProcessingEvent::new(
            "stellar.node.created",
            EventSource::Controller,
            "my-validator",
            "stellar",
            serde_json::json!({"network": "testnet"}),
        )
        .with_severity(EventSeverity::Info)
        .with_label("env", "prod");

        assert_eq!(ev.event_type, "stellar.node.created");
        assert_eq!(ev.labels.get("env").unwrap(), "prod");
    }
}
