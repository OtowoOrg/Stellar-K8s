//! Outbound Webhook Delivery System for Transaction Events
//!
//! This module provides an HTTP outbound webhook delivery system that fires
//! notifications when significant transaction and operator events occur in the
//! Stellar K8s operator.
//!
//! # Features
//!
//! - **Event Types**: Supports reconciliation, node status change, maintenance,
//!   remediation, and transaction-related events.
//! - **HMAC-SHA256 Signing**: Optionally signs payloads so consumers can verify
//!   authenticity.
//! - **Retry with Backoff**: Failed deliveries are retried up to a configurable
//!   maximum with exponential backoff.
//! - **Async Delivery**: Webhook sends are non-blocking — events are queued and
//!   delivered in a background task so reconciliation latency is unaffected.
//!
//! # Example
//!
//! ```rust,no_run
//! use stellar_k8s::controller::webhook_delivery::{
//!     WebhookDeliveryService, WebhookEndpoint, WebhookEvent, TransactionEventPayload,
//! };
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() {
//!     let svc = Arc::new(WebhookDeliveryService::new());
//!
//!     let endpoint = WebhookEndpoint {
//!         id: "my-hook".to_string(),
//!         url: "https://example.com/hooks/stellar".to_string(),
//!         secret: Some("s3cr3t".to_string()),
//!         events: vec![WebhookEventType::NodeStatusChanged],
//!         enabled: true,
//!         max_retries: 3,
//!     };
//!     svc.register_endpoint(endpoint);
//!
//!     let event = WebhookEvent::node_status_changed("stellar", "my-validator", "Ready");
//!     svc.dispatch(event).await;
//! }
//! ```

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};

type HmacSha256 = Hmac<Sha256>;

// ─── Event Types ─────────────────────────────────────────────────────────────

/// Categories of events that can trigger outbound webhook deliveries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEventType {
    /// A StellarNode's readiness / phase changed (e.g. Syncing → Ready).
    NodeStatusChanged,
    /// Reconciliation of a StellarNode completed (successfully or with error).
    ReconcileCompleted,
    /// A maintenance window started or ended.
    MaintenanceWindow,
    /// The operator applied automatic remediation to a node.
    Remediation,
    /// A transaction was submitted or confirmed on the network (Horizon event).
    TransactionConfirmed,
    /// A transaction submission failed.
    TransactionFailed,
    /// An operator admin action was performed (e.g. log level change).
    AdminAction,
    /// A background job changed state.
    BackgroundJobStateChange,
}

// ─── Endpoint ────────────────────────────────────────────────────────────────

/// A registered outbound webhook endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEndpoint {
    /// Unique identifier for this endpoint.
    pub id: String,
    /// The URL to POST events to.
    pub url: String,
    /// Optional HMAC-SHA256 signing secret. When set, a `X-Stellar-Signature`
    /// header is added to every request.
    pub secret: Option<String>,
    /// Event types this endpoint subscribes to. An empty vec means *all* types.
    pub events: Vec<WebhookEventType>,
    /// Whether this endpoint is currently active.
    pub enabled: bool,
    /// Maximum number of delivery attempts before giving up.
    pub max_retries: u32,
}

impl WebhookEndpoint {
    /// Returns `true` if this endpoint should receive `event_type`.
    pub fn subscribes_to(&self, event_type: &WebhookEventType) -> bool {
        self.enabled && (self.events.is_empty() || self.events.contains(event_type))
    }
}

// ─── Event Payloads ──────────────────────────────────────────────────────────

/// Generic envelope wrapping every outbound webhook payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEvent {
    /// Unique delivery ID (UUID v4 string).
    pub id: String,
    /// ISO-8601 timestamp when the event was created.
    pub timestamp: DateTime<Utc>,
    /// The kind of event.
    pub event_type: WebhookEventType,
    /// The serialised event body.
    pub payload: serde_json::Value,
}

impl WebhookEvent {
    fn new(event_type: WebhookEventType, payload: impl Serialize) -> Self {
        Self {
            id: generate_uuid(),
            timestamp: Utc::now(),
            event_type,
            payload: serde_json::to_value(payload).unwrap_or(serde_json::Value::Null),
        }
    }

    /// Convenience constructor for `NodeStatusChanged`.
    pub fn node_status_changed(namespace: &str, name: &str, new_phase: &str) -> Self {
        Self::new(
            WebhookEventType::NodeStatusChanged,
            serde_json::json!({
                "namespace": namespace,
                "name": name,
                "new_phase": new_phase,
            }),
        )
    }

    /// Convenience constructor for `ReconcileCompleted`.
    pub fn reconcile_completed(
        namespace: &str,
        name: &str,
        success: bool,
        error_msg: Option<&str>,
    ) -> Self {
        Self::new(
            WebhookEventType::ReconcileCompleted,
            serde_json::json!({
                "namespace": namespace,
                "name": name,
                "success": success,
                "error": error_msg,
            }),
        )
    }

    /// Convenience constructor for `TransactionConfirmed`.
    pub fn transaction_confirmed(namespace: &str, name: &str, tx_hash: &str, ledger: u64) -> Self {
        Self::new(
            WebhookEventType::TransactionConfirmed,
            serde_json::json!({
                "namespace": namespace,
                "name": name,
                "tx_hash": tx_hash,
                "ledger": ledger,
            }),
        )
    }

    /// Convenience constructor for `TransactionFailed`.
    pub fn transaction_failed(namespace: &str, name: &str, tx_hash: &str, reason: &str) -> Self {
        Self::new(
            WebhookEventType::TransactionFailed,
            serde_json::json!({
                "namespace": namespace,
                "name": name,
                "tx_hash": tx_hash,
                "reason": reason,
            }),
        )
    }

    /// Convenience constructor for `Remediation`.
    pub fn remediation(namespace: &str, name: &str, action: &str) -> Self {
        Self::new(
            WebhookEventType::Remediation,
            serde_json::json!({
                "namespace": namespace,
                "name": name,
                "action": action,
            }),
        )
    }

    /// Convenience constructor for `AdminAction`.
    pub fn admin_action(actor: &str, action: &str, resource: &str) -> Self {
        Self::new(
            WebhookEventType::AdminAction,
            serde_json::json!({
                "actor": actor,
                "action": action,
                "resource": resource,
            }),
        )
    }
}

// ─── Delivery Record ─────────────────────────────────────────────────────────

/// Result of a single delivery attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryAttempt {
    pub attempted_at: DateTime<Utc>,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    pub success: bool,
}

/// Full delivery record kept in the delivery log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryRecord {
    pub event_id: String,
    pub endpoint_id: String,
    pub event_type: WebhookEventType,
    pub attempts: Vec<DeliveryAttempt>,
    pub final_success: bool,
    pub created_at: DateTime<Utc>,
}

// ─── Service ─────────────────────────────────────────────────────────────────

/// Central webhook delivery service.
///
/// Registers endpoints and dispatches events to all matching subscribers
/// with retry logic and optional HMAC signing.
pub struct WebhookDeliveryService {
    endpoints: RwLock<Vec<WebhookEndpoint>>,
    delivery_log: Mutex<Vec<DeliveryRecord>>,
    client: Client,
}

impl WebhookDeliveryService {
    /// Create a new delivery service with a default reqwest client.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        Self {
            endpoints: RwLock::new(Vec::new()),
            delivery_log: Mutex::new(Vec::new()),
            client,
        }
    }

    /// Register a new outbound webhook endpoint.
    pub async fn register_endpoint(&self, endpoint: WebhookEndpoint) {
        let mut endpoints = self.endpoints.write().await;
        // Replace existing endpoint with the same id, or push new.
        if let Some(pos) = endpoints.iter().position(|e| e.id == endpoint.id) {
            endpoints[pos] = endpoint;
        } else {
            endpoints.push(endpoint);
        }
    }

    /// Remove a registered endpoint by id.
    pub async fn unregister_endpoint(&self, id: &str) {
        let mut endpoints = self.endpoints.write().await;
        endpoints.retain(|e| e.id != id);
    }

    /// List currently registered endpoints.
    pub async fn list_endpoints(&self) -> Vec<WebhookEndpoint> {
        self.endpoints.read().await.clone()
    }

    /// Return a snapshot of the delivery log (most recent first).
    pub async fn delivery_log(&self) -> Vec<DeliveryRecord> {
        let log = self.delivery_log.lock().await;
        let mut records = log.clone();
        records.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        records
    }

    /// Dispatch an event to all matching registered endpoints.
    ///
    /// Each matching endpoint is delivered to concurrently in a spawned task
    /// so this method returns quickly.
    pub async fn dispatch(self: &Arc<Self>, event: WebhookEvent) {
        let endpoints = {
            let guard = self.endpoints.read().await;
            guard
                .iter()
                .filter(|e| e.subscribes_to(&event.event_type))
                .cloned()
                .collect::<Vec<_>>()
        };

        for endpoint in endpoints {
            let svc = Arc::clone(self);
            let ev = event.clone();
            tokio::spawn(async move {
                svc.deliver_with_retry(&endpoint, &ev).await;
            });
        }
    }

    /// Attempt delivery to a single endpoint, retrying on failure.
    async fn deliver_with_retry(&self, endpoint: &WebhookEndpoint, event: &WebhookEvent) {
        let payload = match serde_json::to_vec(event) {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to serialise webhook event {}: {e}", event.id);
                return;
            }
        };

        let mut record = DeliveryRecord {
            event_id: event.id.clone(),
            endpoint_id: endpoint.id.clone(),
            event_type: event.event_type.clone(),
            attempts: Vec::new(),
            final_success: false,
            created_at: Utc::now(),
        };

        for attempt in 0..=endpoint.max_retries {
            let attempt_result = self.send_once(endpoint, &payload).await;
            let success = attempt_result.success;
            record.attempts.push(attempt_result);

            if success {
                record.final_success = true;
                info!(
                    "Webhook delivered: event={} endpoint={} attempt={}",
                    event.id, endpoint.id, attempt
                );
                break;
            }

            if attempt < endpoint.max_retries {
                let backoff = Duration::from_secs(2u64.pow(attempt));
                warn!(
                    "Webhook delivery failed for event={} endpoint={}, retrying in {:?} (attempt {}/{})",
                    event.id, endpoint.id, backoff, attempt + 1, endpoint.max_retries
                );
                tokio::time::sleep(backoff).await;
            } else {
                error!(
                    "Webhook delivery permanently failed: event={} endpoint={} after {} attempts",
                    event.id,
                    endpoint.id,
                    endpoint.max_retries + 1
                );
            }
        }

        self.delivery_log.lock().await.push(record);
    }

    /// Perform a single HTTP POST delivery attempt.
    async fn send_once(&self, endpoint: &WebhookEndpoint, payload: &[u8]) -> DeliveryAttempt {
        let attempted_at = Utc::now();

        let mut request = self
            .client
            .post(&endpoint.url)
            .header("Content-Type", "application/json")
            .header("User-Agent", "stellar-k8s-operator/1.0")
            .body(payload.to_vec());

        // Add HMAC-SHA256 signature header if secret is configured.
        if let Some(ref secret) = endpoint.secret {
            match sign_payload(secret, payload) {
                Ok(sig) => {
                    request = request.header("X-Stellar-Signature", format!("sha256={sig}"));
                }
                Err(e) => {
                    debug!("Failed to sign webhook payload: {e}");
                }
            }
        }

        match request.send().await {
            Ok(resp) => {
                let status = resp.status();
                let success = status.is_success();
                DeliveryAttempt {
                    attempted_at,
                    status_code: Some(status.as_u16()),
                    error: if success {
                        None
                    } else {
                        Some(format!("HTTP {status}"))
                    },
                    success,
                }
            }
            Err(e) => DeliveryAttempt {
                attempted_at,
                status_code: None,
                error: Some(e.to_string()),
                success: false,
            },
        }
    }
}

impl Default for WebhookDeliveryService {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Sign `payload` with HMAC-SHA256 using `secret` and return the hex digest.
fn sign_payload(secret: &str, payload: &[u8]) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("Invalid HMAC key: {e}"))?;
    mac.update(payload);
    let result = mac.finalize();
    Ok(hex::encode(result.into_bytes()))
}

/// Generate a simple pseudo-UUID v4 string (without pulling in the `uuid` crate).
fn generate_uuid() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u16::from_be_bytes([bytes[4], bytes[5]]),
        u16::from_be_bytes([bytes[6], bytes[7]]) & 0x0fff,
        (u16::from_be_bytes([bytes[8], bytes[9]]) & 0x3fff) | 0x8000,
        {
            let hi = u32::from_be_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as u64;
            let lo = u16::from_be_bytes([bytes[14], bytes[15]]) as u64;
            hi << 16 | lo
        }
    )
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_endpoint(id: &str, events: Vec<WebhookEventType>) -> WebhookEndpoint {
        WebhookEndpoint {
            id: id.to_string(),
            url: format!("https://example.com/hooks/{id}"),
            secret: Some("test-secret".to_string()),
            events,
            enabled: true,
            max_retries: 2,
        }
    }

    #[test]
    fn test_endpoint_subscribes_to_all_when_empty() {
        let ep = make_endpoint("ep1", vec![]);
        assert!(ep.subscribes_to(&WebhookEventType::NodeStatusChanged));
        assert!(ep.subscribes_to(&WebhookEventType::TransactionConfirmed));
    }

    #[test]
    fn test_endpoint_subscribes_to_specific_type() {
        let ep = make_endpoint("ep2", vec![WebhookEventType::TransactionConfirmed]);
        assert!(ep.subscribes_to(&WebhookEventType::TransactionConfirmed));
        assert!(!ep.subscribes_to(&WebhookEventType::NodeStatusChanged));
    }

    #[test]
    fn test_endpoint_disabled() {
        let mut ep = make_endpoint("ep3", vec![]);
        ep.enabled = false;
        assert!(!ep.subscribes_to(&WebhookEventType::TransactionConfirmed));
    }

    #[test]
    fn test_node_status_changed_event() {
        let event = WebhookEvent::node_status_changed("stellar", "my-validator", "Ready");
        assert_eq!(event.event_type, WebhookEventType::NodeStatusChanged);
        assert_eq!(event.payload["namespace"], "stellar");
        assert_eq!(event.payload["name"], "my-validator");
        assert_eq!(event.payload["new_phase"], "Ready");
        assert!(!event.id.is_empty());
    }

    #[test]
    fn test_transaction_confirmed_event() {
        let event = WebhookEvent::transaction_confirmed("stellar", "my-node", "abc123", 123456);
        assert_eq!(event.event_type, WebhookEventType::TransactionConfirmed);
        assert_eq!(event.payload["tx_hash"], "abc123");
        assert_eq!(event.payload["ledger"], 123456u64);
    }

    #[test]
    fn test_transaction_failed_event() {
        let event =
            WebhookEvent::transaction_failed("stellar", "my-node", "def456", "insufficient_fee");
        assert_eq!(event.event_type, WebhookEventType::TransactionFailed);
        assert_eq!(event.payload["reason"], "insufficient_fee");
    }

    #[test]
    fn test_sign_payload() {
        let sig = sign_payload("secret", b"hello").unwrap();
        assert!(!sig.is_empty());
        // Same input → same output (deterministic HMAC).
        let sig2 = sign_payload("secret", b"hello").unwrap();
        assert_eq!(sig, sig2);
        // Different secret → different output.
        let sig3 = sign_payload("other-secret", b"hello").unwrap();
        assert_ne!(sig, sig3);
    }

    #[test]
    fn test_generate_uuid_uniqueness() {
        let id1 = generate_uuid();
        let id2 = generate_uuid();
        assert_ne!(id1, id2);
        // Rough format check: 8-4-4-4-12 hex groups
        let parts: Vec<&str> = id1.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
    }

    #[tokio::test]
    async fn test_register_and_list_endpoints() {
        let svc = WebhookDeliveryService::new();
        assert!(svc.list_endpoints().await.is_empty());

        let ep = make_endpoint("ep1", vec![WebhookEventType::TransactionConfirmed]);
        svc.register_endpoint(ep.clone()).await;
        assert_eq!(svc.list_endpoints().await.len(), 1);

        // Re-registering the same id replaces it.
        svc.register_endpoint(ep).await;
        assert_eq!(svc.list_endpoints().await.len(), 1);

        svc.unregister_endpoint("ep1").await;
        assert!(svc.list_endpoints().await.is_empty());
    }

    #[tokio::test]
    async fn test_delivery_log_starts_empty() {
        let svc = WebhookDeliveryService::new();
        assert!(svc.delivery_log().await.is_empty());
    }

    #[test]
    fn test_reconcile_completed_event() {
        let event = WebhookEvent::reconcile_completed("ns", "node", true, None);
        assert_eq!(event.event_type, WebhookEventType::ReconcileCompleted);
        assert_eq!(event.payload["success"], true);
        assert!(event.payload["error"].is_null());
    }

    #[test]
    fn test_admin_action_event() {
        let event = WebhookEvent::admin_action("admin-user", "set-log-level", "operator");
        assert_eq!(event.event_type, WebhookEventType::AdminAction);
        assert_eq!(event.payload["actor"], "admin-user");
    }
}
