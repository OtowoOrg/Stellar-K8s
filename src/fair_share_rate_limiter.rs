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
//! Dynamic Rate Limiting Based on Per-Consumer Fair Share
//!
//! Replaces static rate limits with dynamic per-consumer limits that adapt
//! to observed demand while protecting shared capacity.
//!
//! # Design
//!
//! Enforces at the edge proxy with a config channel driven by the fair-share
//! controller, keeping the hot path in proxy memory.
//!
//! ## Acceptance Criteria (from #1509)
//! - [ ] Noisy-consumer containment within 5s of saturation onset
//! - [ ] Well-behaved consumers see zero induced 429s
//! - [ ] Fair-share Jain index >= 0.9 under contention
//! - [ ] Limit config propagates in under 1s

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Maximum time allowed to contain a noisy consumer after saturation onset.
pub const CONTAINMENT_DEADLINE: Duration = Duration::from_secs(5);

use crate::controller::retry_policy_tuner::{ErrorClass, RetryPolicy, RetryPolicyTuner};

/// Consumer identity for rate limiting.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerId {
    pub tenant: String,
    pub workload: Option<String>,
    pub api_key_hash: Option<String>,
}

impl std::fmt::Display for ConsumerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.workload, &self.api_key_hash) {
            (Some(w), Some(k)) => write!(f, "{}:{}:{}", self.tenant, w, k),
            (Some(w), None) => write!(f, "{}:{}", self.tenant, w),
            (None, Some(k)) => write!(f, "{}:{}", self.tenant, k),
            (None, None) => write!(f, "{}", self.tenant),
        }
    }
}

/// Rate limit bucket configuration for a consumer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitBucket {
    /// Maximum tokens in the bucket (burst allowance).
    pub capacity: u64,

    /// Token refill rate per second.
    pub refill_rate: f64,

    /// Current token count (for state persistence).
    #[serde(default)]
    pub tokens: f64,

    /// Last refill timestamp.
    #[serde(default)]
    pub last_refill: Option<u64>, // Unix timestamp millis
}

impl Default for RateLimitBucket {
    fn default() -> Self {
        Self {
            capacity: 100,
            refill_rate: 10.0,
            tokens: 100.0,
            last_refill: None,
        }
    }
}

impl RateLimitBucket {
    /// Try to consume `n` tokens. Returns (allowed, remaining_tokens).
    pub fn try_consume(&mut self, n: u64, now_ms: u64) -> (bool, f64) {
        self.refill(now_ms);
        if self.tokens >= n as f64 {
            self.tokens -= n as f64;
            (true, self.tokens)
        } else {
            (false, self.tokens)
        }
    }

    fn refill(&mut self, now_ms: u64) {
        let last = self.last_refill.unwrap_or(now_ms);
        let elapsed_secs = (now_ms.saturating_sub(last)) as f64 / 1000.0;
        let new_tokens = (self.tokens + elapsed_secs * self.refill_rate).min(self.capacity as f64);
        self.tokens = new_tokens;
        self.last_refill = Some(now_ms);
    }
}

/// Fair-share configuration for the rate limiter.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FairShareConfig {
    /// Total capacity shared across all consumers (requests/sec).
    #[serde(default = "default_total_capacity")]
    pub total_capacity: u64,

    /// Minimum guaranteed share per consumer (requests/sec).
    #[serde(default = "default_min_share")]
    pub min_share_per_consumer: f64,

    /// Maximum share any single consumer can use (requests/sec).
    #[serde(default = "default_max_share")]
    pub max_share_per_consumer: f64,

    /// Burst multiplier for well-behaved consumers.
    #[serde(default = "default_burst_multiplier")]
    pub burst_multiplier: f64,

    /// Window for detecting saturation (seconds).
    #[serde(default = "default_saturation_window_secs")]
    pub saturation_window_secs: u64,

    /// Enable adaptive refill based on backend headroom.
    #[serde(default = "default_adaptive_refill")]
    pub adaptive_refill: bool,
}

fn default_total_capacity() -> u64 {
    10_000
}
fn default_min_share() -> f64 {
    10.0
}
fn default_max_share() -> f64 {
    1000.0
}
fn default_burst_multiplier() -> f64 {
    2.0
}
fn default_saturation_window_secs() -> u64 {
    10
}
fn default_adaptive_refill() -> bool {
    true
}

/// Dynamic rate limiter with fair-share allocation.
pub struct FairShareRateLimiter {
    config: Arc<RwLock<FairShareConfig>>,
    buckets: Arc<RwLock<HashMap<ConsumerId, RateLimitBucket>>>,
    global_usage: Arc<RwLock<GlobalUsageTracker>>,
    config_version: Arc<RwLock<u64>>,
    tuner: Arc<RwLock<RetryPolicyTuner>>, // Reuse for adaptive behavior
    /// Per-consumer observed demand (requests/sec) over the last window.
    demand: Arc<RwLock<HashMap<ConsumerId, DemandSample>>>,
    /// When saturation was first observed (for containment latency tracking).
    saturation_onset: Arc<RwLock<Option<Instant>>>,
    /// Metrics for containment latency and induced 429s.
    metrics: Arc<RwLock<LimiterMetrics>>,
}

/// Observed demand for a consumer over a rolling window.
#[derive(Debug, Clone)]
struct DemandSample {
    /// Requests observed since the window started.
    requests: u64,
    /// When the current demand window started.
    window_start: Instant,
    /// Smoothed demand estimate (requests/sec), EWMA.
    smoothed_rps: f64,
    /// Whether this consumer has been flagged as noisy.
    noisy: bool,
}

impl DemandSample {
    fn new(now: Instant) -> Self {
        Self {
            requests: 0,
            window_start: now,
            smoothed_rps: 0.0,
            noisy: false,
        }
    }
}

/// Metrics exposed for observability of the fair-share limiter.
#[derive(Debug, Default, Clone)]
pub struct LimiterMetrics {
    /// Number of consumers currently flagged as noisy.
    pub noisy_consumers: u64,
    /// Total requests rejected due to fair-share limits.
    pub total_rejected: u64,
    /// Total requests allowed.
    pub total_allowed: u64,
    /// Rejections attributed to well-behaved consumers (induced 429s).
    pub induced_429s: u64,
    /// Last observed containment latency (ms) from saturation onset to containment.
    pub last_containment_latency_ms: Option<u64>,
    /// Number of times a noisy consumer was contained.
    pub containment_events: u64,
}

#[derive(Debug)]
struct GlobalUsageTracker {
    total_requests: u64,
    total_rejected: u64,
    per_consumer: HashMap<ConsumerId, ConsumerUsage>,
    window_start: Instant,
}

impl Default for GlobalUsageTracker {
    fn default() -> Self {
        Self {
            total_requests: 0,
            total_rejected: 0,
            per_consumer: HashMap::new(),
            window_start: Instant::now(),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct ConsumerUsage {
    requests: u64,
    rejected: u64,
    latency_sum_ms: u64,
    error_count: u64,
}

impl FairShareRateLimiter {
    pub fn new(config: FairShareConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            buckets: Arc::new(RwLock::new(HashMap::new())),
            global_usage: Arc::new(RwLock::new(GlobalUsageTracker::default())),
            config_version: Arc::new(RwLock::new(1)),
            tuner: Arc::new(RwLock::new(RetryPolicyTuner::new())),
            demand: Arc::new(RwLock::new(HashMap::new())),
            saturation_onset: Arc::new(RwLock::new(None)),
            metrics: Arc::new(RwLock::new(LimiterMetrics::default())),
        }
    }

    /// Check if a request from `consumer` is allowed.
    /// Returns (allowed, retry_after_ms, current_limit_info).
    pub async fn check_limit(&self, consumer: &ConsumerId, cost: u64) -> RateLimitDecision {
        let now_ms = current_time_ms();
        let now = Instant::now();
        let config = self.config.read().await.clone();

        // Observe demand for this consumer before deciding.
        self.observe_demand(consumer, now, &config).await;

        // Compute fair share for this consumer (dynamic, based on observed demand).
        let fair_share = self.compute_fair_share(consumer, &config).await;
        let effective_capacity = ((fair_share * config.burst_multiplier) as u64).max(1);

        let mut buckets = self.buckets.write().await;
        let bucket = buckets.entry(consumer.clone()).or_default();

        // Update bucket capacity if fair share changed
        if bucket.capacity != effective_capacity {
            bucket.capacity = effective_capacity;
            bucket.refill_rate = fair_share;
            bucket.tokens = bucket.tokens.min(effective_capacity as f64);
        }

        let (allowed, remaining) = bucket.try_consume(cost, now_ms);

        let retry_after_ms = if !allowed {
            // Estimate time until next token
            let deficit = cost as f64 - remaining;
            ((deficit / bucket.refill_rate.max(f64::EPSILON)) * 1000.0).ceil() as u64
        } else {
            0
        };

        let reset_after_ms = ((bucket.capacity as f64 - remaining)
            / bucket.refill_rate.max(f64::EPSILON)
            * 1000.0)
            .ceil() as u64;

        drop(buckets);

        // Record usage and update metrics.
        self.record_usage(consumer, allowed, now_ms).await;
        self.update_metrics(consumer, allowed).await;

        // Update tuner for adaptive behavior
        if !allowed {
            self.tuner
                .write()
                .await
                .record(&consumer.to_string(), true, Some(ErrorClass::RateLimit));
        } else {
            self.tuner.write().await.record(
                &consumer.to_string(),
                false,
                Some(ErrorClass::Transient),
            );
        }

        RateLimitDecision {
            allowed,
            retry_after_ms,
            limit: effective_capacity,
            remaining: remaining.max(0.0) as u64,
            reset_after_ms,
            config_version: *self.config_version.read().await,
        }
    }

    /// Observe demand for a consumer, updating the smoothed rate estimate and
    /// flagging noisy consumers when their demand exceeds their fair share.
    async fn observe_demand(&self, consumer: &ConsumerId, now: Instant, config: &FairShareConfig) {
        let window = Duration::from_secs(config.saturation_window_secs.max(1));
        let mut demand = self.demand.write().await;
        let sample = demand
            .entry(consumer.clone())
            .or_insert_with(|| DemandSample::new(now));

        sample.requests += 1;
        let elapsed = now.saturating_duration_since(sample.window_start);
        if elapsed >= window {
            let observed_rps = sample.requests as f64 / elapsed.as_secs_f64().max(f64::EPSILON);
            // EWMA smoothing so a single burst doesn't immediately flip the flag.
            sample.smoothed_rps = if sample.smoothed_rps == 0.0 {
                observed_rps
            } else {
                0.5 * sample.smoothed_rps + 0.5 * observed_rps
            };
            sample.requests = 0;
            sample.window_start = now;
        }
    }

    /// Compute the dynamic fair share for a consumer based on observed demand.
    async fn compute_fair_share(&self, consumer: &ConsumerId, config: &FairShareConfig) -> f64 {
        let usage = self.global_usage.read().await;
        let demand = self.demand.read().await;

        let active_consumers = usage.per_consumer.len().max(1);
        let total_capacity = config.total_capacity as f64;

        // Base fair share: equal split of total capacity among active consumers.
        let base_share = total_capacity / active_consumers as f64;

        // Clamp to min/max.
        let share = base_share
            .max(config.min_share_per_consumer)
            .min(config.max_share_per_consumer);

        // Well-behaved consumers (low error rate) get a burst bonus.
        let consumer_usage = usage.per_consumer.get(consumer);
        let bonus = consumer_usage.map_or(1.0, |u| {
            let error_rate = if u.requests > 0 {
                u.error_count as f64 / u.requests as f64
            } else {
                0.0
            };
            if error_rate < 0.01 {
                1.5
            } else {
                1.0
            }
        });

        // Noisy consumers are throttled down to their fair share (no bonus),
        // while well-behaved consumers keep their bonus.
        let is_noisy = demand.get(consumer).map_or(false, |d| d.noisy);
        let effective_bonus = if is_noisy { 1.0 } else { bonus };

        (share * effective_bonus).min(config.max_share_per_consumer)
    }

    /// Re-evaluate noisy-consumer flags based on observed demand vs. fair share.
    /// Called periodically by the controller to contain noisy consumers.
    pub async fn reevaluate_noisy_consumers(&self) {
        let config = self.config.read().await.clone();
        let total_capacity = config.total_capacity as f64;
        let mut demand = self.demand.write().await;
        let active = demand.len().max(1) as f64;
        let fair_share = (total_capacity / active)
            .max(config.min_share_per_consumer)
            .min(config.max_share_per_consumer);

        let mut newly_noisy = 0u64;
        for sample in demand.values_mut() {
            // A consumer is noisy if its smoothed demand exceeds its fair share
            // by more than the burst multiplier allows.
            let threshold = fair_share * config.burst_multiplier;
            let was_noisy = sample.noisy;
            sample.noisy = sample.smoothed_rps > threshold;
            if sample.noisy && !was_noisy {
                newly_noisy += 1;
            }
        }

        if newly_noisy > 0 {
            let mut metrics = self.metrics.write().await;
            metrics.noisy_consumers = demand.values().filter(|d| d.noisy).count() as u64;
            metrics.containment_events += newly_noisy;

            // Record containment latency from saturation onset.
            let mut onset = self.saturation_onset.write().await;
            if let Some(start) = *onset {
                let latency = start.elapsed();
                metrics.last_containment_latency_ms = Some(latency.as_millis() as u64);
                if latency > CONTAINMENT_DEADLINE {
                    warn!(
                        "Noisy-consumer containment took {}ms (deadline {}ms)",
                        latency.as_millis(),
                        CONTAINMENT_DEADLINE.as_millis()
                    );
                }
                *onset = None;
            }
        }
    }

    /// Mark saturation onset (called by the controller when capacity saturates).
    pub async fn mark_saturation_onset(&self) {
        let mut onset = self.saturation_onset.write().await;
        if onset.is_none() {
            *onset = Some(Instant::now());
        }
    }

    async fn record_usage(&self, consumer: &ConsumerId, allowed: bool, _now_ms: u64) {
        let mut usage = self.global_usage.write().await;
        let entry = usage.per_consumer.entry(consumer.clone()).or_default();
        entry.requests += 1;
        if !allowed {
            entry.rejected += 1;
        }
        usage.total_requests += 1;
        if !allowed {
            usage.total_rejected += 1;
        }
    }

    async fn update_metrics(&self, consumer: &ConsumerId, allowed: bool) {
        let mut metrics = self.metrics.write().await;
        if allowed {
            metrics.total_allowed += 1;
        } else {
            metrics.total_rejected += 1;
            // A rejection for a consumer that is not flagged noisy is an
            // "induced" 429 against a well-behaved consumer.
            let demand = self.demand.read().await;
            let is_noisy = demand.get(consumer).map_or(false, |d| d.noisy);
            if !is_noisy {
                metrics.induced_429s += 1;
            }
        }
    }

    /// Snapshot of the current limiter metrics.
    pub async fn metrics(&self) -> LimiterMetrics {
        self.metrics.read().await.clone()
    }

    /// Get current fair-share allocation for all consumers (for metrics/export).
    pub async fn get_allocations(&self) -> Vec<ConsumerAllocation> {
        let buckets = self.buckets.read().await;
        let usage = self.global_usage.read().await;

        buckets
            .iter()
            .map(|(consumer, bucket)| {
                let u = usage
                    .per_consumer
                    .get(consumer)
                    .cloned()
                    .unwrap_or_default();
                ConsumerAllocation {
                    consumer: consumer.clone(),
                    allocated_rate: bucket.refill_rate,
                    burst_capacity: bucket.capacity,
                    current_usage_rps: u.requests as f64,
                    rejection_rate: if u.requests > 0 {
                        u.rejected as f64 / u.requests as f64
                    } else {
                        0.0
                    },
                }
            })
            .collect()
    }

    /// Compute Jain's fairness index across all consumers.
    pub async fn jain_index(&self) -> f64 {
        let usage = self.global_usage.read().await;
        let rates: Vec<f64> = usage
            .per_consumer
            .values()
            .map(|u| u.requests as f64)
            .collect();
        if rates.is_empty() {
            return 1.0;
        }
        let n = rates.len() as f64;
        let sum: f64 = rates.iter().sum();
        let sum_sq: f64 = rates.iter().map(|r| r * r).sum();
        if sum_sq == 0.0 {
            1.0
        } else {
            (sum * sum) / (n * sum_sq)
        }
    }

    /// Force config reload (e.g., from config map watcher).
    pub async fn reload_config(&self, new_config: FairShareConfig) {
        *self.config.write().await = new_config;
        *self.config_version.write().await += 1;
        info!(
            "Fair-share config reloaded (version {})",
            *self.config_version.read().await
        );
    }
}

/// Decision returned by the rate limiter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitDecision {
    pub allowed: bool,
    pub retry_after_ms: u64,
    pub limit: u64,
    pub remaining: u64,
    pub reset_after_ms: u64,
    pub config_version: u64,
}

/// Allocation info for metrics export.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerAllocation {
    pub consumer: ConsumerId,
    pub allocated_rate: f64,
    pub burst_capacity: u64,
    pub current_usage_rps: f64,
    pub rejection_rate: f64,
}

/// Export Prometheus metrics for the rate limiter.
pub async fn export_metrics(limiter: &FairShareRateLimiter) -> String {
    let mut out = String::new();

    out.push_str("# HELP fair_share_allocated_rate Allocated rate per consumer\n");
    out.push_str("# TYPE fair_share_allocated_rate gauge\n");
    for alloc in limiter.get_allocations().await {
        out.push_str(&format!(
            "fair_share_allocated_rate{{consumer=\"{}\"}} {}\n",
            alloc.consumer, alloc.allocated_rate
        ));
    }

    let metrics = limiter.metrics().await;
    out.push_str("# HELP fair_share_noisy_consumers Consumers currently flagged noisy\n");
    out.push_str("# TYPE fair_share_noisy_consumers gauge\n");
    out.push_str(&format!(
        "fair_share_noisy_consumers {}\n",
        metrics.noisy_consumers
    ));

    out.push_str("# HELP fair_share_induced_429s Rejections against well-behaved consumers\n");
    out.push_str("# TYPE fair_share_induced_429s counter\n");
    out.push_str(&format!(
        "fair_share_induced_429s {}\n",
        metrics.induced_429s
    ));

    out.push_str("# HELP fair_share_total_rejected Total rejected requests\n");
    out.push_str("# TYPE fair_share_total_rejected counter\n");
    out.push_str(&format!(
        "fair_share_total_rejected {}\n",
        metrics.total_rejected
    ));

    out.push_str("# HELP fair_share_containment_events Noisy-consumer containment events\n");
    out.push_str("# TYPE fair_share_containment_events counter\n");
    out.push_str(&format!(
        "fair_share_containment_events {}\n",
        metrics.containment_events
    ));

    out.push_str("# HELP fair_share_containment_latency_ms Last containment latency\n");
    out.push_str("# TYPE fair_share_containment_latency_ms gauge\n");
    if let Some(latency) = metrics.last_containment_latency_ms {
        out.push_str(&format!(
            "fair_share_containment_latency_ms {}\n",
            latency
        ));
    }

    out
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_rate_limiting() {
        let config = FairShareConfig::default();
        let limiter = FairShareRateLimiter::new(config);

        let consumer = ConsumerId {
            tenant: "tenant-a".into(),
            workload: None,
            api_key_hash: None,
        };

        // Should allow up to capacity
        for i in 0..50 {
            let decision = limiter.check_limit(&consumer, 1).await;
            assert!(decision.allowed, "Request {} should be allowed", i);
        }

        // Should start rejecting
        let mut rejected = 0;
        for _ in 0..100 {
            let decision = limiter.check_limit(&consumer, 1).await;
            if !decision.allowed {
                rejected += 1;
            }
        }
        assert!(rejected > 0, "Should reject after capacity exhausted");
    }

    #[tokio::test]
    async fn test_fair_share_between_consumers() {
        let config = FairShareConfig {
            total_capacity: 100,
            min_share_per_consumer: 10.0,
            max_share_per_consumer: 50.0,
            ..Default::default()
        };
        let limiter = FairShareRateLimiter::new(config);

        let c1 = ConsumerId {
            tenant: "tenant-a".into(),
            workload: None,
            api_key_hash: None,
        };
        let c2 = ConsumerId {
            tenant: "tenant-b".into(),
            workload: None,
            api_key_hash: None,
        };

        // Both consumers should get fair share
        for _ in 0..20 {
            limiter.check_limit(&c1, 1).await;
            limiter.check_limit(&c2, 1).await;
        }

        let jain = limiter.jain_index().await;
        assert!(
            jain > 0.9,
            "Jain index should be > 0.9 for equal consumers, got {}",
            jain
        );
    }

    #[tokio::test]
    async fn test_noisy_consumer_containment() {
        let config = FairShareConfig {
            total_capacity: 100,
            min_share_per_consumer: 5.0,
            max_share_per_consumer: 20.0,
            burst_multiplier: 1.5,
            ..Default::default()
        };
        let limiter = FairShareRateLimiter::new(config);

        let noisy = ConsumerId {
            tenant: "noisy".into(),
            workload: None,
            api_key_hash: None,
        };
        let quiet = ConsumerId {
            tenant: "quiet".into(),
            workload: None,
            api_key_hash: None,
        };

        // Noisy consumer hammers the API
        for _ in 0..50 {
            limiter.check_limit(&noisy, 1).await;
        }

        // Quiet consumer should still get their fair share
        let mut quiet_allowed = 0;
        for _ in 0..20 {
            let decision = limiter.check_limit(&quiet, 1).await;
            if decision.allowed {
                quiet_allowed += 1;
            }
        }
        assert!(
            quiet_allowed >= 10,
            "Quiet consumer should get fair share, got {}",
            quiet_allowed
        );

        // Jain index should still be reasonable
        let jain = limiter.jain_index().await;
        assert!(jain > 0.7, "Jain index degraded: {}", jain);
    }

    #[test]
    fn test_rate_limit_bucket_refill() {
        let mut bucket = RateLimitBucket {
            capacity: 10,
            refill_rate: 5.0, // 5 tokens/sec
            tokens: 10.0,
            last_refill: Some(0),
        };

        // Consume all
        assert!(bucket.try_consume(10, 0).0);
        assert!(!bucket.try_consume(1, 0).0);

        // Wait 2 seconds (simulated)
        assert!(bucket.try_consume(1, 2000).0); // Should have ~10 tokens after 2s refill
        assert_eq!(bucket.tokens, 10.0); // Capped at capacity
    }

    fn consumer(name: &str) -> ConsumerId {
        ConsumerId {
            tenant: name.into(),
            workload: None,
            api_key_hash: None,
        }
    }

    #[tokio::test]
    async fn test_config_reload_propagates() {
        let limiter = FairShareRateLimiter::new(FairShareConfig::default());
        let before = *limiter.config_version.read().await;

        limiter
            .reload_config(FairShareConfig {
                total_capacity: 500,
                ..Default::default()
            })
            .await;

        let after = *limiter.config_version.read().await;
        assert!(after > before, "config version must advance on reload");
        assert_eq!(limiter.config.read().await.total_capacity, 500);
    }

    #[tokio::test]
    async fn test_noisy_consumer_flagged_and_contained() {
        let config = FairShareConfig {
            total_capacity: 100,
            min_share_per_consumer: 5.0,
            max_share_per_consumer: 20.0,
            burst_multiplier: 1.5,
            saturation_window_secs: 1,
            ..Default::default()
        };
        let limiter = FairShareRateLimiter::new(config);

        let noisy = consumer("noisy");
        let quiet = consumer("quiet");

        limiter.mark_saturation_onset().await;

        // Drive the noisy consumer's observed demand above its fair share.
        for _ in 0..200 {
            limiter.check_limit(&noisy, 1).await;
        }
        // Give the demand window a chance to roll over.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        for _ in 0..200 {
            limiter.check_limit(&noisy, 1).await;
        }

        limiter.reevaluate_noisy_consumers().await;

        let metrics = limiter.metrics().await;
        assert!(
            metrics.noisy_consumers >= 1,
            "noisy consumer should be flagged"
        );
        assert!(
            metrics.containment_events >= 1,
            "containment should be recorded"
        );
        if let Some(latency) = metrics.last_containment_latency_ms {
            assert!(
                latency <= CONTAINMENT_DEADLINE.as_millis() as u64,
                "containment latency {}ms exceeds deadline",
                latency
            );
        }

        // Quiet consumer should not be flagged.
        let demand = limiter.demand.read().await;
        assert!(!demand.get(&quiet).map_or(false, |d| d.noisy));
    }

    #[tokio::test]
    async fn test_load_one_abusive_twenty_normal() {
        // Load test: one abusive consumer and 20 normal consumers.
        // Normal consumer success rate must stay at 100%.
        let config = FairShareConfig {
            total_capacity: 1000,
            min_share_per_consumer: 20.0,
            max_share_per_consumer: 100.0,
            burst_multiplier: 2.0,
            saturation_window_secs: 1,
            ..Default::default()
        };
        let limiter = FairShareRateLimiter::new(config);

        let abusive = consumer("abusive");
        let normals: Vec<ConsumerId> = (0..20).map(|i| consumer(&format!("normal-{}", i))).collect();

        limiter.mark_saturation_onset().await;

        // Abusive consumer floods the limiter.
        for _ in 0..500 {
            limiter.check_limit(&abusive, 1).await;
        }
        tokio::time::sleep(Duration::from_millis(1100)).await;
        for _ in 0..500 {
            limiter.check_limit(&abusive, 1).await;
        }
        limiter.reevaluate_noisy_consumers().await;

        // Normal consumers issue a modest, well-behaved load.
        let mut normal_allowed = 0usize;
        let mut normal_total = 0usize;
        for _ in 0..10 {
            for c in &normals {
                normal_total += 1;
                if limiter.check_limit(c, 1).await.allowed {
                    normal_allowed += 1;
                }
            }
        }

        let success_rate = normal_allowed as f64 / normal_total as f64;
        assert_eq!(
            success_rate, 1.0,
            "normal consumers must see 100% success, got {}",
            success_rate
        );

        // Jain index should remain high under contention.
        let jain = limiter.jain_index().await;
        assert!(
            jain >= 0.9,
            "Jain index should be >= 0.9 under contention, got {}",
            jain
        );
    }
}
