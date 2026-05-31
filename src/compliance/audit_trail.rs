//! Evidence collection for compliance audits.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: DateTime<Utc>,
    pub action: String,
    pub resource: String,
    pub resource_kind: String,
    pub actor: String,
    pub outcome: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuditTrail {
    pub entries: Vec<AuditEntry>,
}

pub struct AuditCollector {
    trail: AuditTrail,
}

impl AuditCollector {
    pub fn new() -> Self {
        Self {
            trail: AuditTrail::default(),
        }
    }

    pub fn record_event(
        &mut self,
        action: &str,
        resource: &str,
        resource_kind: &str,
        actor: &str,
        outcome: &str,
        details: Option<String>,
    ) {
        self.trail.entries.push(AuditEntry {
            timestamp: Utc::now(),
            action: action.to_string(),
            resource: resource.to_string(),
            resource_kind: resource_kind.to_string(),
            actor: actor.to_string(),
            outcome: outcome.to_string(),
            details,
        });
    }

    pub fn export_for_period(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<AuditEntry> {
        self.trail
            .entries
            .iter()
            .filter(|e| e.timestamp >= start && e.timestamp <= end)
            .cloned()
            .collect()
    }

    pub fn export_all(&self) -> &[AuditEntry] {
        &self.trail.entries
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "generated_at": Utc::now(),
            "total_entries": self.trail.entries.len(),
            "entries": self.trail.entries,
        })
    }
}

impl Default for AuditCollector {
    fn default() -> Self {
        Self::new()
    }
}
