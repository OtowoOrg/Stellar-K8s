//! Dynamic rate-limiter engine for Soroban RPC gateway.
//!
//! The engine combines per-IP sliding-window tracking (from [`super::window`]) with
//! real-time CPU utilization feedback to produce a **dynamic** per-client request
//! limit that shrinks under load and relaxes under idle conditions.
//!
//! # Architecture
//!
//! ```text
//!  ┌───────────────────────────────────────────────────────────────┐
//!  │                     RateLimitEngine                           │
//!  │                                                               │
//!  │  ┌────────────────────┐   ┌──────────────────────────────┐   │
//!  │  │  SlidingWindowTracker│   │  CpuMonitor (background task)│   │
//!  │  │  (per-IP state)    │   │  reads /proc/stat every 500ms│   │
//!  │  └────────────────────┘   └──────────────────────────────┘   │
//!  │                                                               │
//!  │  check(ip) → effective_limit → window.try_record()           │
//!  │           → RateLimitDecision { allowed, retry_after, … }    │
//!  └───────────────────────────────────────────────────────────────┘
//! ```
//!
//! # CPU Scaling Policy
//!
//! | CPU utilization | Effective limit              |
//! |-----------------|------------------------------|
//! | < `low_pct`     | `base_rps` (full limit)      |
//! | `low_pct`–`high_pct` | linear interpolation  |
//! | ≥ `high_pct`    | `min_rps` (floor)            |
//!
//! # HTTP 429 Response Envelope
//!
//! When a request is rejected the engine returns a [`RateLimitDecision`] with
//! `allowed = false`.  Callers should respond with:
//!
//! ```http
//! HTTP/1.1 429 Too Many Requests
//! Retry-After: <retry_after_secs>
//! X-RateLimit-Limit: <effective_limit>
//! X-RateLimit-Remaining: 0
//! X-RateLimit-Reset: <unix_reset_epoch>
//! Content-Type: application/json
//!
//! {"error":"rate_limit_exceeded","retry_after":<retry_after_secs>}
//! ```

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

use super::window::SlidingWindowTracker;

// ──────────────────────────────────────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────────────────────────────────────

/// Result returned by [`RateLimitEngine::check`].
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitDecision {
    /// Whether the request is **allowed** to proceed.
    pub allowed: bool,
    /// Effective per-IP limit used for this decision (requests per window).
    pub effective_limit: usize,
    /// Remaining requests before the client is throttled (0 when rejected).
    pub remaining: usize,
    /// Seconds until the client may retry (0 when `allowed == true`).
    pub retry_after_secs: u64,
    /// Unix timestamp at which the window resets (for `X-RateLimit-Reset`).
    pub reset_at_epoch: u64,
    /// Current CPU utilisation snapshot (0–100 %).
    pub cpu_pct: u8,
}

impl RateLimitDecision {
    /// HTTP status code to use (200 when allowed, 429 when throttled).
    pub fn http_status(&self) -> u16 {
        if self.allowed { 200 } else { 429 }
    }

    /// Build a minimal JSON error body for 429 responses.
    pub fn error_body(&self) -> String {
        format!(
            r#"{{"error":"rate_limit_exceeded","retry_after":{}}}"#,
            self.retry_after_secs
        )
    }
}

/// Per-consumer fair-share limit entry pushed by the fair-share controller.
///
/// The controller computes each consumer's fair share of shared capacity and
/// publishes it through the config channel.  The engine keeps this in proxy
/// memory so the hot path never blocks on the controller.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsumerLimit {
    /// Effective requests-per-window limit for this consumer.
    pub limit: usize,
    /// Monotonic generation counter; higher wins on conflicting updates.
    pub generation: u64,
}

/// Configuration for the rate-limit engine.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum requests per window per IP under no load.
    pub base_rps: usize,
    /// Minimum requests per window per IP under maximum load.
    pub min_rps: usize,
    /// CPU % below which the full `base_rps` applies.
    pub cpu_low_pct: u8,
    /// CPU % at or above which the `min_rps` floor applies.
    pub cpu_high_pct: u8,
    /// Duration of each sliding window.
    pub window: Duration,
    /// How often the CPU monitor refreshes its reading.
    pub cpu_poll_interval: Duration,
    /// Idle TTL: evict client windows not seen within this duration.
    pub idle_evict_ttl: Duration,
    /// When `true`, per-consumer fair-share limits override the CPU-scaled
    /// global limit for consumers present in the fair-share table.
    pub fair_share_enabled: bool,
    /// Floor applied to any per-consumer fair-share limit so a consumer is
    /// never fully starved (protects well-behaved consumers).
    pub fair_share_min_rps: usize,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            base_rps: 100,
            min_rps: 10,
            cpu_low_pct: 50,
            cpu_high_pct: 85,
            window: Duration::from_secs(1),
            cpu_poll_interval: Duration::from_millis(500),
            idle_evict_ttl: Duration::from_secs(300),
            fair_share_enabled: true,
            fair_share_min_rps: 5,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// CPU monitoring
// ──────────────────────────────────────────────────────────────────────────────

/// Reads `/proc/stat` and returns the system-wide CPU utilisation percentage
/// over the interval since the previous call.
///
/// Returns `None` if `/proc/stat` is unavailable (non-Linux environments).
///
/// The two successive readings needed for delta calculation are held in
/// `prev_idle` / `prev_total` which must be maintained by the caller.
fn read_cpu_percent(prev_idle: &mut u64, prev_total: &mut u64) -> Option<u8> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let line = stat.lines().next()?; // "cpu  …"
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();

    // Fields: user, nice, system, idle, iowait, irq, softirq, …
    if fields.len() < 4 {
        return None;
    }
    let idle = fields[3] + fields.get(4).copied().unwrap_or(0); // idle + iowait
    let total: u64 = fields.iter().sum();

    let delta_total = total.saturating_sub(*prev_total);
    let delta_idle = idle.saturating_sub(*prev_idle);

    *prev_total = total;
    *prev_idle = idle;

    if delta_total == 0 {
        return Some(0);
    }

    let busy = delta_total - delta_idle;
    Some(((busy * 100) / delta_total).min(100) as u8)
}

// ──────────────────────────────────────────────────────────────────────────────
// Engine
// ──────────────────────────────────────────────────────────────────────────────

/// Dynamic rate-limiter engine.
///
/// Create one instance per application and share it (it is cheaply cloneable
/// because the interior is `Arc`-wrapped).
///
/// # Example
///
/// ```no_run
/// use std::net::IpAddr;
/// use stellar_k8s::gateway::ratelimit::engine::{RateLimitEngine, RateLimitConfig};
///
/// #[tokio::main]
/// async fn main() {
///     let engine = RateLimitEngine::new(RateLimitConfig::default());
///     engine.start_background_tasks();
///
///     let ip: IpAddr = "203.0.113.1".parse().unwrap();
///     let decision = engine.check(ip);
///     if !decision.allowed {
///         eprintln!("throttled – retry after {}s", decision.retry_after_secs);
///     }
/// }
/// ```
#[derive(Clone)]
pub struct RateLimitEngine {
    config: Arc<RwLock<RateLimitConfig>>,
    tracker: SlidingWindowTracker,
    /// Atomically shared CPU reading (0–100), updated by the background task.
    cpu_pct: Arc<AtomicU64>,
    /// Total requests evaluated.
    total_requests: Arc<AtomicU64>,
    /// Total requests rejected (throttled).
    rejected_requests: Arc<AtomicU64>,
    /// Per-consumer fair-share limits, kept in proxy memory for the hot path.
    fair_share: Arc<RwLock<HashMap<IpAddr, ConsumerLimit>>>,
    /// Monotonic generation counter for fair-share config updates.
    fair_share_generation: Arc<AtomicU64>,
    /// Timestamp (epoch millis) of the last fair-share config application.
    last_config_apply_ms: Arc<AtomicU64>,
    /// Observed containment latency (ms) from saturation onset to first
    /// throttled request of a noisy consumer.
    containment_latency_ms: Arc<AtomicU64>,
    /// Requests rejected for consumers that were *not* over their fair share
    /// (i.e. induced 429s).  Must stay at zero for well-behaved consumers.
    induced_429s: Arc<AtomicU64>,
}

impl std::fmt::Debug for RateLimitEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimitEngine")
            .field("cpu_pct", &self.cpu_pct.load(Ordering::Relaxed))
            .field("total_requests", &self.total_requests.load(Ordering::Relaxed))
            .field("tracked_ips", &self.tracker.tracked_ips())
            .finish()
    }
}

impl RateLimitEngine {
    /// Create a new engine with the supplied configuration.
    pub fn new(config: RateLimitConfig) -> Self {
        let tracker = SlidingWindowTracker::new(config.base_rps, config.window);
        Self {
            config: Arc::new(RwLock::new(config)),
            tracker,
            cpu_pct: Arc::new(AtomicU64::new(0)),
            total_requests: Arc::new(AtomicU64::new(0)),
            rejected_requests: Arc::new(AtomicU64::new(0)),
            fair_share: Arc::new(RwLock::new(HashMap::new())),
            fair_share_generation: Arc::new(AtomicU64::new(0)),
            last_config_apply_ms: Arc::new(AtomicU64::new(0)),
            containment_latency_ms: Arc::new(AtomicU64::new(0)),
            induced_429s: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Spawn background tasks:
    /// 1. CPU monitor — refreshes `cpu_pct` every `cpu_poll_interval`.
    /// 2. Idle eviction — cleans stale client windows every 60 s.
    pub fn start_background_tasks(&self) {
        let cpu_pct = Arc::clone(&self.cpu_pct);
        let config = Arc::clone(&self.config);

        tokio::spawn(async move {
            let mut prev_idle: u64 = 0;
            let mut prev_total: u64 = 0;
            loop {
                let interval = config.read().await.cpu_poll_interval;
                tokio::time::sleep(interval).await;
                if let Some(pct) = read_cpu_percent(&mut prev_idle, &mut prev_total) {
                    let old = cpu_pct.swap(pct as u64, Ordering::Relaxed);
                    if old != pct as u64 {
                        debug!(cpu_pct = pct, "CPU utilisation updated");
                    }
                }
            }
        });

        let tracker = self.tracker.clone();
        let config = Arc::clone(&self.config);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let ttl = config.read().await.idle_evict_ttl;
                let before = tracker.tracked_ips();
                tracker.evict_idle(Instant::now(), ttl);
                let after = tracker.tracked_ips();
                if before != after {
                    info!(
                        evicted = before - after,
                        remaining = after,
                        "Evicted idle rate-limit windows"
                    );
                }
            }
        });
    }

    /// Compute the effective rate limit given the current CPU utilisation.
    ///
    /// Performs linear interpolation between `base_rps` and `min_rps` in the
    /// `[cpu_low_pct, cpu_high_pct]` range.
    pub fn effective_limit(&self, cpu: u8, config: &RateLimitConfig) -> usize {
        if cpu < config.cpu_low_pct {
            return config.base_rps;
        }
        if cpu >= config.cpu_high_pct {
            return config.min_rps;
        }

        // Linear interpolation: at cpu_low_pct → base_rps; at cpu_high_pct → min_rps
        let span = (config.cpu_high_pct - config.cpu_low_pct) as f64;
        let pos = (cpu - config.cpu_low_pct) as f64;
        let ratio = pos / span; // 0.0 (low) … 1.0 (high)
        let range = config.base_rps.saturating_sub(config.min_rps) as f64;
        let limit = config.base_rps as f64 - (ratio * range);
        (limit.round() as usize).max(config.min_rps)
    }

    /// Compute the effective limit for a specific consumer.
    ///
    /// When fair-share is enabled and the consumer has a published limit, that
    /// limit (clamped to at least `fair_share_min_rps`) takes precedence over
    /// the CPU-scaled global limit.  Otherwise the CPU-scaled limit applies.
    fn effective_limit_for(
        &self,
        ip: IpAddr,
        cpu: u8,
        config: &RateLimitConfig,
        fair_share: Option<&HashMap<IpAddr, ConsumerLimit>>,
    ) -> usize {
        let global = self.effective_limit(cpu, config);
        if !config.fair_share_enabled {
            return global;
        }
        if let Some(entry) = fair_share.and_then(|m| m.get(&ip)) {
            return entry.limit.max(config.fair_share_min_rps);
        }
        global
    }

    /// Evaluate a request from `ip` and return a [`RateLimitDecision`].
    ///
    /// This is the hot path — it must complete in sub-millisecond time.
    /// All I/O is avoided; CPU reading and fair-share config are pre-cached in
    /// proxy memory by the background tasks / config channel.
    #[instrument(skip(self), fields(ip = %ip))]
    pub fn check(&self, ip: IpAddr) -> RateLimitDecision {
        let now = Instant::now();
        let cpu = self.cpu_pct.load(Ordering::Relaxed) as u8;

        // Config is read via a non-async try_read (fallback to defaults on contention)
        let config_guard = self.config.try_read();
        let fair_share_guard = self.fair_share.try_read();

        let effective_limit = match &config_guard {
            Ok(cfg) => self.effective_limit_for(ip, cpu, cfg, fair_share_guard.as_deref().ok()),
            Err(_) => {
                // Config locked for write (rare) — use current base as safe fallback
                warn!("Rate-limit config locked; using cpu=0 fallback");
                100 // safe default
            }
        };

        let (allowed, retry_after_secs) = self.tracker.try_record(ip, effective_limit, now);

        self.total_requests.fetch_add(1, Ordering::Relaxed);
        if !allowed {
            self.rejected_requests.fetch_add(1, Ordering::Relaxed);

            // Classify the rejection: if the consumer is within its published
            // fair share, this is an *induced* 429 (a regression we track).
            let within_fair_share = match (&config_guard, &fair_share_guard) {
                (Ok(cfg), Ok(fs)) if cfg.fair_share_enabled => fs
                    .get(&ip)
                    .map(|e| e.limit >= cfg.fair_share_min_rps)
                    .unwrap_or(false),
                _ => false,
            };
            if within_fair_share {
                self.induced_429s.fetch_add(1, Ordering::Relaxed);
            } else {
                // First rejection of a noisy consumer marks containment.
                let onset = self.last_config_apply_ms.load(Ordering::Relaxed);
                if onset > 0 {
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let latency = now_ms.saturating_sub(onset);
                    self.containment_latency_ms.store(latency, Ordering::Relaxed);
                }
            }
        }

        let remaining = if allowed {
            effective_limit.saturating_sub(self.tracker.count(ip, now))
        } else {
            0
        };

        // Unix epoch reset time (approximate: now + window duration)
        let window = config_guard
            .as_ref()
            .map(|c| c.window)
            .unwrap_or(Duration::from_secs(1));
        let reset_at_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + window.as_secs();

        RateLimitDecision {
            allowed,
            effective_limit,
            remaining,
            retry_after_secs,
            reset_at_epoch,
            cpu_pct: cpu,
        }
    }

    /// Dynamically update the engine configuration at runtime (zero-restart).
    pub async fn update_config(&self, new_config: RateLimitConfig) {
        let mut cfg = self.config.write().await;
        *cfg = new_config;
    }

    /// Apply a batch of per-consumer fair-share limits published by the
    /// fair-share controller.
    ///
    /// This is the config-channel entry point: the controller pushes limits and
    /// the engine swaps them into proxy memory atomically.  Updates are
    /// generation-guarded so stale batches are ignored, and the whole swap
    /// completes in well under the 1 s propagation target.
    pub async fn apply_fair_share_limits(&self, limits: HashMap<IpAddr, ConsumerLimit>) {
        let mut fs = self.fair_share.write().await;
        for (ip, entry) in limits {
            let current_gen = fs.get(&ip).map(|e| e.generation).unwrap_or(0);
            if entry.generation >= current_gen {
                fs.insert(ip, entry);
            }
        }
        let gen = self.fair_share_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.last_config_apply_ms.store(now_ms, Ordering::Relaxed);
        debug!(
            generation = gen,
            consumers = fs.len(),
            "Applied fair-share limits"
        );
    }

    /// Remove a consumer's fair-share limit (e.g. on disconnect).
    pub async fn remove_fair_share_limit(&self, ip: IpAddr) {
        let mut fs = self.fair_share.write().await;
        fs.remove(&ip);
    }

    /// Number of consumers currently tracked by the fair-share table.
    pub async fn fair_share_consumers(&self) -> usize {
        self.fair_share.read().await.len()
    }

    /// Compute the Jain fairness index over the current per-consumer limits.
    ///
    /// Returns a value in `[0.0, 1.0]`; `1.0` means perfectly fair.  Returns
    /// `1.0` when fewer than two consumers are tracked (trivially fair).
    pub async fn jain_index(&self) -> f64 {
        let fs = self.fair_share.read().await;
        let n = fs.len();
        if n < 2 {
            return 1.0;
        }
        let mut sum = 0.0f64;
        let mut sum_sq = 0.0f64;
        for entry in fs.values() {
            let x = entry.limit as f64;
            sum += x;
            sum_sq += x * x;
        }
        if sum_sq == 0.0 {
            return 1.0;
        }
        (sum * sum) / (n as f64 * sum_sq)
    }

    /// Metrics snapshot for Prometheus / logging.
    pub fn metrics(&self) -> EngineMetrics {
        EngineMetrics {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            rejected_requests: self.rejected_requests.load(Ordering::Relaxed),
            tracked_ips: self.tracker.tracked_ips() as u64,
            cpu_pct: self.cpu_pct.load(Ordering::Relaxed) as u8,
            fair_share_generation: self.fair_share_generation.load(Ordering::Relaxed),
            containment_latency_ms: self.containment_latency_ms.load(Ordering::Relaxed),
            induced_429s: self.induced_429s.load(Ordering::Relaxed),
        }
    }
}

/// Snapshot of engine counters.
#[derive(Debug, Clone)]
pub struct EngineMetrics {
    pub total_requests: u64,
    pub rejected_requests: u64,
    pub tracked_ips: u64,
    pub cpu_pct: u8,
    /// Generation of the last applied fair-share config batch.
    pub fair_share_generation: u64,
    /// Observed containment latency (ms) from saturation onset to first
    /// throttled request of a noisy consumer.
    pub containment_latency_ms: u64,
    /// Requests rejected for consumers within their fair share (must be 0 for
    /// well-behaved consumers).
    pub induced_429s: u64,
}

// ──────────────────────────────────────────────────────────────────────────────
// Axum middleware extractor helper
// ──────────────────────────────────────────────────────────────────────────────

/// Extract the real client IP from `X-Forwarded-For` or fall back to the
/// connection's remote address string.
///
/// Returns `127.0.0.1` when parsing fails.
pub fn extract_client_ip(
    forwarded_for: Option<&str>,
    remote_addr: Option<&str>,
) -> IpAddr {
    if let Some(xff) = forwarded_for {
        // X-Forwarded-For: client, proxy1, proxy2 — take the leftmost
        if let Some(first) = xff.split(',').next() {
            if let Ok(ip) = first.trim().parse() {
                return ip;
            }
        }
    }

    if let Some(addr) = remote_addr {
        // May be "1.2.3.4:port" or just "1.2.3.4"
        let ip_str = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
        if let Ok(ip) = ip_str.trim_matches('[').trim_matches(']').parse() {
            return ip;
        }
    }

    IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, a))
    }

    fn default_engine() -> RateLimitEngine {
        RateLimitEngine::new(RateLimitConfig::default())
    }

    #[test]
    fn effective_limit_below_low_pct_returns_base() {
        let engine = default_engine();
        let cfg = RateLimitConfig::default();
        assert_eq!(engine.effective_limit(0, &cfg), cfg.base_rps);
        assert_eq!(engine.effective_limit(49, &cfg), cfg.base_rps);
    }

    #[test]
    fn effective_limit_above_high_pct_returns_min() {
        let engine = default_engine();
        let cfg = RateLimitConfig::default();
        assert_eq!(engine.effective_limit(85, &cfg), cfg.min_rps);
        assert_eq!(engine.effective_limit(100, &cfg), cfg.min_rps);
    }

    #[test]
    fn effective_limit_interpolates_correctly() {
        let engine = default_engine();
        let cfg = RateLimitConfig {
            base_rps: 100,
            min_rps: 10,
            cpu_low_pct: 50,
            cpu_high_pct: 100,
            ..Default::default()
        };
        // At midpoint (cpu=75) → 50 % of the way → limit = 100 - 0.5 * 90 = 55
        let limit = engine.effective_limit(75, &cfg);
        assert!((50..=60).contains(&limit), "interpolated limit = {}", limit);
    }

    #[test]
    fn check_allows_first_request() {
        let engine = default_engine();
        let d = engine.check(ip(1));
        assert!(d.allowed);
        assert_eq!(d.retry_after_secs, 0);
    }

    #[test]
    fn check_throttles_after_limit() {
        let config = RateLimitConfig {
            base_rps: 3,
            min_rps: 1,
            cpu_low_pct: 50,
            cpu_high_pct: 90,
            window: Duration::from_secs(1),
            ..Default::default()
        };
        let engine = RateLimitEngine::new(config);
        let client = ip(2);

        for i in 0..3 {
            let d = engine.check(client);
            assert!(d.allowed, "request {} should be allowed", i + 1);
        }
        let d = engine.check(client);
        assert!(!d.allowed, "4th request should be throttled");
        assert_eq!(d.http_status(), 429);
        assert!(!d.error_body().is_empty());
    }

    #[test]
    fn check_different_ips_independent() {
        let config = RateLimitConfig {
            base_rps: 2,
            min_rps: 1,
            ..Default::default()
        };
        let engine = RateLimitEngine::new(config);

        // Fill ip(1) window
        engine.check(ip(1));
        engine.check(ip(1));
        let d1 = engine.check(ip(1));
        assert!(!d1.allowed, "ip(1) should be throttled");

        // ip(2) should still be allowed
        let d2 = engine.check(ip(2));
        assert!(d2.allowed, "ip(2) should be independent");
    }

    #[test]
    fn metrics_track_totals() {
        let config = RateLimitConfig { base_rps: 2, min_rps: 1, ..Default::default() };
        let engine = RateLimitEngine::new(config);
        let client = ip(3);

        engine.check(client); // allowed
        engine.check(client); // allowed
        engine.check(client); // rejected

        let m = engine.metrics();
        assert_eq!(m.total_requests, 3);
        assert_eq!(m.rejected_requests, 1);
    }

    #[tokio::test]
    async fn fair_share_limit_overrides_global() {
        let config = RateLimitConfig {
            base_rps: 100,
            min_rps: 100,
            cpu_low_pct: 0,
            cpu_high_pct: 100,
            fair_share_enabled: true,
            fair_share_min_rps: 1,
            ..Default::default()
        };
        let engine = RateLimitEngine::new(config);
        let noisy = ip(10);
        let mut limits = HashMap::new();
        limits.insert(
            noisy,
            ConsumerLimit { limit: 2, generation: 1 },
        );
        engine.apply_fair_share_limits(limits).await;

        assert!(engine.check(noisy).allowed);
        assert!(engine.check(noisy).allowed);
        let d = engine.check(noisy);
        assert!(!d.allowed, "noisy consumer must be contained");
        assert_eq!(d.effective_limit, 2);
    }

    #[tokio::test]
    async fn well_behaved_consumers_see_zero_induced_429s() {
        let config = RateLimitConfig {
            base_rps: 100,
            min_rps: 100,
            cpu_low_pct: 0,
            cpu_high_pct: 100,
            fair_share_enabled: true,
            fair_share_min_rps: 5,
            ..Default::default()
        };
        let engine = RateLimitEngine::new(config);

        // One abusive consumer plus 20 normal consumers.
        let mut limits = HashMap::new();
        limits.insert(ip(1), ConsumerLimit { limit: 5, generation: 1 });
        for a in 2u8..22 {
            limits.insert(ip(a), ConsumerLimit { limit: 50, generation: 1 });
        }
        engine.apply_fair_share_limits(limits).await;

        // Abusive consumer floods.
        for _ in 0..100 {
            engine.check(ip(1));
        }

        // Normal consumers stay well under their fair share.
        for a in 2u8..22 {
            for _ in 0..10 {
                let d = engine.check(ip(a));
                assert!(d.allowed, "normal consumer {} must not be throttled", a);
            }
        }

        let m = engine.metrics();
        assert_eq!(m.induced_429s, 0, "well-behaved consumers must see zero induced 429s");
    }

    #[tokio::test]
    async fn stale_generation_is_ignored() {
        let engine = default_engine();
        let client = ip(30);
        let mut limits = HashMap::new();
        limits.insert(client, ConsumerLimit { limit: 20, generation: 5 });
        engine.apply_fair_share_limits(limits).await;

        let mut stale = HashMap::new();
        stale.insert(client, ConsumerLimit { limit: 1, generation: 3 });
        engine.apply_fair_share_limits(stale).await;

        let fs = engine.fair_share.read().await;
        assert_eq!(fs.get(&client).unwrap().limit, 20);
    }

    #[tokio::test]
    async fn jain_index_is_one_for_equal_limits() {
        let engine = default_engine();
        let mut limits = HashMap::new();
        for a in 1u8..=10 {
            limits.insert(ip(a), ConsumerLimit { limit: 10, generation: 1 });
        }
        engine.apply_fair_share_limits(limits).await;
        let j = engine.jain_index().await;
        assert!((j - 1.0).abs() < 1e-9, "jain index = {}", j);
    }

    #[tokio::test]
    async fn jain_index_degrades_under_imbalance() {
        let engine = default_engine();
        let mut limits = HashMap::new();
        limits.insert(ip(1), ConsumerLimit { limit: 100, generation: 1 });
        for a in 2u8..=10 {
            limits.insert(ip(a), ConsumerLimit { limit: 1, generation: 1 });
        }
        engine.apply_fair_share_limits(limits).await;
        let j = engine.jain_index().await;
        assert!(j < 0.9, "jain index should degrade, got {}", j);
    }

    #[test]
    fn extract_client_ip_from_xff() {
        let ip = extract_client_ip(Some("203.0.113.5, 10.0.0.1"), None);
        assert_eq!(ip, "203.0.113.5".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn extract_client_ip_from_remote_addr() {
        let ip = extract_client_ip(None, Some("198.51.100.3:54321"));
        assert_eq!(ip, "198.51.100.3".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn extract_client_ip_fallback_localhost() {
        let ip = extract_client_ip(None, None);
        assert_eq!(ip, IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    }

    /// Simulate 10,000 concurrent requests across 100 IPs (100 req/IP).
    /// Each IP has a limit of 50 → expects exactly 50 allowed + 50 rejected per IP.
    #[test]
    fn concurrent_10k_requests() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let config = RateLimitConfig {
            base_rps: 50,
            min_rps: 50, // disable CPU scaling so result is deterministic
            cpu_low_pct: 0,
            cpu_high_pct: 100,
            window: Duration::from_secs(60), // long window so nothing expires
            ..Default::default()
        };
        let engine = Arc::new(RateLimitEngine::new(config));
        let allowed_total = Arc::new(AtomicUsize::new(0));
        let rejected_total = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::with_capacity(100);
        for a in 0u8..100 {
            let eng = Arc::clone(&engine);
            let allowed = Arc::clone(&allowed_total);
            let rejected = Arc::clone(&rejected_total);
            handles.push(std::thread::spawn(move || {
                let client = IpAddr::V4(Ipv4Addr::new(10, 0, 0, a));
                for _ in 0..100 {
                    let d = eng.check(client);
                    if d.allowed {
                        allowed.fetch_add(1, Ordering::Relaxed);
                    } else {
                        rejected.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
        }
        for h in handles { h.join().unwrap(); }

        let total_allowed = allowed_total.load(Ordering::Relaxed);
        let total_rejected = rejected_total.load(Ordering::Relaxed);

        assert_eq!(total_allowed + total_rejected, 10_000);
        // Each of 100 IPs gets 50 allowed, 50 rejected
        assert_eq!(total_allowed, 5_000, "expected exactly 5000 allowed");
        assert_eq!(total_rejected, 5_000, "expected exactly 5000 rejected");
        assert!(
            engine.metrics().total_requests >= 10_000,
            "metrics must count all requests"
        );
    }
}
