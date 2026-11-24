//! LedgerCloseWebhook CRD — Webhook subscriptions for Stellar ledger-close events.
//!
//! This module defines the `LedgerCloseWebhook` Custom Resource, which lets operators
//! register external HTTP endpoints that should receive a signed POST payload within
//! 5 seconds of every Stellar ledger close.
//!
//! # Example manifest
//!
//! ```yaml
//! apiVersion: stellar.org/v1alpha1
//! kind: LedgerCloseWebhook
//! metadata:
//!   name: my-ledger-hook
//!   namespace: stellar
//! spec:
//!   url: "https://example.com/hooks/ledger"
//!   secret: "my-hmac-secret"          # reference to a K8s Secret key
//!   events:
//!     - LedgerClose
//!   maxRetries: 5
//!   timeoutSeconds: 5
//! ```
//!
//! # Delivery guarantees
//!
//! * **At-least-once** — the dispatcher retries up to `maxRetries` times with
//!   exponential back-off starting at 1 s (1 s → 2 s → 4 s → 8 s → 16 s).
//! * **Ordered per subscription** — deliveries for the same `LedgerCloseWebhook`
//!   resource are serialised; a slow consumer never causes ledger skips.
//! * **HMAC-SHA256 signature** — every POST carries an
//!   `X-Stellar-Signature: sha256=<hex>` header so consumers can verify integrity.

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ─── Spec ────────────────────────────────────────────────────────────────────

/// Which ledger-close event types should trigger delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum LedgerCloseEventType {
    /// Fired on every ledger close — the primary event type.
    LedgerClose,
    /// Fired when the node's ledger sequence falls behind the network tip.
    LedgerLag,
    /// Fired when the node re-synchronises after a lag period.
    LedgerSync,
}

/// Specification for a `LedgerCloseWebhook` resource.
///
/// `LedgerCloseWebhook` — subscribes an external HTTP endpoint to Stellar
/// ledger-close events with HMAC-signed payloads and at-least-once delivery.
#[derive(CustomResource, Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "LedgerCloseWebhook",
    namespaced,
    shortname = "lchook",
    status = "LedgerCloseWebhookStatus",
    printcolumn = r#"{"name":"URL","type":"string","jsonPath":".spec.url"}"#,
    printcolumn = r#"{"name":"Enabled","type":"boolean","jsonPath":".spec.enabled"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Delivered","type":"integer","jsonPath":".status.totalDelivered"}"#,
    printcolumn = r#"{"name":"LastSeq","type":"integer","jsonPath":".status.lastDeliveredSequence"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct LedgerCloseWebhookSpec {
    /// Target URL that will receive POST requests.
    pub url: String,

    /// Name of a Kubernetes `Secret` whose `value` key holds the HMAC signing
    /// secret. Leave empty to skip payload signing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,

    /// Ledger-close event types to subscribe to. Defaults to `[LedgerClose]`.
    #[serde(default = "default_events")]
    pub events: Vec<LedgerCloseEventType>,

    /// Maximum delivery retries before the delivery is marked `Failed`.
    /// Defaults to 5.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// HTTP request timeout in seconds per attempt. Defaults to 5.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,

    /// Whether delivery is active. Set to `false` to pause without deleting.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_events() -> Vec<LedgerCloseEventType> {
    vec![LedgerCloseEventType::LedgerClose]
}

fn default_max_retries() -> u32 {
    5
}

fn default_timeout_seconds() -> u64 {
    5
}

fn default_enabled() -> bool {
    true
}

// ─── Status ──────────────────────────────────────────────────────────────────

/// Phase of the most-recent delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "PascalCase")]
pub enum DeliveryPhase {
    /// No delivery has been attempted yet.
    #[default]
    Pending,
    /// Currently being delivered (or retried).
    Delivering,
    /// Successfully delivered.
    Delivered,
    /// All retry attempts exhausted — delivery failed.
    Failed,
}

/// Per-delivery log entry persisted in the resource status.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryLogEntry {
    /// Stellar ledger sequence number this delivery corresponds to.
    pub ledger_sequence: u64,
    /// Wall-clock time the delivery was first attempted.
    pub attempted_at: DateTime<Utc>,
    /// Wall-clock time the delivery succeeded or was given up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Final phase after all retries.
    pub phase: DeliveryPhase,
    /// Number of attempts made (1 = first try, 2 = first retry, …).
    pub attempts: u32,
    /// HTTP status code from the last attempt (if a response was received).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status_code: Option<u16>,
    /// Error message from the last failed attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Status subresource written back by the dispatcher.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LedgerCloseWebhookStatus {
    /// Sequence number of the last ledger successfully delivered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_delivered_sequence: Option<u64>,
    /// Timestamp of the last successful delivery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_delivered_at: Option<DateTime<Utc>>,
    /// Total number of successful deliveries since the resource was created.
    pub total_delivered: u64,
    /// Total number of failed deliveries (all retries exhausted).
    pub total_failed: u64,
    /// Current delivery phase (most-recent attempt).
    pub phase: DeliveryPhase,
    /// Ring-buffer of the last 20 delivery log entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivery_log: Vec<DeliveryLogEntry>,
}

// ─── Payload ─────────────────────────────────────────────────────────────────

/// JSON body POSTed to subscriber endpoints on each ledger close.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerClosePayload {
    /// Always `"LedgerClose"` (for forward-compat envelope dispatch).
    pub event: String,
    /// ISO-8601 timestamp at which the operator processed this ledger close.
    pub timestamp: DateTime<Utc>,
    /// Ledger sequence number.
    pub ledger_sequence: u64,
    /// Ledger close time as reported by the network (Unix epoch seconds).
    pub ledger_close_time: i64,
    /// Total number of transactions in this ledger.
    pub transaction_count: u32,
    /// Total number of operations across all transactions.
    pub operation_count: u32,
    /// Network passphrase that distinguishes mainnet / testnet.
    pub network_passphrase: String,
    /// Name of the `StellarNode` whose horizon reported this close.
    pub source_node: String,
    /// Kubernetes namespace of the source node.
    pub source_namespace: String,
}

impl LedgerClosePayload {
    /// Build a payload for a ledger close event.
    pub fn new(
        ledger_sequence: u64,
        ledger_close_time: i64,
        transaction_count: u32,
        operation_count: u32,
        network_passphrase: impl Into<String>,
        source_node: impl Into<String>,
        source_namespace: impl Into<String>,
    ) -> Self {
        Self {
            event: "LedgerClose".to_string(),
            timestamp: Utc::now(),
            ledger_sequence,
            ledger_close_time,
            transaction_count,
            operation_count,
            network_passphrase: network_passphrase.into(),
            source_node: source_node.into(),
            source_namespace: source_namespace.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_spec_has_ledger_close_event() {
        let spec = LedgerCloseWebhookSpec::default();
        assert!(spec.events.contains(&LedgerCloseEventType::LedgerClose));
    }

    #[test]
    fn default_spec_is_enabled() {
        assert!(LedgerCloseWebhookSpec::default().enabled);
    }

    #[test]
    fn payload_event_field_is_correct() {
        let p = LedgerClosePayload::new(100, 1_700_000_000, 5, 10, "Test SDF Network", "n", "ns");
        assert_eq!(p.event, "LedgerClose");
        assert_eq!(p.ledger_sequence, 100);
    }
}
