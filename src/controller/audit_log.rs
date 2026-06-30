//! Admin Activity Audit Log
//!
//! Provides an in-memory, bounded ring-buffer audit log that records
//! operator admin actions. Entries are exposed via the REST API so operators
//! can inspect recent activity without reaching into raw Kubernetes events.
//!
//! # Design
//!
//! - **Thread-safe**: wrapped in `Arc<AuditLog>` for shared ownership across
//!   async tasks and the HTTP server.
//! - **Bounded**: holds at most `MAX_ENTRIES` records, discarding the oldest
//!   once the cap is reached (ring-buffer semantics).
//! - **Filterable**: callers can query by namespace, resource name, actor, or
//!   action type.
//!
//! # Example
//!
//! ```rust
//! use stellar_k8s::controller::audit_log::{AuditLog, AuditEntry, AdminAction};
//! use std::sync::Arc;
//!
//! let log = Arc::new(AuditLog::new());
//! log.record(AuditEntry::new(
//!     AdminAction::SetLogLevel,
//!     "admin-user",
//!     "operator",
//!     "stellar-system",
//!     Some(r#"{"level":"debug"}"#),
//! ));
//! let entries = log.list(None, None, None, 50);
//! assert_eq!(entries.len(), 1);
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use tokio::sync::broadcast;
use tracing::debug;

/// Maximum number of audit entries kept in memory.
pub const MAX_ENTRIES: usize = 10_000;

// ─── Action Types ─────────────────────────────────────────────────────────────

/// Enumeration of admin actions that can be recorded in the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminAction {
    /// A node was created via the API.
    NodeCreate,
    /// A node was updated.
    NodeUpdate,
    /// A node was deleted.
    NodeDelete,
    /// A node was suspended.
    NodeSuspend,
    /// A node was resumed from suspension.
    NodeResume,
    /// The operator log level was changed dynamically.
    SetLogLevel,
    /// A maintenance window was triggered manually.
    TriggerMaintenance,
    /// A forensic snapshot was requested.
    ForensicSnapshot,
    /// A disaster-recovery drill was started.
    DrDrillStart,
    /// A disaster-recovery restore was initiated.
    DrRestore,
    /// A CVE patch cycle was triggered manually.
    CvePatch,
    /// A webhook endpoint was registered.
    WebhookRegister,
    /// Triggered when the operator's runtime configuration is updated.
    ConfigUpdate,
    /// Triggered when the operator's runtime configuration is deleted.
    ConfigDelete,
    /// A webhook endpoint was removed.
    WebhookUnregister,
    /// A background job was manually triggered.
    BackgroundJobTrigger,
    /// Other admin action not covered by the above variants.
    Other(String),
}

impl std::fmt::Display for AdminAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdminAction::Other(s) => write!(f, "{s}"),
            _ => write!(f, "{}", serde_json::to_string(self).unwrap_or_default()),
        }
    }
}

// ─── Audit Entry ──────────────────────────────────────────────────────────────

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Unique identifier for this entry.
    pub id: String,
    /// When the action occurred.
    pub timestamp: DateTime<Utc>,
    /// The action that was performed.
    pub action: AdminAction,
    /// Identity of the actor (Kubernetes service account, token subject, etc.).
    pub actor: String,
    /// Optional metadata about the actor (groups, IP, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_metadata: Option<serde_json::Value>,
    /// The resource that was affected.
    pub resource: String,
    /// Namespace in which the action occurred (may be empty for cluster-scoped actions).
    pub namespace: String,
    /// Optional JSON blob with additional context (parameters, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// Optional JSON patch or diff showing what changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<serde_json::Value>,
    /// Whether the action succeeded.
    pub success: bool,
    /// Error message, if `success` is `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AuditEntry {
    /// Create a new successful audit entry.
    pub fn new(
        action: AdminAction,
        actor: impl Into<String>,
        resource: impl Into<String>,
        namespace: impl Into<String>,
        details: Option<&str>,
    ) -> Self {
        Self {
            id: generate_id(),
            timestamp: Utc::now(),
            action,
            actor: actor.into(),
            actor_metadata: None,
            resource: resource.into(),
            namespace: namespace.into(),
            details: details.map(|s| s.to_string()),
            diff: None,
            success: true,
            error: None,
        }
    }

    /// Create a failed audit entry.
    pub fn failed(
        action: AdminAction,
        actor: impl Into<String>,
        resource: impl Into<String>,
        namespace: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            id: generate_id(),
            timestamp: Utc::now(),
            action,
            actor: actor.into(),
            actor_metadata: None,
            resource: resource.into(),
            namespace: namespace.into(),
            details: None,
            diff: None,
            success: false,
            error: Some(error.into()),
        }
    }

    /// Set the diff for this entry.
    pub fn with_diff(mut self, diff: serde_json::Value) -> Self {
        self.diff = Some(diff);
        self
    }

    /// Set the actor metadata for this entry.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.actor_metadata = Some(metadata);
        self
    }
}

// ─── Audit Log ────────────────────────────────────────────────────────────────

/// Thread-safe, bounded in-memory audit log.
pub struct AuditLog {
    entries: RwLock<Vec<AuditEntry>>,
    broadcaster: broadcast::Sender<AuditEntry>,
}

impl AuditLog {
    /// Create a new, empty audit log.
    pub fn new() -> Self {
        let (broadcaster, _rx) = broadcast::channel(MAX_ENTRIES);
        Self {
            entries: RwLock::new(Vec::with_capacity(MAX_ENTRIES)),
            broadcaster,
        }
    }

    /// Record an audit entry.
    ///
    /// If the log is at capacity (`MAX_ENTRIES`), the oldest entry is dropped.
    pub fn record(&self, entry: AuditEntry) {
        debug!(
            action = %entry.action,
            actor = %entry.actor,
            resource = %entry.resource,
            "Audit log entry recorded"
        );
        let mut entries = self.entries.write().unwrap();
        if entries.len() >= MAX_ENTRIES {
            entries.remove(0);
        }
        entries.push(entry.clone());
        let _ = self.broadcaster.send(entry);
    }

    /// Subscribe to real-time audit entries.
    pub fn subscribe(&self) -> broadcast::Receiver<AuditEntry> {
        self.broadcaster.subscribe()
    }

    /// List audit entries, optionally filtering by namespace, resource name,
    /// and/or actor. Results are returned newest-first, capped at `limit`.
    ///
    /// Pass `0` for `limit` to return all matching entries (up to `MAX_ENTRIES`).
    pub fn list(
        &self,
        namespace: Option<&str>,
        resource: Option<&str>,
        actor: Option<&str>,
        limit: usize,
    ) -> Vec<AuditEntry> {
        let entries = self.entries.read().unwrap();
        let iter = entries.iter().rev().filter(|e| {
            namespace.is_none_or(|ns| e.namespace == ns)
                && resource.is_none_or(|r| e.resource.contains(r))
                && actor.is_none_or(|a| e.actor == a)
        });

        if limit == 0 {
            iter.cloned().collect()
        } else {
            iter.take(limit).cloned().collect()
        }
    }

    /// Return the total number of entries in the log.
    pub fn count(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    /// Clear all entries (useful for testing).
    pub fn clear(&self) {
        self.entries.write().unwrap().clear();
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn generate_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let n: u64 = rng.gen();
    format!("audit-{n:016x}")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(action: AdminAction, namespace: &str, resource: &str, actor: &str) -> AuditEntry {
        AuditEntry::new(action, actor, resource, namespace, None)
    }

    #[test]
    fn test_record_and_list() {
        let log = AuditLog::new();
        assert_eq!(log.count(), 0);

        log.record(make_entry(
            AdminAction::SetLogLevel,
            "stellar-system",
            "operator",
            "admin",
        ));
        log.record(make_entry(
            AdminAction::NodeCreate,
            "default",
            "my-node",
            "user1",
        ));
        assert_eq!(log.count(), 2);

        let all = log.list(None, None, None, 0);
        assert_eq!(all.len(), 2);
        // Newest-first
        assert_eq!(all[0].resource, "my-node");
    }

    #[test]
    fn test_filter_by_namespace() {
        let log = AuditLog::new();
        log.record(make_entry(
            AdminAction::NodeCreate,
            "ns-a",
            "node-a",
            "user",
        ));
        log.record(make_entry(
            AdminAction::NodeDelete,
            "ns-b",
            "node-b",
            "user",
        ));

        let ns_a = log.list(Some("ns-a"), None, None, 0);
        assert_eq!(ns_a.len(), 1);
        assert_eq!(ns_a[0].namespace, "ns-a");
    }

    #[test]
    fn test_filter_by_actor() {
        let log = AuditLog::new();
        log.record(make_entry(AdminAction::SetLogLevel, "sys", "op", "admin1"));
        log.record(make_entry(AdminAction::SetLogLevel, "sys", "op", "admin2"));

        let a1 = log.list(None, None, Some("admin1"), 0);
        assert_eq!(a1.len(), 1);
        assert_eq!(a1[0].actor, "admin1");
    }

    #[test]
    fn test_limit() {
        let log = AuditLog::new();
        for i in 0..10 {
            log.record(make_entry(
                AdminAction::NodeCreate,
                "ns",
                &format!("node-{i}"),
                "user",
            ));
        }
        let limited = log.list(None, None, None, 3);
        assert_eq!(limited.len(), 3);
    }

    #[test]
    fn test_ring_buffer_max_entries() {
        let log = AuditLog::new();
        // Temporarily override MAX_ENTRIES is not possible for const, so we
        // just push MAX_ENTRIES + 1 entries and verify the count is capped.
        for i in 0..=MAX_ENTRIES {
            log.record(make_entry(
                AdminAction::NodeCreate,
                "ns",
                &format!("node-{i}"),
                "user",
            ));
        }
        assert_eq!(log.count(), MAX_ENTRIES);
    }

    #[test]
    fn test_failed_entry() {
        let entry = AuditEntry::failed(
            AdminAction::NodeDelete,
            "user",
            "my-node",
            "default",
            "permission denied",
        );
        assert!(!entry.success);
        assert_eq!(entry.error.unwrap(), "permission denied");
    }

    #[test]
    fn test_clear() {
        let log = AuditLog::new();
        log.record(make_entry(AdminAction::NodeCreate, "ns", "n", "u"));
        assert_eq!(log.count(), 1);
        log.clear();
        assert_eq!(log.count(), 0);
    }

    #[test]
    fn test_generate_id_uniqueness() {
        let id1 = generate_id();
        let id2 = generate_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("audit-"));
    }
}
