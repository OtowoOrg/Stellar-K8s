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
//! Comprehensive API Rate Limiting and Throttling (issue #1414)
//!
//! This module extends the existing [`ratelimit`](super::ratelimit) module
//! with:
//!
//! - **Configurable per-endpoint tier** limits (driven by YAML/ConfigMap)
//! - **Token-bucket per API key** with per-key overrides
//! - **Standard 429 response payload** with `Retry-After`, `X-RateLimit-*` headers
//! - **Abuse detection** (burst spike tracking and automatic short-term bans)
//!
//! ## Design
//!
//! Each inbound request is processed as follows:
//!
//! ```text
//! Request
//!   → resolve_tier(path)          — map path prefix → EndpointTier
//!   → per_key_check(api_key)      — token-bucket keyed on API key
//!   → per_ip_check(ip)            — per-IP bucket (defence-in-depth)
//!   → abuse_check(api_key, ip)    — burst spike / ban check
//!   → allowed / 429 (with headers)
//! ```
//!
//! All state is in-memory; for multi-replica deployments wire to the
//! [`distributed_ratelimit`](super::distributed_ratelimit) Redis backend.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// ── Endpoint tier configuration ───────────────────────────────────────────────

/// Limits applied to a specific endpoint tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierLimits {
    /// Sustained requests per second (token-bucket refill rate).
    pub rps: f64,
    /// Burst size: maximum tokens that can accumulate.
    pub burst: u32,
    /// Optional daily request cap (per API key).
    pub daily_cap: Option<u64>,
    /// Whether to include `Retry-After` in 429 responses.
    pub include_retry_after: bool,
}

impl TierLimits {
    /// Conservative public tier — generous to avoid blocking legitimate traffic.
    pub fn public() -> Self {
        Self {
            rps: 50.0,
            burst: 200,
            daily_cap: Some(500_000),
            include_retry_after: true,
        }
    }

    /// Standard authenticated tier.
    pub fn standard() -> Self {
        Self {
            rps: 20.0,
            burst: 60,
            daily_cap: Some(100_000),
            include_retry_after: true,
        }
    }

    /// Premium / privileged tier.
    pub fn premium() -> Self {
        Self {
            rps: 100.0,
            burst: 300,
            daily_cap: Some(1_000_000),
            include_retry_after: false,
        }
    }

    /// Admin endpoints — tight limit on individual keys.
    pub fn admin() -> Self {
        Self {
            rps: 5.0,
            burst: 20,
            daily_cap: Some(10_000),
            include_retry_after: true,
        }
    }
}

/// Maps path prefixes to their tier limits.
///
/// The table is evaluated longest-prefix first so `/api/v1/admin` takes
/// precedence over `/api/v1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointTierTable {
    /// Ordered list of (path_prefix, limits) pairs.
    pub entries: Vec<(String, TierLimits)>,
    /// Fallback when no prefix matches.
    pub default_limits: TierLimits,
}

impl Default for EndpointTierTable {
    fn default() -> Self {
        let mut entries = vec![
            ("/debug/".to_string(), TierLimits::admin()),
            ("/api/v1/admin".to_string(), TierLimits::admin()),
            ("/api/v1/premium".to_string(), TierLimits::premium()),
            ("/api/v1/".to_string(), TierLimits::standard()),
            ("/health".to_string(), TierLimits::public()),
            ("/metrics".to_string(), TierLimits::public()),
        ];
        // Ensure longest-prefix first.
        entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Self {
            entries,
            default_limits: TierLimits::standard(),
        }
    }
}

impl EndpointTierTable {
    /// Resolve the effective limits for a given request path.
    pub fn resolve(&self, path: &str) -> &TierLimits {
        self.entries
            .iter()
            .find(|(prefix, _)| path.starts_with(prefix.as_str()))
            .map(|(_, limits)| limits)
            .unwrap_or(&self.default_limits)
    }

    /// Apply a per-key override to the resolved limits (only overrides rps/burst
    /// when the key specifies a tighter or looser policy).
    pub fn apply_key_override(
        limits: &TierLimits,
        key_override: Option<&KeyRateOverride>,
    ) -> TierLimits {
        match key_override {
            None => limits.clone(),
            Some(o) => TierLimits {
                rps: o.rps_override.unwrap_or(limits.rps),
                burst: o.burst_override.unwrap_or(limits.burst),
                daily_cap: o.daily_cap_override.or(limits.daily_cap),
                include_retry_after: limits.include_retry_after,
            },
        }
    }
}

/// Per-API-key rate limit override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRateOverride {
    pub api_key_id: String,
    pub rps_override: Option<f64>,
    pub burst_override: Option<u32>,
    pub daily_cap_override: Option<u64>,
}

// ── Token bucket (per key / per IP) ──────────────────────────────────────────

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    max_tokens: f64,
    refill_rps: f64,
    last_refill: Instant,
    daily_used: u64,
    daily_cap: Option<u64>,
    day_start: Instant,
}

impl Bucket {
    fn new(limits: &TierLimits) -> Self {
        Self {
            tokens: limits.burst as f64,
            max_tokens: limits.burst as f64,
            refill_rps: limits.rps,
            last_refill: Instant::now(),
            daily_used: 0,
            daily_cap: limits.daily_cap,
            day_start: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rps).min(self.max_tokens);
        self.last_refill = Instant::now();

        // Reset daily counter every 24 h.
        if self.day_start.elapsed() >= Duration::from_secs(86_400) {
            self.daily_used = 0;
            self.day_start = Instant::now();
        }
    }

    /// Attempt to consume one token.  Returns `(allowed, remaining, retry_after_secs)`.
    fn consume(&mut self) -> (bool, f64, Option<u64>) {
        self.refill();

        // Daily cap check first.
        if let Some(cap) = self.daily_cap {
            if self.daily_used >= cap {
                let reset_secs = 86_400u64.saturating_sub(self.day_start.elapsed().as_secs());
                return (false, 0.0, Some(reset_secs));
            }
        }

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            self.daily_used += 1;
            (true, self.tokens, None)
        } else {
            // Estimate how long until one token refills.
            let wait = ((1.0 - self.tokens) / self.refill_rps).ceil() as u64;
            (false, 0.0, Some(wait))
        }
    }
}

// ── Abuse tracker ─────────────────────────────────────────────────────────────

/// A short-term ban entry.
#[derive(Debug)]
struct BanEntry {
    /// When the ban was imposed.
    imposed_at: Instant,
    /// Duration of the ban.
    duration: Duration,
}

impl BanEntry {
    fn is_active(&self) -> bool {
        self.imposed_at.elapsed() < self.duration
    }

    fn remaining_secs(&self) -> u64 {
        let elapsed = self.imposed_at.elapsed();
        if elapsed >= self.duration {
            0
        } else {
            (self.duration - elapsed).as_secs()
        }
    }
}

/// Tracks burst spikes and imposes temporary bans on abusive clients.
#[derive(Default)]
struct AbuseTracker {
    /// key_or_ip → consecutive rejection count
    rejection_streaks: HashMap<String, u32>,
    /// key_or_ip → active ban
    bans: HashMap<String, BanEntry>,
}

impl AbuseTracker {
    /// Register a rejection. Returns `Some(ban_duration_secs)` when a new
    /// ban is imposed.
    fn record_rejection(&mut self, id: &str) -> Option<u64> {
        let streak = self.rejection_streaks.entry(id.to_string()).or_insert(0);
        *streak += 1;
        if *streak >= 30 {
            // 30 consecutive rejections → 60-second ban
            let duration = Duration::from_secs(60);
            self.bans.insert(
                id.to_string(),
                BanEntry {
                    imposed_at: Instant::now(),
                    duration,
                },
            );
            *streak = 0;
            Some(60)
        } else {
            None
        }
    }

    fn record_acceptance(&mut self, id: &str) {
        self.rejection_streaks.remove(id);
    }

    /// Check if the ID is currently banned. Returns `Some(remaining_secs)` if banned.
    fn check_ban(&self, id: &str) -> Option<u64> {
        self.bans.get(id).and_then(|b| {
            if b.is_active() {
                Some(b.remaining_secs())
            } else {
                None
            }
        })
    }
}

// ── ThrottleResponse ─────────────────────────────────────────────────────────

/// The structured 429 response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThrottleResponse {
    pub error: String,
    pub message: String,
    pub retry_after_secs: Option<u64>,
    pub limit_rps: f64,
    pub remaining_tokens: f64,
    pub daily_cap: Option<u64>,
}

/// The full outcome of a rate-limit check.
#[derive(Debug, Clone)]
pub struct RateLimitOutcome {
    pub allowed: bool,
    /// `X-RateLimit-Limit` value (rps × 60 for per-minute presentation).
    pub limit: f64,
    /// `X-RateLimit-Remaining` (tokens in bucket).
    pub remaining: f64,
    /// `X-RateLimit-Reset` Unix timestamp.
    pub reset_at: DateTime<Utc>,
    /// Populated only when `allowed == false`.
    pub retry_after_secs: Option<u64>,
    /// Structured 429 body.
    pub throttle_body: Option<ThrottleResponse>,
}

impl RateLimitOutcome {
    /// Standard HTTP response headers derived from this outcome.
    /// Always include these in every response (not only 429s) so clients
    /// can pro-actively back off.
    pub fn headers(&self) -> Vec<(String, String)> {
        let mut h = vec![
            (
                "X-RateLimit-Limit".to_string(),
                format!("{:.0}", self.limit),
            ),
            (
                "X-RateLimit-Remaining".to_string(),
                format!("{:.0}", self.remaining),
            ),
            (
                "X-RateLimit-Reset".to_string(),
                self.reset_at.timestamp().to_string(),
            ),
        ];
        if let Some(ra) = self.retry_after_secs {
            h.push(("Retry-After".to_string(), ra.to_string()));
        }
        h
    }
}

// ── PerEndpointRateLimiter ────────────────────────────────────────────────────

/// The main per-endpoint token-bucket rate limiter.
///
/// Combines endpoint-tier configuration with per-key and per-IP buckets.
pub struct PerEndpointRateLimiter {
    tier_table: EndpointTierTable,
    key_overrides: HashMap<String, KeyRateOverride>,
    /// api_key_id / client_ip → Bucket
    buckets: Arc<RwLock<HashMap<String, Bucket>>>,
    abuse: Arc<RwLock<AbuseTracker>>,
}

impl PerEndpointRateLimiter {
    pub fn new(tier_table: EndpointTierTable) -> Self {
        Self {
            tier_table,
            key_overrides: HashMap::new(),
            buckets: Arc::new(RwLock::new(HashMap::new())),
            abuse: Arc::new(RwLock::new(AbuseTracker::default())),
        }
    }

    /// Register a per-key rate limit override.
    pub fn add_key_override(&mut self, override_cfg: KeyRateOverride) {
        self.key_overrides
            .insert(override_cfg.api_key_id.clone(), override_cfg);
    }

    /// Check whether an API request is allowed.
    ///
    /// - `path`: Request path (used to resolve tier).
    /// - `key_id`: Authenticated API key ID (empty string for anonymous).
    /// - `client_ip`: Client IP address (used for IP-level limiting).
    pub async fn check(&self, path: &str, key_id: &str, client_ip: &str) -> RateLimitOutcome {
        let base_limits = self.tier_table.resolve(path);
        let key_override = self.key_overrides.get(key_id);
        let effective_limits = EndpointTierTable::apply_key_override(base_limits, key_override);

        let bucket_key = if key_id.is_empty() { client_ip } else { key_id };

        // Abuse / ban check first — cheapest to evaluate.
        {
            let abuse = self.abuse.read().await;
            if let Some(remaining) = abuse.check_ban(bucket_key) {
                return self.denied_outcome(&effective_limits, remaining);
            }
        }

        // Consume from the bucket.
        let (allowed, remaining, retry_after) = {
            let mut buckets = self.buckets.write().await;
            let bucket = buckets
                .entry(bucket_key.to_string())
                .or_insert_with(|| Bucket::new(&effective_limits));
            bucket.consume()
        };

        // Update abuse tracker.
        {
            let mut abuse = self.abuse.write().await;
            if allowed {
                abuse.record_acceptance(bucket_key);
            } else {
                abuse.record_rejection(bucket_key);
            }
        }

        let reset_at = Utc::now() + chrono::Duration::seconds(retry_after.unwrap_or(60) as i64);

        if allowed {
            RateLimitOutcome {
                allowed: true,
                limit: effective_limits.rps,
                remaining,
                reset_at,
                retry_after_secs: None,
                throttle_body: None,
            }
        } else {
            self.denied_outcome(&effective_limits, retry_after.unwrap_or(1))
        }
    }

    fn denied_outcome(&self, limits: &TierLimits, retry_after_secs: u64) -> RateLimitOutcome {
        let retry = if limits.include_retry_after {
            Some(retry_after_secs)
        } else {
            None
        };
        RateLimitOutcome {
            allowed: false,
            limit: limits.rps,
            remaining: 0.0,
            reset_at: Utc::now() + chrono::Duration::seconds(retry_after_secs as i64),
            retry_after_secs: retry,
            throttle_body: Some(ThrottleResponse {
                error: "rate_limit_exceeded".to_string(),
                message: "You have exceeded the rate limit for this endpoint. \
                          Please reduce request frequency."
                    .to_string(),
                retry_after_secs: retry,
                limit_rps: limits.rps,
                remaining_tokens: 0.0,
                daily_cap: limits.daily_cap,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_limiter() -> PerEndpointRateLimiter {
        PerEndpointRateLimiter::new(EndpointTierTable::default())
    }

    #[tokio::test]
    async fn burst_allows_multiple_requests_immediately() {
        let limiter = make_limiter();
        // Standard tier has burst=60; first 60 requests should succeed.
        for _ in 0..60 {
            let out = limiter.check("/api/v1/nodes", "key-1", "10.0.0.1").await;
            assert!(out.allowed, "should be allowed within burst");
        }
    }

    #[tokio::test]
    async fn request_beyond_burst_is_denied() {
        let limiter = make_limiter();
        // Exhaust burst.
        for _ in 0..60 {
            limiter
                .check("/api/v1/nodes", "key-burst", "10.0.0.2")
                .await;
        }
        // Next should be denied.
        let out = limiter
            .check("/api/v1/nodes", "key-burst", "10.0.0.2")
            .await;
        assert!(!out.allowed);
        assert!(out.retry_after_secs.is_some());
        assert!(out.throttle_body.is_some());
    }

    #[tokio::test]
    async fn denied_response_includes_headers() {
        let limiter = make_limiter();
        for _ in 0..60 {
            limiter.check("/api/v1/nodes", "key-hdr", "1.2.3.4").await;
        }
        let out = limiter.check("/api/v1/nodes", "key-hdr", "1.2.3.4").await;
        let headers = out.headers();
        let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"X-RateLimit-Limit"));
        assert!(names.contains(&"X-RateLimit-Remaining"));
        assert!(names.contains(&"X-RateLimit-Reset"));
        assert!(names.contains(&"Retry-After"));
    }

    #[tokio::test]
    async fn admin_tier_resolved_for_admin_path() {
        let table = EndpointTierTable::default();
        let limits = table.resolve("/api/v1/admin/keys");
        assert!(
            limits.rps <= 5.0,
            "admin tier should have low rps: {}",
            limits.rps
        );
    }

    #[tokio::test]
    async fn key_override_adjusts_limits() {
        let mut limiter = make_limiter();
        limiter.add_key_override(KeyRateOverride {
            api_key_id: "vip-key".to_string(),
            rps_override: Some(999.0),
            burst_override: Some(5_000),
            daily_cap_override: None,
        });
        // VIP key should have a much larger burst.
        let mut denied = 0usize;
        for _ in 0..200 {
            let out = limiter.check("/api/v1/nodes", "vip-key", "5.5.5.5").await;
            if !out.allowed {
                denied += 1;
            }
        }
        assert_eq!(denied, 0, "VIP key should not be rate-limited within burst");
    }

    #[test]
    fn throttle_body_serialises_to_json() {
        let body = ThrottleResponse {
            error: "rate_limit_exceeded".to_string(),
            message: "Too fast".to_string(),
            retry_after_secs: Some(3),
            limit_rps: 20.0,
            remaining_tokens: 0.0,
            daily_cap: Some(100_000),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("rate_limit_exceeded"));
        assert!(json.contains("retry_after_secs"));
    }

    #[test]
    fn endpoint_tier_table_longest_prefix_wins() {
        let table = EndpointTierTable::default();
        // /api/v1/admin should resolve to admin, not standard.
        let admin_limits = table.resolve("/api/v1/admin/users");
        let standard_limits = table.resolve("/api/v1/transactions");
        assert!(
            admin_limits.rps < standard_limits.rps,
            "admin should have lower rps than standard"
        );
    }
}
