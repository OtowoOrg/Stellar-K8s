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
//! Distributed rate limiting across API gateway instances (issue #1335).
//!
//! [`super::ratelimit::RateLimiter`] keeps its token buckets in a process-local
//! `HashMap`. With N gateway replicas behind a load balancer that lets through
//! roughly N times the configured limit, because each replica only ever sees
//! its own share of the traffic. This module moves the counters into a shared
//! store so the limit is enforced across the fleet.
//!
//! # How instances stay in sync
//!
//! There is no gossip, leader, or replication protocol. Every instance derives
//! the *same* counter key from the request identity and the wall-clock window:
//!
//! ```text
//! {prefix}:{scope}:{identifier}:{window_start_epoch_seconds}
//! ```
//!
//! Because `window_start` is `now - (now % window)`, every replica computes an
//! identical key for the same request within the same window, and they all
//! increment one shared counter. Synchronisation falls out of the key
//! derivation instead of being a protocol that can lag or split-brain.
//!
//! # Atomicity and overhead
//!
//! Each check is a single Redis round trip running a small Lua script:
//!
//! ```lua
//! local c = redis.call('INCR', KEYS[1])
//! if c == 1 then redis.call('PEXPIRE', KEYS[1], ARGV[1]) end
//! return c
//! ```
//!
//! `INCR` and the TTL set are one atomic server-side operation, so concurrent
//! replicas cannot race to create a counter that never expires. One round trip
//! against a same-cluster Redis is well under the 1ms budget; the check
//! duration is recorded in a histogram so the budget can be asserted rather
//! than assumed.
//!
//! # Failure behaviour
//!
//! The store is a dependency of the *request path*, so it fails open: if Redis
//! is unreachable, the limiter falls back to a process-local counter, records
//! the failure on [`RateLimitMetrics::backend_errors`], and keeps serving.
//! Rejecting production traffic because a rate-limit counter is unavailable
//! trades an availability incident for a capacity one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::Histogram;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Lua script executed for every check. Atomic INCR plus first-write TTL.
const INCR_WITH_TTL_SCRIPT: &str =
    "local c = redis.call('INCR', KEYS[1]) if c == 1 then redis.call('PEXPIRE', KEYS[1], ARGV[1]) end return c";

/// Errors raised by a [`DistributedCounterStore`].
#[derive(Debug, Error)]
pub enum StoreError {
    /// The store could not be reached or the connection dropped mid-command.
    #[error("rate limit store unavailable: {0}")]
    Unavailable(String),
    /// The store returned a reply this client cannot interpret.
    #[error("rate limit store protocol error: {0}")]
    Protocol(String),
}

/// A shared counter store backing distributed rate limits.
///
/// Implementations must make [`Self::increment`] atomic: two concurrent
/// callers must never both observe the same returned count.
#[async_trait]
pub trait DistributedCounterStore: Send + Sync {
    /// Increment `key`, setting `ttl` on first creation, and return the new
    /// value. Returning `1` means this call created the counter.
    async fn increment(&self, key: &str, ttl: Duration) -> Result<u64, StoreError>;

    /// Read `key` without incrementing. Returns 0 when absent.
    async fn get(&self, key: &str) -> Result<u64, StoreError>;

    /// Short name used in logs and metric labels.
    fn backend_name(&self) -> &'static str;
}

// ─────────────────────────────────────────────────────────────────────────────
// In-memory store
// ─────────────────────────────────────────────────────────────────────────────

/// Process-local store. Correct for a single replica, and used as the
/// fail-open fallback when the shared store is unreachable.
#[derive(Default)]
pub struct InMemoryCounterStore {
    entries: Mutex<HashMap<String, (u64, Instant)>>,
}

impl InMemoryCounterStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop entries whose TTL has elapsed. Called on every increment, so the
    /// map cannot grow without bound under key churn.
    fn purge_expired(entries: &mut HashMap<String, (u64, Instant)>, now: Instant) {
        entries.retain(|_, (_, expires_at)| *expires_at > now);
    }
}

#[async_trait]
impl DistributedCounterStore for InMemoryCounterStore {
    async fn increment(&self, key: &str, ttl: Duration) -> Result<u64, StoreError> {
        let now = Instant::now();
        let mut entries = self.entries.lock().await;
        Self::purge_expired(&mut entries, now);
        let entry = entries.entry(key.to_string()).or_insert((0, now + ttl));
        entry.0 += 1;
        Ok(entry.0)
    }

    async fn get(&self, key: &str) -> Result<u64, StoreError> {
        let now = Instant::now();
        let entries = self.entries.lock().await;
        Ok(entries
            .get(key)
            .filter(|(_, expires_at)| *expires_at > now)
            .map(|(count, _)| *count)
            .unwrap_or(0))
    }

    fn backend_name(&self) -> &'static str {
        "memory"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Redis store (RESP over TCP, no extra dependency)
// ─────────────────────────────────────────────────────────────────────────────

/// One RESP reply value. Only the shapes this client can receive are modelled.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Integer(i64),
    Simple(String),
    Bulk(Option<String>),
    Error(String),
}

/// Encode a command as a RESP array of bulk strings.
fn encode_command(args: &[&str]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + args.iter().map(|a| a.len() + 16).sum::<usize>());
    out.extend_from_slice(format!("*{}\r\n", args.len()).as_bytes());
    for arg in args {
        out.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
        out.extend_from_slice(arg.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Read one RESP reply from `reader`.
async fn read_reply<R>(reader: &mut BufReader<R>) -> Result<Reply, StoreError>
where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .await
        .map_err(|e| StoreError::Unavailable(e.to_string()))?;
    if read == 0 {
        return Err(StoreError::Unavailable("connection closed".into()));
    }
    let line = line.trim_end_matches(['\r', '\n']);
    let (tag, rest) = line
        .split_at_checked(1)
        .ok_or_else(|| StoreError::Protocol("empty reply".into()))?;

    match tag {
        ":" => rest
            .parse::<i64>()
            .map(Reply::Integer)
            .map_err(|_| StoreError::Protocol(format!("bad integer reply: {rest}"))),
        "+" => Ok(Reply::Simple(rest.to_string())),
        "-" => Ok(Reply::Error(rest.to_string())),
        "$" => {
            let len: i64 = rest
                .parse()
                .map_err(|_| StoreError::Protocol(format!("bad bulk length: {rest}")))?;
            if len < 0 {
                return Ok(Reply::Bulk(None));
            }
            let mut buf = vec![0u8; len as usize + 2]; // payload + CRLF
            tokio::io::AsyncReadExt::read_exact(reader, &mut buf)
                .await
                .map_err(|e| StoreError::Unavailable(e.to_string()))?;
            buf.truncate(len as usize);
            String::from_utf8(buf)
                .map(|s| Reply::Bulk(Some(s)))
                .map_err(|e| StoreError::Protocol(e.to_string()))
        }
        other => Err(StoreError::Protocol(format!(
            "unsupported reply type '{other}'"
        ))),
    }
}

/// A pooled Redis connection.
struct RedisConnection {
    reader: BufReader<TcpStream>,
}

impl RedisConnection {
    async fn connect(address: &str, timeout: Duration) -> Result<Self, StoreError> {
        let stream = tokio::time::timeout(timeout, TcpStream::connect(address))
            .await
            .map_err(|_| StoreError::Unavailable(format!("connect to {address} timed out")))?
            .map_err(|e| StoreError::Unavailable(format!("connect to {address}: {e}")))?;
        // Rate-limit checks are tiny and latency-critical; Nagle would batch
        // them into the next ACK and blow the sub-millisecond budget.
        let _ = stream.set_nodelay(true);
        Ok(Self {
            reader: BufReader::new(stream),
        })
    }

    async fn command(&mut self, args: &[&str], timeout: Duration) -> Result<Reply, StoreError> {
        let payload = encode_command(args);
        tokio::time::timeout(timeout, async {
            self.reader
                .get_mut()
                .write_all(&payload)
                .await
                .map_err(|e| StoreError::Unavailable(e.to_string()))?;
            read_reply(&mut self.reader).await
        })
        .await
        .map_err(|_| StoreError::Unavailable("command timed out".into()))?
    }
}

/// Configuration for [`RedisCounterStore`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisStoreConfig {
    /// `host:port` of the Redis endpoint.
    pub address: String,
    /// Maximum idle connections kept for reuse.
    pub pool_size: usize,
    /// Deadline for connecting and for each command.
    pub timeout: Duration,
}

impl Default for RedisStoreConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1:6379".to_string(),
            pool_size: 8,
            // Deliberately tight: the limiter fails open, so waiting longer
            // than this only delays the request it was meant to protect.
            timeout: Duration::from_millis(50),
        }
    }
}

/// Redis-backed shared counter store.
///
/// Speaks RESP directly over a pooled `TcpStream` rather than pulling in a
/// Redis client crate: the two commands this needs are a handful of lines, and
/// the request path stays free of an extra dependency tree.
pub struct RedisCounterStore {
    config: RedisStoreConfig,
    pool: Mutex<Vec<RedisConnection>>,
}

impl RedisCounterStore {
    /// Create a store. No connection is opened until the first command.
    pub fn new(config: RedisStoreConfig) -> Self {
        Self {
            config,
            pool: Mutex::new(Vec::new()),
        }
    }

    /// Run one command, taking a pooled connection or opening a new one.
    ///
    /// A connection that errors is dropped rather than returned to the pool,
    /// so a half-open socket cannot poison later requests.
    async fn run(&self, args: &[&str]) -> Result<Reply, StoreError> {
        let mut conn = match self.pool.lock().await.pop() {
            Some(conn) => conn,
            None => RedisConnection::connect(&self.config.address, self.config.timeout).await?,
        };

        match conn.command(args, self.config.timeout).await {
            Ok(reply) => {
                let mut pool = self.pool.lock().await;
                if pool.len() < self.config.pool_size {
                    pool.push(conn);
                }
                Ok(reply)
            }
            Err(err) => Err(err),
        }
    }
}

#[async_trait]
impl DistributedCounterStore for RedisCounterStore {
    async fn increment(&self, key: &str, ttl: Duration) -> Result<u64, StoreError> {
        let ttl_ms = ttl.as_millis().max(1).to_string();
        let reply = self
            .run(&[
                "EVAL",
                INCR_WITH_TTL_SCRIPT,
                "1", // one KEYS entry
                key,
                &ttl_ms,
            ])
            .await?;
        match reply {
            Reply::Integer(n) if n >= 0 => Ok(n as u64),
            Reply::Error(e) => Err(StoreError::Protocol(e)),
            other => Err(StoreError::Protocol(format!("unexpected reply: {other:?}"))),
        }
    }

    async fn get(&self, key: &str) -> Result<u64, StoreError> {
        match self.run(&["GET", key]).await? {
            Reply::Bulk(None) => Ok(0),
            Reply::Bulk(Some(v)) => v
                .parse()
                .map_err(|_| StoreError::Protocol(format!("non-numeric counter: {v}"))),
            Reply::Integer(n) if n >= 0 => Ok(n as u64),
            Reply::Error(e) => Err(StoreError::Protocol(e)),
            other => Err(StoreError::Protocol(format!("unexpected reply: {other:?}"))),
        }
    }

    fn backend_name(&self) -> &'static str {
        "redis"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Metrics
// ─────────────────────────────────────────────────────────────────────────────

/// Metric label identifying which limit scope was evaluated.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ScopeLabel {
    /// Limit scope, e.g. `ip` or `client`.
    pub scope: String,
}

/// Prometheus metrics for distributed rate limiting.
///
/// Exposed metric names (see `monitoring/rate-limit-alerts.yaml`):
///
/// - `stellar_gateway_rate_limit_checks_total{scope}`
/// - `stellar_gateway_rate_limit_exceeded_total{scope}`
/// - `stellar_gateway_rate_limit_backend_errors_total`
/// - `stellar_gateway_rate_limit_check_duration_seconds`
#[derive(Debug)]
pub struct RateLimitMetrics {
    /// Every limit decision, allowed or not.
    pub checks: Family<ScopeLabel, Counter>,
    /// Decisions that rejected the request.
    pub exceeded: Family<ScopeLabel, Counter>,
    /// Store failures that forced a fail-open fallback.
    pub backend_errors: Counter,
    /// Wall-clock cost of a check, for asserting the sub-millisecond budget.
    pub check_duration_seconds: Histogram,
}

impl Default for RateLimitMetrics {
    fn default() -> Self {
        Self {
            checks: Family::default(),
            exceeded: Family::default(),
            backend_errors: Counter::default(),
            // Buckets straddle the 1ms budget so the SLO is directly readable.
            check_duration_seconds: Histogram::new(
                [0.0001, 0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.05].into_iter(),
            ),
        }
    }
}

impl RateLimitMetrics {
    /// Create metrics with the default buckets.
    pub fn new() -> Self {
        Self::default()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Limiter
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for [`DistributedRateLimiter`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributedRateLimitConfig {
    /// Requests permitted per identifier per window.
    pub max_requests: u32,
    /// Length of the fixed window.
    pub window: Duration,
    /// Key namespace, so several limiters can share one Redis.
    pub key_prefix: String,
    /// Serve the request when the store is unreachable.
    ///
    /// `true` (the default) trades exact enforcement for availability. Set
    /// `false` only where exceeding the limit is worse than dropping traffic.
    pub fail_open: bool,
}

impl Default for DistributedRateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 100,
            window: Duration::from_secs(60),
            key_prefix: "stellar:ratelimit".to_string(),
            fail_open: true,
        }
    }
}

/// Outcome of one rate limit check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitDecision {
    /// Whether the request may proceed.
    pub allowed: bool,
    /// Requests observed in this window, including the current one.
    pub current_count: u64,
    /// Configured ceiling for the window.
    pub limit: u32,
    /// Requests left before the limit is hit.
    pub remaining: u32,
    /// Unix seconds at which the window rolls over.
    pub reset_at_epoch: u64,
    /// True when the shared store failed and the decision used the local
    /// fallback, so it is not fleet-wide.
    pub degraded: bool,
}

impl RateLimitDecision {
    /// `Retry-After` value in seconds, for a 429 response.
    pub fn retry_after_seconds(&self, now_epoch: u64) -> u64 {
        self.reset_at_epoch.saturating_sub(now_epoch)
    }
}

/// Fixed-window rate limiter shared by every gateway instance.
pub struct DistributedRateLimiter {
    config: DistributedRateLimitConfig,
    store: Arc<dyn DistributedCounterStore>,
    fallback: InMemoryCounterStore,
    metrics: Arc<RateLimitMetrics>,
    /// Monotonic count of fail-open fallbacks, mirrored by the metric so it is
    /// assertable in tests without scraping Prometheus.
    degraded_checks: AtomicU64,
}

impl DistributedRateLimiter {
    /// Build a limiter over `store`.
    pub fn new(
        config: DistributedRateLimitConfig,
        store: Arc<dyn DistributedCounterStore>,
    ) -> Self {
        Self {
            config,
            store,
            fallback: InMemoryCounterStore::new(),
            metrics: Arc::new(RateLimitMetrics::new()),
            degraded_checks: AtomicU64::new(0),
        }
    }

    /// Build a limiter that shares an existing metric set.
    pub fn with_metrics(
        config: DistributedRateLimitConfig,
        store: Arc<dyn DistributedCounterStore>,
        metrics: Arc<RateLimitMetrics>,
    ) -> Self {
        Self {
            config,
            store,
            fallback: InMemoryCounterStore::new(),
            metrics,
            degraded_checks: AtomicU64::new(0),
        }
    }

    /// The shared metric set, for registration with a Prometheus registry.
    pub fn metrics(&self) -> Arc<RateLimitMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Number of checks that fell back to the local counter.
    pub fn degraded_check_count(&self) -> u64 {
        self.degraded_checks.load(Ordering::Relaxed)
    }

    /// Current unix time in seconds.
    fn now_epoch() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Start of the window containing `now_epoch`.
    ///
    /// This is the whole synchronisation mechanism: every instance floors the
    /// same clock to the same boundary and therefore addresses the same key.
    fn window_start(&self, now_epoch: u64) -> u64 {
        let window_secs = self.config.window.as_secs().max(1);
        now_epoch - (now_epoch % window_secs)
    }

    /// The counter key for `scope`/`identifier` in the current window.
    pub fn counter_key(&self, scope: &str, identifier: &str, now_epoch: u64) -> String {
        format!(
            "{}:{}:{}:{}",
            self.config.key_prefix,
            scope,
            identifier,
            self.window_start(now_epoch)
        )
    }

    /// Evaluate the limit for `identifier` within `scope`.
    ///
    /// Records `checks`, `exceeded`, `backend_errors`, and the check duration.
    pub async fn check(&self, scope: &str, identifier: &str) -> RateLimitDecision {
        let started = Instant::now();
        let now_epoch = Self::now_epoch();
        let key = self.counter_key(scope, identifier, now_epoch);

        // TTL covers the remainder of the window plus a second of slack, so a
        // counter cannot expire before the window it belongs to has closed.
        let window_secs = self.config.window.as_secs().max(1);
        let reset_at_epoch = self.window_start(now_epoch) + window_secs;
        let ttl = Duration::from_secs(reset_at_epoch.saturating_sub(now_epoch) + 1);

        let (count, degraded) = match self.store.increment(&key, ttl).await {
            Ok(count) => (count, false),
            Err(err) => {
                warn!(
                    backend = self.store.backend_name(),
                    "rate limit store unavailable, falling back to local counter: {err}"
                );
                self.metrics.backend_errors.inc();
                self.degraded_checks.fetch_add(1, Ordering::Relaxed);

                if !self.config.fail_open {
                    // Fail closed: reject rather than under-enforce.
                    let decision = RateLimitDecision {
                        allowed: false,
                        current_count: 0,
                        limit: self.config.max_requests,
                        remaining: 0,
                        reset_at_epoch,
                        degraded: true,
                    };
                    self.record(scope, &decision, started);
                    return decision;
                }
                let count = self.fallback.increment(&key, ttl).await.unwrap_or(1);
                (count, true)
            }
        };

        let limit = u64::from(self.config.max_requests);
        let decision = RateLimitDecision {
            allowed: count <= limit,
            current_count: count,
            limit: self.config.max_requests,
            remaining: limit.saturating_sub(count) as u32,
            reset_at_epoch,
            degraded,
        };
        self.record(scope, &decision, started);
        decision
    }

    /// Read the current count without consuming quota.
    pub async fn peek(&self, scope: &str, identifier: &str) -> u64 {
        let key = self.counter_key(scope, identifier, Self::now_epoch());
        match self.store.get(&key).await {
            Ok(count) => count,
            Err(_) => self.fallback.get(&key).await.unwrap_or(0),
        }
    }

    /// Update the metric family for a completed decision.
    fn record(&self, scope: &str, decision: &RateLimitDecision, started: Instant) {
        let label = ScopeLabel {
            scope: scope.to_string(),
        };
        self.metrics.checks.get_or_create(&label).inc();
        if !decision.allowed {
            self.metrics.exceeded.get_or_create(&label).inc();
            debug!(
                scope,
                count = decision.current_count,
                limit = decision.limit,
                "rate limit exceeded"
            );
        }
        self.metrics
            .check_duration_seconds
            .observe(started.elapsed().as_secs_f64());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    fn config(max: u32, window_secs: u64) -> DistributedRateLimitConfig {
        DistributedRateLimitConfig {
            max_requests: max,
            window: Duration::from_secs(window_secs),
            key_prefix: "test".to_string(),
            fail_open: true,
        }
    }

    fn limiter(max: u32) -> DistributedRateLimiter {
        DistributedRateLimiter::new(config(max, 60), Arc::new(InMemoryCounterStore::new()))
    }

    // ── RESP encoding ────────────────────────────────────────────────────

    #[test]
    fn commands_encode_as_resp_bulk_arrays() {
        assert_eq!(
            encode_command(&["GET", "k"]),
            b"*2\r\n$3\r\nGET\r\n$1\r\nk\r\n".to_vec()
        );
    }

    #[test]
    fn encoding_is_length_prefixed_not_delimiter_based() {
        // A key containing CRLF must survive, which is why lengths are sent.
        let encoded = encode_command(&["GET", "a\r\nb"]);
        assert_eq!(encoded, b"*2\r\n$3\r\nGET\r\n$4\r\na\r\nb\r\n".to_vec());
    }

    #[tokio::test]
    async fn integer_replies_are_parsed() {
        let mut reader = BufReader::new(&b":42\r\n"[..]);
        assert_eq!(read_reply(&mut reader).await.unwrap(), Reply::Integer(42));
    }

    #[tokio::test]
    async fn error_replies_are_parsed() {
        let mut reader = BufReader::new(&b"-ERR nope\r\n"[..]);
        assert_eq!(
            read_reply(&mut reader).await.unwrap(),
            Reply::Error("ERR nope".into())
        );
    }

    #[tokio::test]
    async fn bulk_replies_are_parsed() {
        let mut reader = BufReader::new(&b"$3\r\nabc\r\n"[..]);
        assert_eq!(
            read_reply(&mut reader).await.unwrap(),
            Reply::Bulk(Some("abc".into()))
        );
    }

    #[tokio::test]
    async fn null_bulk_replies_are_parsed() {
        let mut reader = BufReader::new(&b"$-1\r\n"[..]);
        assert_eq!(read_reply(&mut reader).await.unwrap(), Reply::Bulk(None));
    }

    #[tokio::test]
    async fn a_closed_connection_is_reported_as_unavailable() {
        let mut reader = BufReader::new(&b""[..]);
        assert!(matches!(
            read_reply(&mut reader).await,
            Err(StoreError::Unavailable(_))
        ));
    }

    // ── In-memory store ──────────────────────────────────────────────────

    #[tokio::test]
    async fn increment_counts_up_from_one() {
        let store = InMemoryCounterStore::new();
        let ttl = Duration::from_secs(60);
        assert_eq!(store.increment("k", ttl).await.unwrap(), 1);
        assert_eq!(store.increment("k", ttl).await.unwrap(), 2);
        assert_eq!(store.get("k").await.unwrap(), 2);
    }

    #[tokio::test]
    async fn distinct_keys_do_not_share_a_counter() {
        let store = InMemoryCounterStore::new();
        let ttl = Duration::from_secs(60);
        store.increment("a", ttl).await.unwrap();
        assert_eq!(store.increment("b", ttl).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn absent_keys_read_as_zero() {
        assert_eq!(InMemoryCounterStore::new().get("nope").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn expired_entries_are_purged() {
        let store = InMemoryCounterStore::new();
        store
            .increment("k", Duration::from_millis(10))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(store.get("k").await.unwrap(), 0);
        // A later increment starts a fresh window rather than resuming.
        assert_eq!(
            store.increment("k", Duration::from_secs(60)).await.unwrap(),
            1
        );
    }

    // ── Key derivation (the synchronisation mechanism) ────────────────────

    #[test]
    fn instances_derive_the_same_key_within_a_window() {
        let store = Arc::new(InMemoryCounterStore::new());
        let a = DistributedRateLimiter::new(config(10, 60), store.clone());
        let b = DistributedRateLimiter::new(config(10, 60), store);
        // Two replicas, same window, same identity → one shared counter.
        assert_eq!(
            a.counter_key("ip", "10.0.0.1", 1_700_000_045),
            b.counter_key("ip", "10.0.0.1", 1_700_000_099)
        );
    }

    #[test]
    fn keys_differ_once_the_window_rolls_over() {
        let l = limiter(10);
        assert_ne!(
            l.counter_key("ip", "10.0.0.1", 1_700_000_099),
            l.counter_key("ip", "10.0.0.1", 1_700_000_100)
        );
    }

    #[test]
    fn window_start_floors_to_the_boundary() {
        let l = limiter(10);
        assert_eq!(l.window_start(1_700_000_099), 1_700_000_040);
        // An exact boundary is its own window start.
        assert_eq!(l.window_start(1_700_000_100), 1_700_000_100);
    }

    #[test]
    fn scope_and_identifier_namespace_the_key() {
        let l = limiter(10);
        let now = 1_700_000_000;
        assert_ne!(
            l.counter_key("ip", "1.1.1.1", now),
            l.counter_key("client", "1.1.1.1", now)
        );
        assert_ne!(
            l.counter_key("ip", "1.1.1.1", now),
            l.counter_key("ip", "2.2.2.2", now)
        );
    }

    #[test]
    fn key_prefix_isolates_limiters_sharing_one_store() {
        let mut other = config(10, 60);
        other.key_prefix = "other".into();
        let a = limiter(10);
        let b = DistributedRateLimiter::new(other, Arc::new(InMemoryCounterStore::new()));
        assert_ne!(
            a.counter_key("ip", "x", 1_700_000_000),
            b.counter_key("ip", "x", 1_700_000_000)
        );
    }

    // ── Enforcement ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn requests_within_the_limit_are_allowed() {
        let l = limiter(3);
        for expected_remaining in [2, 1, 0] {
            let d = l.check("ip", "1.1.1.1").await;
            assert!(d.allowed);
            assert_eq!(d.remaining, expected_remaining);
        }
    }

    #[tokio::test]
    async fn the_request_past_the_limit_is_rejected() {
        let l = limiter(2);
        l.check("ip", "1.1.1.1").await;
        l.check("ip", "1.1.1.1").await;
        let d = l.check("ip", "1.1.1.1").await;
        assert!(!d.allowed);
        assert_eq!(d.current_count, 3);
        assert_eq!(d.remaining, 0);
    }

    #[tokio::test]
    async fn a_shared_store_enforces_one_limit_across_instances() {
        // The whole point of the issue: two limiters, one budget of 4.
        let store = Arc::new(InMemoryCounterStore::new());
        let a = DistributedRateLimiter::new(config(4, 60), store.clone());
        let b = DistributedRateLimiter::new(config(4, 60), store);

        assert!(a.check("ip", "1.1.1.1").await.allowed);
        assert!(b.check("ip", "1.1.1.1").await.allowed);
        assert!(a.check("ip", "1.1.1.1").await.allowed);
        assert!(b.check("ip", "1.1.1.1").await.allowed);
        // Fifth request anywhere in the fleet is over budget.
        assert!(!a.check("ip", "1.1.1.1").await.allowed);
        assert!(!b.check("ip", "1.1.1.1").await.allowed);
    }

    #[tokio::test]
    async fn per_process_limiters_would_let_through_n_times_the_limit() {
        // Contrast case documenting the bug this module fixes.
        let a = DistributedRateLimiter::new(config(2, 60), Arc::new(InMemoryCounterStore::new()));
        let b = DistributedRateLimiter::new(config(2, 60), Arc::new(InMemoryCounterStore::new()));
        assert!(a.check("ip", "1.1.1.1").await.allowed);
        assert!(a.check("ip", "1.1.1.1").await.allowed);
        assert!(b.check("ip", "1.1.1.1").await.allowed); // separate store → allowed
    }

    #[tokio::test]
    async fn identifiers_are_limited_independently() {
        let l = limiter(1);
        assert!(l.check("ip", "1.1.1.1").await.allowed);
        assert!(l.check("ip", "2.2.2.2").await.allowed);
    }

    #[tokio::test]
    async fn peek_does_not_consume_quota() {
        let l = limiter(2);
        l.check("ip", "1.1.1.1").await;
        assert_eq!(l.peek("ip", "1.1.1.1").await, 1);
        assert_eq!(l.peek("ip", "1.1.1.1").await, 1);
        assert!(l.check("ip", "1.1.1.1").await.allowed);
    }

    #[tokio::test]
    async fn reset_time_lands_on_the_next_window_boundary() {
        let l = limiter(5);
        let d = l.check("ip", "1.1.1.1").await;
        let now = DistributedRateLimiter::now_epoch();
        assert!(d.reset_at_epoch > now);
        assert!(d.reset_at_epoch - l.window_start(now) == 60);
        assert!(d.retry_after_seconds(now) <= 60);
    }

    // ── Failure behaviour ────────────────────────────────────────────────

    struct AlwaysFailingStore;

    #[async_trait]
    impl DistributedCounterStore for AlwaysFailingStore {
        async fn increment(&self, _: &str, _: Duration) -> Result<u64, StoreError> {
            Err(StoreError::Unavailable("down".into()))
        }
        async fn get(&self, _: &str) -> Result<u64, StoreError> {
            Err(StoreError::Unavailable("down".into()))
        }
        fn backend_name(&self) -> &'static str {
            "failing"
        }
    }

    #[tokio::test]
    async fn an_unreachable_store_fails_open_to_a_local_counter() {
        let l = DistributedRateLimiter::new(config(2, 60), Arc::new(AlwaysFailingStore));
        let d = l.check("ip", "1.1.1.1").await;
        assert!(d.allowed);
        assert!(d.degraded, "decision must be flagged as fleet-inaccurate");
        assert_eq!(l.degraded_check_count(), 1);
    }

    #[tokio::test]
    async fn the_local_fallback_still_enforces_the_limit() {
        let l = DistributedRateLimiter::new(config(2, 60), Arc::new(AlwaysFailingStore));
        assert!(l.check("ip", "1.1.1.1").await.allowed);
        assert!(l.check("ip", "1.1.1.1").await.allowed);
        assert!(!l.check("ip", "1.1.1.1").await.allowed);
    }

    #[tokio::test]
    async fn fail_closed_rejects_when_the_store_is_down() {
        let mut cfg = config(100, 60);
        cfg.fail_open = false;
        let l = DistributedRateLimiter::new(cfg, Arc::new(AlwaysFailingStore));
        let d = l.check("ip", "1.1.1.1").await;
        assert!(!d.allowed);
        assert!(d.degraded);
    }

    #[tokio::test]
    async fn a_healthy_store_never_marks_a_decision_degraded() {
        assert!(!limiter(5).check("ip", "1.1.1.1").await.degraded);
    }

    // ── Redis store against a stub server ────────────────────────────────

    /// Minimal RESP server that replies to each command from `replies`.
    async fn stub_redis(replies: Vec<&'static str>) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            for reply in replies {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
            seen
        });
        (addr, handle)
    }

    fn redis_store(address: String) -> RedisCounterStore {
        RedisCounterStore::new(RedisStoreConfig {
            address,
            pool_size: 4,
            timeout: Duration::from_secs(2),
        })
    }

    #[tokio::test]
    async fn redis_increment_sends_one_eval_and_returns_the_count() {
        let (addr, handle) = stub_redis(vec![":7\r\n"]).await;
        let store = redis_store(addr);

        let count = store.increment("k", Duration::from_secs(60)).await.unwrap();
        assert_eq!(count, 7);

        let sent = String::from_utf8(handle.await.unwrap()).unwrap();
        assert!(sent.starts_with("*5\r\n"), "one EVAL command: {sent}");
        assert!(sent.contains("EVAL"));
        assert!(sent.contains("INCR"));
        assert!(sent.contains("PEXPIRE"), "TTL must be set atomically");
        assert!(sent.contains("60000"), "TTL is sent in milliseconds");
        assert_eq!(sent.matches("EVAL").count(), 1, "exactly one round trip");
    }

    #[tokio::test]
    async fn redis_get_parses_a_bulk_counter() {
        let (addr, _handle) = stub_redis(vec!["$2\r\n12\r\n"]).await;
        assert_eq!(redis_store(addr).get("k").await.unwrap(), 12);
    }

    #[tokio::test]
    async fn redis_get_treats_a_missing_key_as_zero() {
        let (addr, _handle) = stub_redis(vec!["$-1\r\n"]).await;
        assert_eq!(redis_store(addr).get("k").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_redis_error_reply_surfaces_as_a_protocol_error() {
        let (addr, _handle) = stub_redis(vec!["-NOSCRIPT bad\r\n"]).await;
        let err = redis_store(addr)
            .increment("k", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::Protocol(_)));
    }

    #[tokio::test]
    async fn an_unreachable_redis_surfaces_as_unavailable() {
        // Port 1 on loopback refuses connections.
        let store = RedisCounterStore::new(RedisStoreConfig {
            address: "127.0.0.1:1".into(),
            pool_size: 1,
            timeout: Duration::from_millis(200),
        });
        assert!(matches!(
            store.increment("k", Duration::from_secs(1)).await,
            Err(StoreError::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn connections_are_reused_across_commands() {
        let (addr, handle) = stub_redis(vec![":1\r\n", ":2\r\n"]).await;
        let store = redis_store(addr);
        assert_eq!(
            store.increment("k", Duration::from_secs(60)).await.unwrap(),
            1
        );
        assert_eq!(
            store.increment("k", Duration::from_secs(60)).await.unwrap(),
            2
        );
        // The stub only ever accepts one socket, so a second connection
        // attempt would have hung rather than returning 2.
        let sent = String::from_utf8(handle.await.unwrap()).unwrap();
        assert_eq!(sent.matches("EVAL").count(), 2);
    }

    #[tokio::test]
    async fn a_redis_backed_limiter_enforces_the_limit() {
        // Counts 1, 2, 3 against a limit of 2.
        let (addr, _handle) = stub_redis(vec![":1\r\n", ":2\r\n", ":3\r\n"]).await;
        let l = DistributedRateLimiter::new(config(2, 60), Arc::new(redis_store(addr)));
        assert!(l.check("ip", "1.1.1.1").await.allowed);
        assert!(l.check("ip", "1.1.1.1").await.allowed);
        assert!(!l.check("ip", "1.1.1.1").await.allowed);
        assert_eq!(l.degraded_check_count(), 0);
    }

    #[tokio::test]
    async fn the_backend_name_labels_the_store() {
        assert_eq!(InMemoryCounterStore::new().backend_name(), "memory");
        assert_eq!(redis_store("127.0.0.1:1".into()).backend_name(), "redis");
    }

    // ── Overhead budget ──────────────────────────────────────────────────

    #[tokio::test]
    async fn local_check_overhead_stays_under_the_one_millisecond_budget() {
        // Bounds the limiter's own cost (key derivation, metrics, bookkeeping)
        // independently of network latency to the store.
        let l = limiter(1_000_000);
        // Warm up so allocator/lazy-init cost is not attributed to the budget.
        for _ in 0..100 {
            l.check("ip", "1.1.1.1").await;
        }
        let started = Instant::now();
        let iterations = 1_000;
        for _ in 0..iterations {
            l.check("ip", "1.1.1.1").await;
        }
        let per_check = started.elapsed() / iterations;
        assert!(
            per_check < Duration::from_millis(1),
            "per-check overhead {per_check:?} exceeds the 1ms budget"
        );
    }
}
