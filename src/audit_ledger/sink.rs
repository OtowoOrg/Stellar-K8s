// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Out-of-process audit webhook sink.
//!
//! Receives `audit.k8s.io/v1` `EventList` payloads from the kube-apiserver
//! webhook backend, keeps only **mutating** events, strips request/response
//! bodies, and appends them to the hash-chained ledger.
//!
//! Delivery semantics:
//!
//! * **Ack after fsync.** `200` is returned only after the batch is durable.
//! * **Backpressure.** The ingest queue is bounded; when full the sink answers
//!   `503` + `Retry-After` so the apiserver retries instead of events being
//!   dropped here.
//! * **Duplicates.** Re-delivered events (same `auditID` + `stage`) inside the
//!   dedupe window are acknowledged but not appended again.
//! * **Ordering.** A single writer assigns sequence numbers in arrival order.
//! * **No silent loss.** Invalid events are counted (`..._events_invalid_total`)
//!   and logged by `auditID`; write failures roll back and return `500`.

use super::store::{AppendOutcome, LedgerError, LedgerWriter};
use serde_json::{json, Map, Value};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};

/// Verbs that change cluster state.
pub const MUTATING_VERBS: &[&str] = &["create", "update", "patch", "delete", "deletecollection"];
const STAGES: &[&str] = &[
    "RequestReceived",
    "ResponseStarted",
    "ResponseComplete",
    "Panic",
];
const MAX_ID_LEN: usize = 128;

/// Why an event was not accepted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Reject {
    #[error("missing or invalid field `{0}`")]
    Field(&'static str),
    #[error("unknown stage {0:?}")]
    Stage(String),
}

/// Controls what is kept from each event.
#[derive(Debug, Clone, Default)]
pub struct NormalizeConfig {
    /// Keep `sourceIPs` (off by default: personal data).
    pub include_source_ips: bool,
}

fn s<'a>(v: &'a Value, k: &'static str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// Reduce a raw Kubernetes audit event to the stored form.
///
/// Returns `Ok(None)` for non-mutating verbs (expected, counted as filtered).
/// Request/response bodies and annotations are never copied; the query string of
/// `requestURI` is dropped because it can carry tokens or selectors.
pub fn normalize(ev: &Value, cfg: &NormalizeConfig) -> Result<Option<Value>, Reject> {
    let verb = s(ev, "verb").ok_or(Reject::Field("verb"))?;
    if !MUTATING_VERBS.contains(&verb) {
        return Ok(None);
    }
    let audit_id = s(ev, "auditID")
        .filter(|v| !v.is_empty() && v.len() <= MAX_ID_LEN)
        .ok_or(Reject::Field("auditID"))?;
    let stage = s(ev, "stage").ok_or(Reject::Field("stage"))?;
    if !STAGES.contains(&stage) {
        return Err(Reject::Stage(stage.to_string()));
    }
    let mut out = Map::new();
    out.insert("auditID".into(), json!(audit_id));
    out.insert("stage".into(), json!(stage));
    out.insert("verb".into(), json!(verb));
    if let Some(l) = s(ev, "level") {
        out.insert("level".into(), json!(l));
    }
    if let Some(u) = s(ev, "requestURI") {
        out.insert("requestURI".into(), json!(u.split('?').next().unwrap_or(u)));
    }
    let user = ev.get("user").ok_or(Reject::Field("user"))?;
    let username = s(user, "username").ok_or(Reject::Field("user.username"))?;
    let groups: Vec<&str> = user
        .get("groups")
        .and_then(Value::as_array)
        .map(|g| g.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    out.insert(
        "user".into(),
        json!({"username": username, "groups": groups}),
    );
    if cfg.include_source_ips {
        if let Some(ips) = ev.get("sourceIPs").and_then(Value::as_array) {
            let ips: Vec<&str> = ips.iter().filter_map(Value::as_str).collect();
            out.insert("sourceIPs".into(), json!(ips));
        }
    }
    if let Some(o) = ev.get("objectRef").and_then(Value::as_object) {
        let mut r = Map::new();
        for k in [
            "apiGroup",
            "apiVersion",
            "resource",
            "subresource",
            "namespace",
            "name",
        ] {
            if let Some(v) = o.get(k).and_then(Value::as_str) {
                r.insert(k.into(), json!(v));
            }
        }
        out.insert("objectRef".into(), Value::Object(r));
    }
    if let Some(code) = ev
        .get("responseStatus")
        .and_then(|r| r.get("code"))
        .and_then(Value::as_i64)
    {
        out.insert("responseStatus".into(), json!({"code": code}));
    }
    for k in ["requestReceivedTimestamp", "stageTimestamp"] {
        if let Some(t) = s(ev, k) {
            out.insert(k.into(), json!(t));
        }
    }
    Ok(Some(Value::Object(out)))
}

/// Extract events from an `EventList` or a single `Event` document.
pub fn extract_events(doc: &Value) -> Option<Vec<&Value>> {
    if let Some(items) = doc.get("items").and_then(Value::as_array) {
        return Some(items.iter().collect());
    }
    if doc.get("auditID").is_some() {
        return Some(vec![doc]);
    }
    None
}

// ───────────────────────────── metrics ─────────────────────────────

const LAT_BUCKETS_MS: [u64; 9] = [1, 5, 10, 25, 50, 100, 250, 1000, 5000];

/// Lock-free counters exposed in Prometheus text format.
#[derive(Default)]
pub struct SinkMetrics {
    pub events_received: AtomicU64,
    pub events_appended: AtomicU64,
    pub events_duplicate: AtomicU64,
    pub events_filtered: AtomicU64,
    pub events_invalid: AtomicU64,
    pub requests_backpressured: AtomicU64,
    pub write_failures: AtomicU64,
    pub batches: AtomicU64,
    pub queue_depth: AtomicI64,
    pub head_seq: AtomicI64,
    /// 1 if the last write succeeded (or none attempted), 0 if the store is failing.
    pub sink_up: AtomicU64,
    pub last_append_unix: AtomicU64,
    lat_buckets: [AtomicU64; 9],
    lat_count: AtomicU64,
    lat_sum_us: AtomicU64,
    // monitor
    pub chain_valid: AtomicU64,
    pub chain_gaps: AtomicU64,
    pub verification_failures: AtomicU64,
    pub verified_seq: AtomicI64,
    pub last_scan_unix: AtomicU64,
    pub scan_errors: AtomicU64,
}

impl SinkMetrics {
    pub fn new() -> Arc<Self> {
        let m = Self::default();
        m.sink_up.store(1, Ordering::Relaxed);
        m.chain_valid.store(1, Ordering::Relaxed);
        m.head_seq.store(-1, Ordering::Relaxed);
        m.verified_seq.store(-1, Ordering::Relaxed);
        Arc::new(m)
    }

    pub fn observe_latency(&self, d: Duration) {
        let ms = d.as_millis() as u64;
        for (i, b) in LAT_BUCKETS_MS.iter().enumerate() {
            if ms <= *b {
                self.lat_buckets[i].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.lat_count.fetch_add(1, Ordering::Relaxed);
        self.lat_sum_us
            .fetch_add(d.as_micros() as u64, Ordering::Relaxed);
    }

    /// Prometheus text exposition.
    pub fn render(&self) -> String {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let mut o = String::new();
        let mut c = |name: &str, help: &str, kind: &str, v: i64| {
            let _ = writeln!(o, "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {v}");
        };
        c(
            "stellar_audit_ledger_events_received_total",
            "Audit events received from the apiserver.",
            "counter",
            g(&self.events_received) as i64,
        );
        c(
            "stellar_audit_ledger_events_appended_total",
            "Audit events appended to the ledger.",
            "counter",
            g(&self.events_appended) as i64,
        );
        c(
            "stellar_audit_ledger_events_duplicate_total",
            "Duplicate deliveries acknowledged without appending.",
            "counter",
            g(&self.events_duplicate) as i64,
        );
        c(
            "stellar_audit_ledger_events_filtered_total",
            "Non-mutating events filtered out.",
            "counter",
            g(&self.events_filtered) as i64,
        );
        c(
            "stellar_audit_ledger_events_invalid_total",
            "Events rejected as invalid (never silently dropped).",
            "counter",
            g(&self.events_invalid) as i64,
        );
        c(
            "stellar_audit_ledger_requests_backpressured_total",
            "Requests answered 503 because the queue was full.",
            "counter",
            g(&self.requests_backpressured) as i64,
        );
        c(
            "stellar_audit_ledger_write_failures_total",
            "Batches that failed to persist (delivery failures).",
            "counter",
            g(&self.write_failures) as i64,
        );
        c(
            "stellar_audit_ledger_batches_total",
            "Committed group-commit batches.",
            "counter",
            g(&self.batches) as i64,
        );
        c(
            "stellar_audit_ledger_queue_depth",
            "Requests waiting for the writer.",
            "gauge",
            self.queue_depth.load(Ordering::Relaxed),
        );
        c(
            "stellar_audit_ledger_head_sequence",
            "Sequence number of the newest committed entry (-1 if empty).",
            "gauge",
            self.head_seq.load(Ordering::Relaxed),
        );
        c(
            "stellar_audit_ledger_sink_up",
            "1 if the ledger store accepted the last write.",
            "gauge",
            g(&self.sink_up) as i64,
        );
        c(
            "stellar_audit_ledger_last_append_timestamp_seconds",
            "Unix time of the last successful append.",
            "gauge",
            g(&self.last_append_unix) as i64,
        );
        c(
            "stellar_audit_ledger_chain_valid",
            "1 if the incremental monitor has found no integrity violation.",
            "gauge",
            g(&self.chain_valid) as i64,
        );
        c(
            "stellar_audit_ledger_chain_gaps_total",
            "Sequence gaps detected by the monitor.",
            "counter",
            g(&self.chain_gaps) as i64,
        );
        c(
            "stellar_audit_ledger_verification_failures_total",
            "Hash/continuity verification failures detected.",
            "counter",
            g(&self.verification_failures) as i64,
        );
        c(
            "stellar_audit_ledger_verified_sequence",
            "Highest sequence verified by the monitor (-1 if none).",
            "gauge",
            self.verified_seq.load(Ordering::Relaxed),
        );
        c(
            "stellar_audit_ledger_last_scan_timestamp_seconds",
            "Unix time of the last completed monitor scan.",
            "gauge",
            g(&self.last_scan_unix) as i64,
        );
        c(
            "stellar_audit_ledger_scan_errors_total",
            "Monitor scans that failed with an I/O error.",
            "counter",
            g(&self.scan_errors) as i64,
        );
        let name = "stellar_audit_ledger_processing_latency_seconds";
        let _ = writeln!(o, "# HELP {name} Time from request receipt to durable acknowledgement.\n# TYPE {name} histogram");
        for (i, b) in LAT_BUCKETS_MS.iter().enumerate() {
            let _ = writeln!(
                o,
                "{name}_bucket{{le=\"{}\"}} {}",
                *b as f64 / 1000.0,
                g(&self.lat_buckets[i])
            );
        }
        let _ = writeln!(o, "{name}_bucket{{le=\"+Inf\"}} {}", g(&self.lat_count));
        let _ = writeln!(
            o,
            "{name}_sum {}\n{name}_count {}",
            g(&self.lat_sum_us) as f64 / 1e6,
            g(&self.lat_count)
        );
        o
    }
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ───────────────────────────── service ─────────────────────────────

/// Sink configuration.
#[derive(Debug, Clone)]
pub struct SinkConfig {
    pub path: PathBuf,
    pub chain_id: String,
    /// Max queued requests before answering 503.
    pub queue_capacity: usize,
    /// Max requests merged into one fsync.
    pub max_batch: usize,
    pub dedupe_window: usize,
    pub normalize: NormalizeConfig,
}

impl SinkConfig {
    pub fn new(path: impl Into<PathBuf>, chain_id: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            chain_id: chain_id.into(),
            queue_capacity: 1024,
            max_batch: 256,
            dedupe_window: 200_000,
            normalize: NormalizeConfig::default(),
        }
    }
}

/// Why a submission was not durably accepted. Both variants are retryable.
#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("ingest queue full")]
    Backpressure,
    #[error("ledger write failed: {0}")]
    WriteFailed(String),
}

pub struct Submit {
    pub events: Vec<Value>,
    pub ack: oneshot::Sender<Result<AppendOutcome, String>>,
}

/// Cloneable handle to the writer thread.
#[derive(Clone)]
pub struct SinkHandle {
    #[doc(hidden)]
    pub tx: mpsc::Sender<Submit>,
    pub metrics: Arc<SinkMetrics>,
    pub config: Arc<SinkConfig>,
}

impl SinkHandle {
    /// Normalize raw apiserver events (counting filtered/invalid) and persist them.
    pub async fn ingest_raw(&self, raw: &[&Value]) -> Result<AppendOutcome, SubmitError> {
        let started = Instant::now();
        let m = &self.metrics;
        m.events_received
            .fetch_add(raw.len() as u64, Ordering::Relaxed);
        let mut events = Vec::with_capacity(raw.len());
        for ev in raw {
            match normalize(ev, &self.config.normalize) {
                Ok(Some(e)) => events.push(e),
                Ok(None) => {
                    m.events_filtered.fetch_add(1, Ordering::Relaxed);
                }
                Err(r) => {
                    m.events_invalid.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(audit_id = s(ev, "auditID").unwrap_or("?"), reason = %r, "rejected invalid audit event");
                }
            }
        }
        if events.is_empty() {
            return Ok(AppendOutcome::default());
        }
        let r = self.submit(events).await;
        m.observe_latency(started.elapsed());
        r
    }

    /// Persist already-normalized events.
    pub async fn submit(&self, events: Vec<Value>) -> Result<AppendOutcome, SubmitError> {
        let (ack, rx) = oneshot::channel();
        self.metrics.queue_depth.fetch_add(1, Ordering::Relaxed);
        if self.tx.try_send(Submit { events, ack }).is_err() {
            self.metrics.queue_depth.fetch_sub(1, Ordering::Relaxed);
            self.metrics
                .requests_backpressured
                .fetch_add(1, Ordering::Relaxed);
            return Err(SubmitError::Backpressure);
        }
        match rx.await {
            Ok(Ok(o)) => Ok(o),
            Ok(Err(e)) => Err(SubmitError::WriteFailed(e)),
            Err(_) => Err(SubmitError::WriteFailed("writer thread terminated".into())),
        }
    }
}

/// Open the ledger and start the writer thread.
pub fn spawn_sink(
    cfg: SinkConfig,
) -> Result<(SinkHandle, std::thread::JoinHandle<()>), LedgerError> {
    let writer = LedgerWriter::open(&cfg.path, &cfg.chain_id, cfg.dedupe_window)?;
    spawn_with_writer(cfg, writer)
}

pub fn spawn_with_writer(
    cfg: SinkConfig,
    mut writer: LedgerWriter,
) -> Result<(SinkHandle, std::thread::JoinHandle<()>), LedgerError> {
    let metrics = SinkMetrics::new();
    metrics
        .head_seq
        .store(writer.next_seq() as i64 - 1, Ordering::Relaxed);
    let (tx, mut rx) = mpsc::channel::<Submit>(cfg.queue_capacity.max(1));
    let max_batch = cfg.max_batch.max(1);
    let m = Arc::clone(&metrics);
    let join = std::thread::Builder::new()
        .name("audit-ledger-writer".into())
        .spawn(move || {
            while let Some(first) = rx.blocking_recv() {
                let mut batch = vec![first];
                while batch.len() < max_batch {
                    match rx.try_recv() {
                        Ok(s) => batch.push(s),
                        Err(_) => break,
                    }
                }
                m.queue_depth.fetch_sub(batch.len() as i64, Ordering::Relaxed);
                let mut outcomes = Vec::with_capacity(batch.len());
                let mut failure: Option<String> = None;
                for sub in &batch {
                    match writer.stage(&sub.events) {
                        Ok(o) => outcomes.push(o),
                        Err(e) => {
                            failure = Some(e.to_string());
                            break;
                        }
                    }
                }
                if failure.is_none() {
                    if let Err(e) = writer.commit() {
                        failure = Some(e.to_string());
                    }
                }
                match failure {
                    Some(msg) => {
                        if let Err(e) = writer.rollback() {
                            tracing::error!(error = %e, "rollback of failed audit batch also failed");
                        }
                        m.write_failures.fetch_add(1, Ordering::Relaxed);
                        m.sink_up.store(0, Ordering::Relaxed);
                        tracing::error!(error = %msg, "audit batch not persisted; senders will retry");
                        for sub in batch {
                            let _ = sub.ack.send(Err(msg.clone()));
                        }
                    }
                    None => {
                        m.sink_up.store(1, Ordering::Relaxed);
                        m.batches.fetch_add(1, Ordering::Relaxed);
                        m.last_append_unix.store(now_unix(), Ordering::Relaxed);
                        m.head_seq.store(writer.next_seq() as i64 - 1, Ordering::Relaxed);
                        for (sub, o) in batch.into_iter().zip(outcomes) {
                            m.events_appended.fetch_add(o.appended, Ordering::Relaxed);
                            m.events_duplicate.fetch_add(o.duplicates, Ordering::Relaxed);
                            let _ = sub.ack.send(Ok(o));
                        }
                    }
                }
            }
        })
        .map_err(LedgerError::Io)?;
    Ok((
        SinkHandle {
            tx,
            metrics,
            config: Arc::new(cfg),
        },
        join,
    ))
}

// ───────────────────────────── HTTP ─────────────────────────────

#[cfg(feature = "axum")]
pub mod http {
    use super::*;
    use axum::body::Bytes;
    use axum::extract::{DefaultBodyLimit, State};
    use axum::http::{header, HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::Router;

    const MAX_BODY: usize = 16 << 20;
    const MAX_EVENTS_PER_REQUEST: usize = 10_000;

    #[derive(Clone)]
    struct AppState {
        sink: SinkHandle,
        token: Option<Arc<String>>,
    }

    /// Router exposing `POST /audit`, `GET /healthz`, `GET /metrics`.
    ///
    /// When `bearer_token` is set, `POST /audit` requires `Authorization: Bearer <token>`.
    pub fn router(sink: SinkHandle, bearer_token: Option<String>) -> Router {
        Router::new()
            .route("/audit", post(audit))
            .route("/healthz", get(healthz))
            .route("/metrics", get(metrics))
            .layer(DefaultBodyLimit::max(MAX_BODY))
            .with_state(AppState {
                sink,
                token: bearer_token.map(Arc::new),
            })
    }

    fn ct_eq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    fn authorized(st: &AppState, h: &HeaderMap) -> bool {
        match &st.token {
            None => true,
            Some(t) => h
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(|p| ct_eq(p.as_bytes(), t.as_bytes()))
                .unwrap_or(false),
        }
    }

    async fn audit(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
        if !authorized(&st, &headers) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let doc: Value = match serde_json::from_slice(&body) {
            Ok(d) => d,
            Err(e) => {
                return (StatusCode::BAD_REQUEST, format!("invalid JSON: {e}")).into_response()
            }
        };
        let Some(events) = extract_events(&doc) else {
            return (
                StatusCode::BAD_REQUEST,
                "expected audit.k8s.io EventList or Event",
            )
                .into_response();
        };
        if events.len() > MAX_EVENTS_PER_REQUEST {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                "too many events in one request",
            )
                .into_response();
        }
        match st.sink.ingest_raw(&events).await {
            Ok(o) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                json!({"appended": o.appended, "duplicates": o.duplicates, "headSeq": o.head_seq})
                    .to_string(),
            )
                .into_response(),
            Err(SubmitError::Backpressure) => (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::RETRY_AFTER, "1")],
                "ingest queue full",
            )
                .into_response(),
            Err(SubmitError::WriteFailed(e)) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::RETRY_AFTER, "1")],
                e,
            )
                .into_response(),
        }
    }

    async fn healthz(State(st): State<AppState>) -> Response {
        if st.sink.metrics.sink_up.load(Ordering::Relaxed) == 1 {
            (StatusCode::OK, "ok").into_response()
        } else {
            (StatusCode::SERVICE_UNAVAILABLE, "ledger store failing").into_response()
        }
    }

    async fn metrics(State(st): State<AppState>) -> Response {
        (
            [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
            st.sink.metrics.render(),
        )
            .into_response()
    }
}
