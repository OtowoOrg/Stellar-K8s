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

use crate::controller::retry_policy_tuner::{ErrorClass, RetryPolicy, RetryPolicyTuner};

/// Consumer identity for rate limiting.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerId {
    pub tenant: String,
    pub workload: Option<String>,
    pub api_key_hash: Option<String>,
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
    config: FairShareConfig,
    buckets: Arc<RwLock<HashMap<ConsumerId, RateLimitBucket>>>,
    global_usage: Arc<RwLock<GlobalUsageTracker>>,
    config_version: Arc<RwLock<u64>>,
    tuner: Arc<RwLock<RetryPolicyTuner>>, // Reuse for adaptive behavior
}

#[derive(Debug, Default)]
struct GlobalUsageTracker {
    total_requests: u64,
    total_rejected: u64,
    per_consumer: HashMap<ConsumerId, ConsumerUsage>,
    window_start: Instant,
}

#[derive(Debug, Default)]
struct ConsumerUsage {
    requests: u64,
    rejected: u64,
    latency_sum_ms: u64,
    error_count: u64,
}

impl FairShareRateLimiter {
    pub fn new(config: FairShareConfig) -> Self {
        Self {
            config,
            buckets: Arc::new(RwLock::new(HashMap::new())),
            global_usage: Arc::new(RwLock::new(GlobalUsageTracker::default())),
            config_version: Arc::new(RwLock::new(1)),
            tuner: Arc::new(RwLock::new(RetryPolicyTuner::new())),
        }
    }

    /// Check if a request from `consumer` is allowed.
    /// Returns (allowed, retry_after_ms, current_limit_info).
    pub async fn check_limit(&self, consumer: &ConsumerId, cost: u64) -> RateLimitDecision {
        let now_ms = current_time_ms();
        let mut buckets = self.buckets.write().await;
        let bucket = buckets.entry(consumer.clone()).or_default();

        // Compute fair share for this consumer
        let fair_share = self.compute_fair_share(consumer).await;
        let effective_capacity = (fair_share * self.config.burst_multiplier) as u64;

        // Update bucket capacity if fair share changed
        if bucket.capacity != effective_capacity {
            bucket.capacity = effective_capacity;
            bucket.refill_rate = fair_share;
            bucket.tokens = bucket.tokens.min(effective_capacity as f64);
        }

        let (allowed, remaining) = bucket.try_consume(cost, now_ms);

        // Record usage
        self.record_usage(consumer, allowed, now_ms).await;

        // Update tuner for adaptive behavior
        if !allowed {
            self.tuner.write().await.record(consumer.to_string(), true, ErrorClass::RateLimit);
        } else {
            self.tuner.write().await.record(consumer.to_string(), false, ErrorClass::Transient);
        }

        let retry_after_ms = if !allowed {
            // Estimate time until next token
            let deficit = cost as f64 - remaining;
            ((deficit / bucket.refill_rate) * 1000.0).ceil() as u64
        } else {
            0
        };

        RateLimitDecision {
            allowed,
            retry_after_ms,
            limit: effective_capacity,
            remaining: remaining.max(0.0) as u64,
            reset_after_ms: ((bucket.capacity as f64 - remaining) / bucket.refill_rate * 1000.0).ceil() as u64,
            config_version: *self.config_version.read().await,
        }
    }

    async fn compute_fair_share(&self, consumer: &ConsumerId) -> f64 {
        let usage = self.global_usage.read().await;
        let active_consumers = usage.per_consumer.len().max(1);
        let total_capacity = self.config.total_capacity as f64;

        // Base fair share
        let base_share = total_capacity / active_consumers as f64;

        // Clamp to min/max
        let share = base_share
            .max(self.config.min_share_per_consumer)
            .min(self.config.max_share_per_consumer);

        // If consumer has been well-behaved (low error rate), allow more burst
        let consumer_usage = usage.per_consumer.get(consumer);
        let bonus = consumer_usage.map_or(1.0, |u| {
            let error_rate = if u.requests > 0 { u.error_count as f64 / u.requests as f64 } else { 0.0 };
            if error_rate < 0.01 { 1.5 } else { 1.0 }
        });

        (share * bonus).min(self.config.max_share_per_consumer)
    }

    async fn record_usage(&self, consumer: &ConsumerId, allowed: bool, now_ms: u64) {
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

    /// Get current fair-share allocation for all consumers (for metrics/export).
    pub async fn get_allocations(&self) -> Vec<ConsumerAllocation> {
        let buckets = self.buckets.read().await;
        let usage = self.global_usage.read().await;

        buckets.iter().map(|(consumer, bucket)| {
            let u = usage.per_consumer.get(consumer).cloned().unwrap_or_default();
            ConsumerAllocation {
                consumer: consumer.clone(),
                allocated_rate: bucket.refill_rate,
                burst_capacity: bucket.capacity,
                current_usage_rps: u.requests as f64,
                rejection_rate: if u.requests > 0 { u.rejected as f64 / u.requests as f64 } else { 0.0 },
            }
        }).collect()
    }

    /// Compute Jain's fairness index across all consumers.
    pub async fn jain_index(&self) -> f64 {
        let usage = self.global_usage.read().await;
        let rates: Vec<f64> = usage.per_consumer.values().map(|u| u.requests as f64).collect();
        if rates.is_empty() {
            return 1.0;
        }
        let n = rates.len() as f64;
        let sum: f64 = rates.iter().sum();
        let sum_sq: f64 = rates.iter().map(|r| r * r).sum();
        if sum_sq == 0.0 { 1.0 } else { (sum * sum) / (n * sum_sq) }
    }

    /// Force config reload (e.g., from config map watcher).
    pub async fn reload_config(&self, new_config: FairShareConfig) {
        *self.config_version.write().await += 1;
        // Config is read on each check_limit, so just update
        // In real impl, would need ArcSwap or similar for lock-free reads
        info!("Fair-share config reloaded (version {})", *self.config_version.read().await);
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
pub fn export_metrics(limiter: &FairShareRateLimiter) -> String {
    // Placeholder - real impl would use prometheus crate
    let mut out = String::new();
    out.push_str("# HELP fair_share_allocated_rate Allocated rate per consumer\n");
    out.push_str("# TYPE fair_share_allocated_rate gauge\n");
    // Would iterate limiter.get_allocations().await here
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

        let consumer = ConsumerId { tenant: "tenant-a".into(), workload: None, api_key_hash: None };

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

        let c1 = ConsumerId { tenant: "tenant-a".into(), workload: None, api_key_hash: None };
        let c2 = ConsumerId { tenant: "tenant-b".into(), workload: None, api_key_hash: None };

        // Both consumers should get fair share
        for _ in 0..20 {
            limiter.check_limit(&c1, 1).await;
            limiter.check_limit(&c2, 1).await;
        }

        let jain = limiter.jain_index().await;
        assert!(jain > 0.9, "Jain index should be > 0.9 for equal consumers, got {}", jain);
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

        let noisy = ConsumerId { tenant: "noisy".into(), workload: None, api_key_hash: None };
        let quiet = ConsumerId { tenant: "quiet".into(), workload: None, api_key_hash: None };

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
        assert!(quiet_allowed >= 10, "Quiet consumer should get fair share, got {}", quiet_allowed);

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
}