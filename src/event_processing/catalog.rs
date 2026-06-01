//! Event catalog with schema registry.
//!
//! Maintains a versioned registry of event schemas and validates incoming
//! events against their registered schema.

use crate::event_processing::schema::{EventSchema, ProcessingEvent, SchemaVersion};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

// ── Validation result ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
}

impl ValidationResult {
    fn ok() -> Self {
        Self { valid: true, errors: vec![] }
    }
    fn fail(errors: Vec<String>) -> Self {
        Self { valid: false, errors }
    }
}

// ── Catalog entry ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub schema: EventSchema,
    pub registered_at: DateTime<Utc>,
    pub registered_by: String,
    /// Previous versions kept for compatibility checks
    pub history: Vec<EventSchema>,
}

// ── Catalog ───────────────────────────────────────────────────────────────────

/// The event catalog / schema registry
pub struct EventCatalog {
    /// Key: event_type → latest entry
    entries: RwLock<HashMap<String, CatalogEntry>>,
}

impl EventCatalog {
    pub fn new() -> Arc<Self> {
        let catalog = Arc::new(Self {
            entries: RwLock::new(HashMap::new()),
        });
        // Pre-register built-in Stellar-K8s event schemas
        let c = catalog.clone();
        tokio::spawn(async move { c.register_builtin_schemas().await });
        catalog
    }

    /// Register a new schema (or a new version of an existing one).
    pub async fn register(
        &self,
        schema: EventSchema,
        registered_by: impl Into<String>,
    ) -> crate::error::Result<()> {
        let mut entries = self.entries.write().await;
        let registered_by = registered_by.into();

        if let Some(existing) = entries.get_mut(&schema.event_type) {
            // Validate backward compatibility
            if schema.version.major != existing.schema.version.major {
                warn!(
                    "catalog: breaking schema change for '{}': {} → {}",
                    schema.event_type, existing.schema.version, schema.version
                );
            }
            let old = existing.schema.clone();
            existing.history.push(old);
            existing.schema = schema.clone();
            existing.registered_at = Utc::now();
            existing.registered_by = registered_by;
        } else {
            info!("catalog: registering new schema '{}'", schema.event_type);
            entries.insert(
                schema.event_type.clone(),
                CatalogEntry {
                    schema,
                    registered_at: Utc::now(),
                    registered_by,
                    history: vec![],
                },
            );
        }
        Ok(())
    }

    /// Look up the latest schema for an event type.
    pub async fn get(&self, event_type: &str) -> Option<CatalogEntry> {
        self.entries.read().await.get(event_type).cloned()
    }

    /// List all registered event types.
    pub async fn list_event_types(&self) -> Vec<String> {
        let mut types: Vec<_> = self.entries.read().await.keys().cloned().collect();
        types.sort();
        types
    }

    /// Validate an event's payload against its registered schema.
    ///
    /// If no schema is registered the event is considered valid (open schema).
    pub async fn validate(&self, event: &ProcessingEvent) -> ValidationResult {
        let entries = self.entries.read().await;
        let Some(entry) = entries.get(&event.event_type) else {
            return ValidationResult::ok(); // unknown type → pass-through
        };

        // Version compatibility check
        if !event.schema_version.is_compatible_with(&entry.schema.version) {
            return ValidationResult::fail(vec![format!(
                "schema version mismatch: event={} registered={}",
                event.schema_version, entry.schema.version
            )]);
        }

        // Basic JSON Schema validation (required fields only)
        let errors = validate_against_schema(&event.payload, &entry.schema.payload_schema);
        if errors.is_empty() {
            ValidationResult::ok()
        } else {
            ValidationResult::fail(errors)
        }
    }

    /// Render an HTML catalog page.
    pub async fn render_html(&self) -> String {
        let entries = self.entries.read().await;
        let mut rows = String::new();
        let mut types: Vec<_> = entries.keys().collect();
        types.sort();
        for t in types {
            let e = &entries[t];
            rows.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                e.schema.event_type,
                e.schema.version,
                e.schema.description,
                e.registered_by,
                e.registered_at.format("%Y-%m-%d %H:%M UTC"),
            ));
        }
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8"/>
  <title>Stellar-K8s Event Catalog</title>
  <style>
    body {{ font-family: sans-serif; margin: 2rem; }}
    h1 {{ color: #1a73e8; }}
    table {{ border-collapse: collapse; width: 100%; }}
    th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
    th {{ background: #f2f2f2; }}
    tr:nth-child(even) {{ background: #fafafa; }}
  </style>
</head>
<body>
  <h1>Stellar-K8s Event Catalog</h1>
  <table>
    <thead>
      <tr><th>Event Type</th><th>Version</th><th>Description</th><th>Registered By</th><th>Registered At</th></tr>
    </thead>
    <tbody>{rows}</tbody>
  </table>
</body>
</html>"#
        )
    }

    async fn register_builtin_schemas(&self) {
        let schemas = vec![
            EventSchema::new(
                "stellar.node.created",
                SchemaVersion::v1(),
                "A StellarNode resource was created",
                serde_json::json!({"required": ["name", "namespace", "node_type"]}),
            ),
            EventSchema::new(
                "stellar.node.deleted",
                SchemaVersion::v1(),
                "A StellarNode resource was deleted",
                serde_json::json!({"required": ["name", "namespace"]}),
            ),
            EventSchema::new(
                "stellar.node.health_changed",
                SchemaVersion::v1(),
                "Node health status changed",
                serde_json::json!({"required": ["name", "status"]}),
            ),
            EventSchema::new(
                "stellar.disk.warning",
                SchemaVersion::v1(),
                "Disk usage exceeded warning threshold",
                serde_json::json!({"required": ["usage_percent"]}),
            ),
            EventSchema::new(
                "stellar.peer.discovered",
                SchemaVersion::v1(),
                "A new peer was discovered",
                serde_json::json!({"required": ["peer_id"]}),
            ),
        ];

        for schema in schemas {
            let _ = self.register(schema, "system").await;
        }
    }
}

impl Default for EventCatalog {
    fn default() -> Self {
        Self { entries: RwLock::new(HashMap::new()) }
    }
}

/// Minimal JSON Schema validation: checks `required` array only.
fn validate_against_schema(payload: &serde_json::Value, schema: &serde_json::Value) -> Vec<String> {
    let mut errors = vec![];
    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        for field in required {
            if let Some(field_name) = field.as_str() {
                if payload.get(field_name).is_none() {
                    errors.push(format!("missing required field: {field_name}"));
                }
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_processing::schema::{EventSource, SchemaVersion};

    #[tokio::test]
    async fn register_and_lookup() {
        let catalog = EventCatalog::default();
        catalog
            .register(
                EventSchema::new(
                    "test.event",
                    SchemaVersion::v1(),
                    "test",
                    serde_json::json!({"required": ["name"]}),
                ),
                "test",
            )
            .await
            .unwrap();

        let entry = catalog.get("test.event").await.unwrap();
        assert_eq!(entry.schema.event_type, "test.event");
    }

    #[tokio::test]
    async fn validation_catches_missing_field() {
        let catalog = EventCatalog::default();
        catalog
            .register(
                EventSchema::new(
                    "test.event",
                    SchemaVersion::v1(),
                    "test",
                    serde_json::json!({"required": ["name"]}),
                ),
                "test",
            )
            .await
            .unwrap();

        let ev = ProcessingEvent::new(
            "test.event",
            EventSource::Controller,
            "agg",
            "ns",
            serde_json::json!({}), // missing "name"
        );
        let result = catalog.validate(&ev).await;
        assert!(!result.valid);
        assert!(result.errors[0].contains("name"));
    }

    #[tokio::test]
    async fn validation_passes_with_required_fields() {
        let catalog = EventCatalog::default();
        catalog
            .register(
                EventSchema::new(
                    "test.event",
                    SchemaVersion::v1(),
                    "test",
                    serde_json::json!({"required": ["name"]}),
                ),
                "test",
            )
            .await
            .unwrap();

        let ev = ProcessingEvent::new(
            "test.event",
            EventSource::Controller,
            "agg",
            "ns",
            serde_json::json!({"name": "validator-1"}),
        );
        let result = catalog.validate(&ev).await;
        assert!(result.valid);
    }
}
