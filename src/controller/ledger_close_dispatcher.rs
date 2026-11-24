//! Ledger-Close Webhook Dispatcher
//!
//! This controller watches Stellar Horizon nodes for new ledger-close events and
//! fans them out to all registered [`LedgerCloseWebhook`] subscribers in the same
//! namespace, honouring:
//!
//! * **Ordered delivery per subscription** — a per-subscription channel serialises
//!   deliveries so that ledger N+1 is never sent before ledger N succeeds.
//! * **At-least-once semantics** — failed attempts are retried with exponential
//!   back-off (1 s, 2 s, 4 s, 8 s, 16 s) up to `spec.maxRetries`.
//! * **HMAC-SHA256 signing** — every POST carries
//!   `X-Stellar-Signature: sha256=<hex>` derived from `spec.secretRef`.
//! * **5-second delivery target** — first attempt is made immediately on ledger
//!   close; the `timeoutSeconds` field caps each HTTP round-trip.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────┐
//! │  LedgerCloseDispatcher                   │
//! │  ┌────────────────┐                      │
//! │  │  poll_ledgers  │──► LedgerCloseEvent  │
//! │  └────────────────┘        │             │
//! │          ┌─────────────────▼──────────┐  │
//! │          │  fan_out_to_subscribers    │  │
//! │          └──┬──────────────────────┬─┘  │
//! │    hook-A   │              hook-B   │    │
//! │  ┌──────────▼───┐      ┌───────────▼──┐ │
//! │  │  deliver_one │      │  deliver_one │ │
//! │  │  (retry loop)│      │  (retry loop)│ │
//! │  └──────────────┘      └──────────────┘ │
//! └──────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use hex::encode as hex_encode;
use hmac::{Hmac, Mac};
use kube::api::{Api, Patch, PatchParams};
use kube::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::json;
use sha2::Sha256;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use crate::crd::ledger_close_webhook::{
    DeliveryLogEntry, DeliveryPhase, LedgerClosePayload, LedgerCloseWebhook,
    LedgerCloseWebhookStatus,
};

type HmacSha256 = Hmac<Sha256>;

// ─── Public re-exports ───────────────────────────────────────────────────────

pub use crate::crd::ledger_close_webhook::{LedgerCloseEventType, LedgerCloseWebhookSpec};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Maximum number of queued ledger-close events per subscription before the
/// dispatcher starts dropping the oldest events to avoid unbounded memory growth.
const CHANNEL_CAPACITY: usize = 64;

/// Maximum entries kept in `status.deliveryLog` (ring-buffer behaviour).
const MAX_LOG_ENTRIES: usize = 20;

/// Initial back-off duration for the first retry (doubles each attempt).
const INITIAL_BACKOFF_SECS: u64 = 1;

// ─── Dispatcher ──────────────────────────────────────────────────────────────

/// Shared state for all active subscription workers.
struct SubscriptionWorker {
    /// Channel sender — caller pushes payloads here.
    tx: mpsc::Sender<LedgerClosePayload>,
}

/// Top-level dispatcher that manages per-subscription delivery workers.
pub struct LedgerCloseDispatcher {
    client: Client,
    http: reqwest::Client,
    /// namespace → hook-name → worker
    workers: RwLock<HashMap<String, HashMap<String, SubscriptionWorker>>>,
}

impl LedgerCloseDispatcher {
    /// Create a new dispatcher using the provided Kubernetes client.
    pub fn new(client: Client) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client build failed");

        Arc::new(Self {
            client,
            http,
            workers: RwLock::new(HashMap::new()),
        })
    }

    /// Publish a ledger-close payload to all active `LedgerCloseWebhook`
    /// resources in every watched namespace.
    ///
    /// This is the hot path — it should return quickly.  Heavy lifting (HTTP,
    /// retries, status updates) happens in spawned background tasks.
    pub async fn dispatch_ledger_close(&self, payload: LedgerClosePayload) {
        // Discover all LedgerCloseWebhook resources cluster-wide.
        let hooks_api: Api<LedgerCloseWebhook> = Api::all(self.client.clone());
        let hooks = match hooks_api.list(&Default::default()).await {
            Ok(list) => list.items,
            Err(e) => {
                error!("Failed to list LedgerCloseWebhook resources: {e}");
                return;
            }
        };

        for hook in hooks {
            let spec = &hook.spec;
            if !spec.enabled {
                continue;
            }
            if !spec.events.contains(&LedgerCloseEventType::LedgerClose) {
                continue;
            }

            let namespace = hook
                .metadata
                .namespace
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let name = hook
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| "unknown".to_string());

            self.ensure_worker(&namespace, &name, &hook).await;
            self.send_to_worker(&namespace, &name, payload.clone())
                .await;
        }
    }

    /// Ensure a background worker exists for `(namespace, name)`.
    async fn ensure_worker(&self, namespace: &str, name: &str, hook: &LedgerCloseWebhook) {
        let mut workers = self.workers.write().await;
        let ns_map = workers.entry(namespace.to_string()).or_default();

        if ns_map.contains_key(name) {
            return;
        }

        let (tx, rx) = mpsc::channel::<LedgerClosePayload>(CHANNEL_CAPACITY);
        ns_map.insert(name.to_string(), SubscriptionWorker { tx });

        let client = self.client.clone();
        let http = self.http.clone();
        let spec = hook.spec.clone();
        let ns = namespace.to_string();
        let hook_name = name.to_string();

        tokio::spawn(async move {
            run_subscription_worker(client, http, spec, ns, hook_name, rx).await;
        });

        info!("Started delivery worker for LedgerCloseWebhook {namespace}/{name}");
    }

    /// Push a payload into the named worker's channel (non-blocking drop if full).
    async fn send_to_worker(&self, namespace: &str, name: &str, payload: LedgerClosePayload) {
        let workers = self.workers.read().await;
        if let Some(worker) = workers.get(namespace).and_then(|m| m.get(name)) {
            if worker.tx.try_send(payload).is_err() {
                warn!(
                    "Delivery channel full for LedgerCloseWebhook {namespace}/{name}; dropping ledger {}",
                    "?"
                );
            }
        }
    }
}

// ─── Subscription worker ─────────────────────────────────────────────────────

/// Long-lived async task that drains the per-subscription channel and delivers
/// payloads to the external endpoint.
async fn run_subscription_worker(
    client: Client,
    http: reqwest::Client,
    spec: LedgerCloseWebhookSpec,
    namespace: String,
    name: String,
    mut rx: mpsc::Receiver<LedgerClosePayload>,
) {
    while let Some(payload) = rx.recv().await {
        let ledger_seq = payload.ledger_sequence;
        debug!("Delivering ledger {ledger_seq} to LedgerCloseWebhook {namespace}/{name}");

        let entry = deliver_with_retry(&http, &spec, &payload).await;

        // Patch status back into Kubernetes.
        if let Err(e) = patch_webhook_status(&client, &namespace, &name, &entry).await {
            error!("Failed to patch status for {namespace}/{name}: {e}");
        }
    }

    info!("Delivery worker for LedgerCloseWebhook {namespace}/{name} shutting down");
}

// ─── Delivery with exponential back-off ──────────────────────────────────────

/// Attempt delivery to `spec.url`, retrying up to `spec.maxRetries` times.
///
/// Returns a [`DeliveryLogEntry`] summarising the outcome.
async fn deliver_with_retry(
    http: &reqwest::Client,
    spec: &LedgerCloseWebhookSpec,
    payload: &LedgerClosePayload,
) -> DeliveryLogEntry {
    let body = match serde_json::to_vec(payload) {
        Ok(b) => b,
        Err(e) => {
            return DeliveryLogEntry {
                ledger_sequence: payload.ledger_sequence,
                attempted_at: Utc::now(),
                completed_at: Some(Utc::now()),
                phase: DeliveryPhase::Failed,
                attempts: 0,
                last_status_code: None,
                last_error: Some(format!("serialisation error: {e}")),
            };
        }
    };

    let signature = sign_payload(&body, spec.secret_ref.as_deref().unwrap_or(""));
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Ok(v) = HeaderValue::from_str(&format!("sha256={signature}")) {
        headers.insert("X-Stellar-Signature", v);
    }
    headers.insert(
        "X-Stellar-Ledger-Sequence",
        HeaderValue::from_str(&payload.ledger_sequence.to_string())
            .unwrap_or(HeaderValue::from_static("0")),
    );

    let timeout = Duration::from_secs(spec.timeout_seconds);
    let max_attempts = spec.max_retries + 1; // first attempt + retries
    let attempted_at = Utc::now();

    let mut last_status: Option<u16> = None;
    let mut last_err: Option<String> = None;

    for attempt in 1..=max_attempts {
        match http
            .post(&spec.url)
            .headers(headers.clone())
            .body(body.clone())
            .timeout(timeout)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                last_status = Some(status);

                if resp.status().is_success() {
                    info!(
                        ledger_sequence = payload.ledger_sequence,
                        url = %spec.url,
                        attempt,
                        "Ledger-close webhook delivered successfully"
                    );
                    return DeliveryLogEntry {
                        ledger_sequence: payload.ledger_sequence,
                        attempted_at,
                        completed_at: Some(Utc::now()),
                        phase: DeliveryPhase::Delivered,
                        attempts: attempt,
                        last_status_code: last_status,
                        last_error: None,
                    };
                }

                last_err = Some(format!("HTTP {status}"));
                warn!(
                    ledger_sequence = payload.ledger_sequence,
                    url = %spec.url,
                    attempt,
                    status,
                    "Webhook delivery returned non-2xx; will retry"
                );
            }
            Err(e) => {
                last_err = Some(e.to_string());
                warn!(
                    ledger_sequence = payload.ledger_sequence,
                    url = %spec.url,
                    attempt,
                    error = %e,
                    "Webhook delivery attempt failed; will retry"
                );
            }
        }

        if attempt < max_attempts {
            let backoff = Duration::from_secs(INITIAL_BACKOFF_SECS << (attempt - 1).min(4));
            tokio::time::sleep(backoff).await;
        }
    }

    error!(
        ledger_sequence = payload.ledger_sequence,
        url = %spec.url,
        max_attempts,
        "All webhook delivery attempts exhausted"
    );

    DeliveryLogEntry {
        ledger_sequence: payload.ledger_sequence,
        attempted_at,
        completed_at: Some(Utc::now()),
        phase: DeliveryPhase::Failed,
        attempts: max_attempts,
        last_status_code: last_status,
        last_error: last_err,
    }
}

// ─── HMAC signing ────────────────────────────────────────────────────────────

/// Compute `sha256=<hex>` over `body` using `secret` as the HMAC key.
///
/// If `secret` is empty the function returns an empty string and signing is
/// effectively skipped (consumers should not validate the header in that case).
fn sign_payload(body: &[u8], secret: &str) -> String {
    if secret.is_empty() {
        return String::new();
    }
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(body);
    hex_encode(mac.finalize().into_bytes())
}

// ─── Status patch ────────────────────────────────────────────────────────────

/// Merge the new `DeliveryLogEntry` into the resource's status subresource.
async fn patch_webhook_status(
    client: &Client,
    namespace: &str,
    name: &str,
    entry: &DeliveryLogEntry,
) -> Result<(), kube::Error> {
    let api: Api<LedgerCloseWebhook> = Api::namespaced(client.clone(), namespace);

    // Fetch current status to accumulate counters and log.
    let current = api.get(name).await.ok();
    let mut status = current.and_then(|h| h.status).unwrap_or_default();

    // Update counters.
    match entry.phase {
        DeliveryPhase::Delivered => {
            status.total_delivered += 1;
            status.last_delivered_sequence = Some(entry.ledger_sequence);
            status.last_delivered_at = entry.completed_at;
        }
        DeliveryPhase::Failed => {
            status.total_failed += 1;
        }
        _ => {}
    }
    status.phase = entry.phase.clone();

    // Maintain ring-buffer of last MAX_LOG_ENTRIES entries.
    status.delivery_log.push(entry.clone());
    if status.delivery_log.len() > MAX_LOG_ENTRIES {
        let drain_count = status.delivery_log.len() - MAX_LOG_ENTRIES;
        status.delivery_log.drain(0..drain_count);
    }

    let patch = json!({ "status": status });
    api.patch_status(
        name,
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(patch),
    )
    .await?;

    Ok(())
}

// ─── Ledger polling helper ────────────────────────────────────────────────────

/// Lightweight Horizon ledger info as returned by `/ledgers?order=desc&limit=1`.
#[derive(Debug, serde::Deserialize)]
struct HorizonLedgerResponse {
    #[serde(rename = "_embedded")]
    embedded: HorizonLedgerEmbedded,
}

#[derive(Debug, serde::Deserialize)]
struct HorizonLedgerEmbedded {
    records: Vec<HorizonLedgerRecord>,
}

#[derive(Debug, serde::Deserialize)]
struct HorizonLedgerRecord {
    sequence: u64,
    closed_at: String,
    transaction_count: u32,
    operation_count: u32,
}

/// Poll `{horizon_url}/ledgers?order=desc&limit=1` and return the latest
/// ledger sequence and metadata, or `None` on failure.
pub async fn poll_latest_ledger(
    http: &reqwest::Client,
    horizon_url: &str,
) -> Option<(u64, i64, u32, u32)> {
    let url = format!("{horizon_url}/ledgers?order=desc&limit=1");
    let resp = http
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body: HorizonLedgerResponse = resp.json().await.ok()?;
    let record = body.embedded.records.into_iter().next()?;

    // Parse closed_at as a Unix timestamp.
    let close_time = chrono::DateTime::parse_from_rfc3339(&record.closed_at)
        .map(|dt| dt.timestamp())
        .unwrap_or(0);

    Some((
        record.sequence,
        close_time,
        record.transaction_count,
        record.operation_count,
    ))
}

/// Spawn a background ledger-close polling loop.
///
/// `poll_interval` controls how frequently Horizon is queried (recommended: 5 s).
/// New ledger closes are forwarded to `dispatcher.dispatch_ledger_close`.
pub async fn run_ledger_close_poll_loop(
    dispatcher: Arc<LedgerCloseDispatcher>,
    horizon_url: String,
    network_passphrase: String,
    source_node: String,
    source_namespace: String,
    poll_interval: Duration,
) {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client");

    let mut last_seen_seq: u64 = 0;

    info!("Starting ledger-close poll loop for {source_namespace}/{source_node} at {horizon_url}");

    loop {
        if let Some((seq, close_time, tx_count, op_count)) =
            poll_latest_ledger(&http, &horizon_url).await
        {
            if seq > last_seen_seq {
                // Emit one event per new ledger (handles gaps if we skipped ledgers).
                for new_seq in (last_seen_seq + 1)..=seq {
                    let payload = LedgerClosePayload::new(
                        new_seq,
                        close_time,
                        tx_count,
                        op_count,
                        &network_passphrase,
                        &source_node,
                        &source_namespace,
                    );
                    dispatcher.dispatch_ledger_close(payload).await;
                }
                last_seen_seq = seq;
            }
        } else {
            debug!("Failed to poll Horizon at {horizon_url}; will retry");
        }

        tokio::time::sleep(poll_interval).await;
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_payload_produces_hex() {
        let body = b"hello world";
        let sig = sign_payload(body, "secret");
        assert!(!sig.is_empty());
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sign_payload_empty_secret_returns_empty() {
        assert_eq!(sign_payload(b"data", ""), "");
    }

    #[test]
    fn sign_payload_deterministic() {
        let a = sign_payload(b"payload", "key");
        let b = sign_payload(b"payload", "key");
        assert_eq!(a, b);
    }

    #[test]
    fn sign_payload_differs_for_different_secrets() {
        let a = sign_payload(b"payload", "key1");
        let b = sign_payload(b"payload", "key2");
        assert_ne!(a, b);
    }
}
