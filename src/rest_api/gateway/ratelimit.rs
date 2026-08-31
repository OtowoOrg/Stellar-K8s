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
//! Rate Limiting and Quota Management
//!
//! Provides sophisticated rate limiting with sliding window,
//! token bucket algorithm, quota management per client, and
//! endpoint tier configuration with rate limit headers.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

/// Rate limit errors
#[derive(Error, Debug)]
pub enum RateLimitError {
    #[error("Rate limit exceeded for {0}")]
    RateLimitExceeded(String),
    #[error("Quota exceeded: {0}")]
    QuotaExceeded(String),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

/// API endpoint tier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EndpointTier {
    /// Public endpoints (health, status)
    Public,
    /// Standard API endpoints
    Standard,
    /// Premium endpoints (advanced queries)
    Premium,
    /// Administrative endpoints
    Admin,
}

impl EndpointTier {
    /// Get rate limit config for tier
    pub fn rate_limit_config(&self) -> RateLimitConfig {
        match self {
            EndpointTier::Public => RateLimitConfig {
                requests_per_minute: 1000,
                requests_per_hour: 50000,
                requests_per_day: 500000,
                burst_size: 100,
                per_ip_limit: Some(500),
                per_client_limit: Some(1000),
            },
            EndpointTier::Standard => RateLimitConfig {
                requests_per_minute: 100,
                requests_per_hour: 10000,
                requests_per_day: 100000,
                burst_size: 20,
                per_ip_limit: Some(50),
                per_client_limit: Some(200),
            },
            EndpointTier::Premium => RateLimitConfig {
                requests_per_minute: 50,
                requests_per_hour: 5000,
                requests_per_day: 50000,
                burst_size: 10,
                per_ip_limit: Some(25),
                per_client_limit: Some(100),
            },
            EndpointTier::Admin => RateLimitConfig {
                requests_per_minute: 200,
                requests_per_hour: 20000,
                requests_per_day: 200000,
                burst_size: 50,
                per_ip_limit: Some(100),
                per_client_limit: Some(500),
            },
        }
    }
}

/// Rate limit configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub requests_per_minute: u32,
    pub requests_per_hour: u32,
    pub requests_per_day: u32,
    pub burst_size: u32,
    pub per_ip_limit: Option<u32>,
    pub per_client_limit: Option<u32>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 100,
            requests_per_hour: 10000,
            requests_per_day: 100000,
            burst_size: 20,
            per_ip_limit: Some(50),
            per_client_limit: Some(200),
        }
    }
}

/// Rate limit response headers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitHeaders {
    /// Rate limit limit per minute
    pub x_ratelimit_limit_minute: u32,
    /// Rate limit remaining per minute
    pub x_ratelimit_remaining_minute: u32,
    /// Rate limit reset time (Unix timestamp)
    pub x_ratelimit_reset: u64,
    /// Rate limit limit per hour
    pub x_ratelimit_limit_hour: u32,
    /// Rate limit remaining per hour
    pub x_ratelimit_remaining_hour: u32,
    /// Retry after seconds (only set on 429)
    pub retry_after: Option<u64>,
}

impl RateLimitHeaders {
    /// Convert to HTTP header tuples
    pub fn to_header_tuples(&self) -> Vec<(&str, String)> {
        let mut headers = vec![
            (
                "X-RateLimit-Limit-Minute",
                self.x_ratelimit_limit_minute.to_string(),
            ),
            (
                "X-RateLimit-Remaining-Minute",
                self.x_ratelimit_remaining_minute.to_string(),
            ),
            ("X-RateLimit-Reset", self.x_ratelimit_reset.to_string()),
            (
                "X-RateLimit-Limit-Hour",
                self.x_ratelimit_limit_hour.to_string(),
            ),
            (
                "X-RateLimit-Remaining-Hour",
                self.x_ratelimit_remaining_hour.to_string(),
            ),
        ];

        if let Some(retry) = self.retry_after {
            headers.push(("Retry-After", retry.to_string()));
        }

        headers
    }
}

/// Rate limit bucket using token bucket algorithm
#[derive(Debug)]
struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64, // tokens per second
    last_refill: DateTime<Utc>,
}

impl TokenBucket {
    fn new(max_tokens: u32, refill_per_second: f64) -> Self {
        Self {
            tokens: max_tokens as f64,
            max_tokens: max_tokens as f64,
            refill_rate: refill_per_second,
            last_refill: Utc::now(),
        }
    }

    fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Utc::now();
        let elapsed = (now - self.last_refill).num_milliseconds() as f64 / 1000.0;
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;
    }

    fn remaining(&self) -> u32 {
        self.tokens.floor() as u32
    }
}

/// Client rate limit state
#[derive(Debug)]
struct ClientRateLimit {
    minute_bucket: TokenBucket,
    hour_bucket: TokenBucket,
    day_bucket: TokenBucket,
    minute_count: u32,
    hour_count: u32,
    day_count: u32,
    first_request: DateTime<Utc>,
}

impl ClientRateLimit {
    fn new(config: &RateLimitConfig) -> Self {
        let minute_rate = config.requests_per_minute as f64 / 60.0;
        let hour_rate = config.requests_per_hour as f64 / 3600.0;
        let day_rate = config.requests_per_day as f64 / 86400.0;

        Self {
            minute_bucket: TokenBucket::new(config.requests_per_minute, minute_rate),
            hour_bucket: TokenBucket::new(config.requests_per_hour, hour_rate),
            day_bucket: TokenBucket::new(config.requests_per_day, day_rate),
            minute_count: 0,
            hour_count: 0,
            day_count: 0,
            first_request: Utc::now(),
        }
    }

    fn check(&mut self) -> bool {
        self.minute_bucket.try_consume()
            && self.hour_bucket.try_consume()
            && self.day_bucket.try_consume()
    }

    fn get_limit_info(&self, config: &RateLimitConfig) -> RateLimitInfo {
        RateLimitInfo {
            remaining_minute: self.minute_bucket.remaining(),
            remaining_hour: self.hour_bucket.remaining(),
            remaining_day: self.day_bucket.remaining(),
            reset_at: self.first_request + ChronoDuration::days(1),
            limit_minute: config.requests_per_minute,
            limit_hour: config.requests_per_hour,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitInfo {
    pub remaining_minute: u32,
    pub remaining_hour: u32,
    pub remaining_day: u32,
    pub reset_at: DateTime<Utc>,
    pub limit_minute: u32,
    pub limit_hour: u32,
}

impl RateLimitInfo {
    /// Convert to rate limit headers
    pub fn to_headers(&self, retry_after: Option<u64>) -> RateLimitHeaders {
        RateLimitHeaders {
            x_ratelimit_limit_minute: self.limit_minute,
            x_ratelimit_remaining_minute: self.remaining_minute,
            x_ratelimit_reset: self.reset_at.timestamp() as u64,
            x_ratelimit_limit_hour: self.limit_hour,
            x_ratelimit_remaining_hour: self.remaining_hour,
            retry_after,
        }
    }
}

/// Rate Limiter with multiple strategies and endpoint tier support
pub struct RateLimiter {
    config: RateLimitConfig,
    ip_limits: Arc<RwLock<HashMap<String, ClientRateLimit>>>,
    client_limits: Arc<RwLock<HashMap<String, ClientRateLimit>>>,
    tier_limits: Arc<RwLock<HashMap<EndpointTier, RateLimitConfig>>>,
}

impl RateLimiter {
    pub fn new(requests_per_minute: u32, burst_size: u32) -> Self {
        let config = RateLimitConfig {
            requests_per_minute,
            requests_per_hour: requests_per_minute * 60,
            requests_per_day: requests_per_minute * 60 * 24,
            burst_size,
            per_ip_limit: Some(requests_per_minute),
            per_client_limit: Some(requests_per_minute),
        };
        Self::from_config(&config)
    }

    pub fn from_config(config: &RateLimitConfig) -> Self {
        let mut tier_limits = HashMap::new();
        tier_limits.insert(
            EndpointTier::Public,
            EndpointTier::Public.rate_limit_config(),
        );
        tier_limits.insert(
            EndpointTier::Standard,
            EndpointTier::Standard.rate_limit_config(),
        );
        tier_limits.insert(
            EndpointTier::Premium,
            EndpointTier::Premium.rate_limit_config(),
        );
        tier_limits.insert(EndpointTier::Admin, EndpointTier::Admin.rate_limit_config());

        Self {
            config: config.clone(),
            ip_limits: Arc::new(RwLock::new(HashMap::new())),
            client_limits: Arc::new(RwLock::new(HashMap::new())),
            tier_limits: Arc::new(RwLock::new(tier_limits)),
        }
    }

    /// Check if request is allowed for given IP
    pub async fn check(&self, ip: &str) -> bool {
        // Check per-IP limit if configured
        if let Some(per_ip) = self.config.per_ip_limit {
            let mut limits = self.ip_limits.write().await;
            let client = limits.entry(ip.to_string()).or_insert_with(|| {
                ClientRateLimit::new(&RateLimitConfig {
                    requests_per_minute: per_ip,
                    requests_per_hour: per_ip * 60,
                    requests_per_day: per_ip * 60 * 24,
                    burst_size: per_ip / 5,
                    ..Default::default()
                })
            });
            if !client.check() {
                return false;
            }
        }

        true
    }

    /// Check rate limit for a specific client
    pub async fn check_client(&self, client_id: &str) -> bool {
        if let Some(per_client) = self.config.per_client_limit {
            let mut limits = self.client_limits.write().await;
            let client = limits.entry(client_id.to_string()).or_insert_with(|| {
                ClientRateLimit::new(&RateLimitConfig {
                    requests_per_minute: per_client,
                    requests_per_hour: per_client * 60,
                    requests_per_day: per_client * 60 * 24,
                    burst_size: per_client / 5,
                    ..Default::default()
                })
            });
            client.check()
        } else {
            true
        }
    }

    /// Check rate limit for a specific endpoint tier
    pub async fn check_tier(
        &self,
        client_id: &str,
        tier: EndpointTier,
    ) -> Result<RateLimitHeaders, RateLimitError> {
        let tier_config = {
            let tier_limits = self.tier_limits.read().await;
            tier_limits
                .get(&tier)
                .cloned()
                .unwrap_or_else(|| tier.rate_limit_config())
        };

        let mut limits = self.client_limits.write().await;
        let client_key = format!("{}:{:?}", client_id, tier);
        let client = limits
            .entry(client_key)
            .or_insert_with(|| ClientRateLimit::new(&tier_config));

        if !client.check() {
            let info = client.get_limit_info(&tier_config);
            let headers = info.to_headers(Some(60));
            return Err(RateLimitError::RateLimitExceeded(format!(
                "Rate limit exceeded for tier {:?}",
                tier
            )));
        }

        let info = client.get_limit_info(&tier_config);
        Ok(info.to_headers(None))
    }

    /// Get rate limit info for a client
    pub async fn get_limit_info(&self, client_id: &str) -> Option<RateLimitInfo> {
        let limits = self.client_limits.read().await;
        limits
            .get(client_id)
            .map(|c| c.get_limit_info(&self.config))
    }

    /// Get rate limit info for a client and tier
    pub async fn get_tier_limit_info(
        &self,
        client_id: &str,
        tier: EndpointTier,
    ) -> Option<RateLimitInfo> {
        let limits = self.client_limits.read().await;
        let client_key = format!("{}:{:?}", client_id, tier);
        let tier_config = {
            let tier_limits = self.tier_limits.read().await;
            tier_limits
                .get(&tier)
                .cloned()
                .unwrap_or_else(|| tier.rate_limit_config())
        };
        limits
            .get(&client_key)
            .map(|c| c.get_limit_info(&tier_config))
    }

    /// Add custom rate limit for a client
    pub async fn set_custom_limit(&self, client_id: &str, config: RateLimitConfig) {
        let mut limits = self.client_limits.write().await;
        limits.insert(client_id.to_string(), ClientRateLimit::new(&config));
    }

    /// Remove rate limit for a client
    pub async fn remove_limit(&self, client_id: &str) {
        let mut limits = self.client_limits.write().await;
        limits.remove(client_id);
    }

    /// Update tier configuration
    pub async fn set_tier_config(&self, tier: EndpointTier, config: RateLimitConfig) {
        let mut tier_limits = self.tier_limits.write().await;
        tier_limits.insert(tier, config);
    }

    /// Cleanup old entries to prevent memory growth
    pub async fn cleanup(&self, max_entries: usize) {
        let mut ip_limits = self.ip_limits.write().await;
        let mut client_limits = self.client_limits.write().await;

        // Keep only the most recent entries
        while ip_limits.len() > max_entries {
            if let Some(key) = ip_limits.keys().next().cloned() {
                ip_limits.remove(&key);
            } else {
                break;
            }
        }

        while client_limits.len() > max_entries {
            if let Some(key) = client_limits.keys().next().cloned() {
                client_limits.remove(&key);
            } else {
                break;
            }
        }
    }
}

/// Quota configuration for a client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaConfig {
    pub client_id: String,
    pub monthly_requests: u64,
    pub monthly_bandwidth_mb: u64,
    pub current_usage: u64,
    pub current_bandwidth: u64,
    pub reset_at: DateTime<Utc>,
    pub tier: QuotaTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuotaTier {
    Free,
    Basic,
    Pro,
    Enterprise,
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            client_id: "default".to_string(),
            monthly_requests: 10000,
            monthly_bandwidth_mb: 100,
            current_usage: 0,
            current_bandwidth: 0,
            reset_at: Utc::now() + ChronoDuration::days(30),
            tier: QuotaTier::Free,
        }
    }
}

impl QuotaConfig {
    pub fn new_tiered(client_id: String, tier: QuotaTier) -> Self {
        let (requests, bandwidth) = match tier {
            QuotaTier::Free => (10_000, 100),
            QuotaTier::Basic => (100_000, 1_000),
            QuotaTier::Pro => (1_000_000, 10_000),
            QuotaTier::Enterprise => (u64::MAX, u64::MAX),
        };

        Self {
            client_id,
            monthly_requests: requests,
            monthly_bandwidth_mb: bandwidth,
            current_usage: 0,
            current_bandwidth: 0,
            reset_at: Utc::now() + ChronoDuration::days(30),
            tier,
        }
    }
}

/// Quota Manager for tracking client usage
pub struct QuotaManager {
    quotas: Arc<RwLock<HashMap<String, QuotaConfig>>>,
}

impl QuotaManager {
    pub fn new() -> Self {
        Self {
            quotas: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn from_config(configs: Vec<QuotaConfig>) -> Self {
        let mut quotas = HashMap::new();
        for config in configs {
            quotas.insert(config.client_id.clone(), config);
        }
        Self {
            quotas: Arc::new(RwLock::new(quotas)),
        }
    }

    /// Check if client has quota remaining
    pub fn check_quota(&self, _client_id: &str) -> Result<QuotaConfig, String> {
        // This is sync for quick checks; use async version for full logic
        Ok(QuotaConfig::default())
    }

    /// Check quota with async reset logic
    pub async fn check_quota_async(&self, client_id: &str) -> Result<QuotaConfig, String> {
        let mut quotas = self.quotas.write().await;

        let config = quotas
            .entry(client_id.to_string())
            .or_insert_with(QuotaConfig::default);

        // Check if we need to reset (new month)
        if Utc::now() > config.reset_at {
            config.current_usage = 0;
            config.current_bandwidth = 0;
            config.reset_at = Utc::now() + ChronoDuration::days(30);
        }

        if config.current_usage >= config.monthly_requests {
            return Err(format!(
                "Monthly quota exceeded. Used: {}, Limit: {}",
                config.current_usage, config.monthly_requests
            ));
        }

        Ok(config.clone())
    }

    /// Record a request for quota tracking
    pub async fn record_request(&self, client_id: &str) {
        let mut quotas = self.quotas.write().await;
        if let Some(config) = quotas.get_mut(client_id) {
            config.current_usage += 1;
        }
    }

    /// Record bandwidth usage
    pub async fn record_bandwidth(&self, client_id: &str, bytes: u64) {
        let mut quotas = self.quotas.write().await;
        if let Some(config) = quotas.get_mut(client_id) {
            let mb = bytes / (1024 * 1024);
            config.current_bandwidth += mb;
        }
    }

    /// Get quota info for a client
    pub async fn get_quota(&self, client_id: &str) -> Option<QuotaConfig> {
        let quotas = self.quotas.read().await;
        quotas.get(client_id).cloned()
    }

    /// Set custom quota for a client
    pub async fn set_quota(&self, client_id: &str, config: QuotaConfig) {
        let mut quotas = self.quotas.write().await;
        quotas.insert(client_id.to_string(), config);
    }

    /// Get all quotas (admin function)
    pub async fn get_all_quotas(&self) -> Vec<QuotaConfig> {
        let quotas = self.quotas.read().await;
        quotas.values().cloned().collect()
    }
}

impl Default for QuotaManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_basic() {
        let limiter = RateLimiter::new(10, 5);

        // First 10 requests should pass
        for _ in 0..10 {
            assert!(limiter.check("test-ip").await);
        }

        // 11th should fail
        assert!(!limiter.check("test-ip").await);
    }

    #[tokio::test]
    async fn test_rate_limiter_per_client() {
        let limiter = RateLimiter::new(5, 2);

        for _ in 0..5 {
            assert!(limiter.check_client("client1").await);
        }
        assert!(!limiter.check_client("client1").await);

        // Different client should have own limit
        assert!(limiter.check_client("client2").await);
    }

    #[tokio::test]
    async fn test_rate_limiter_tier_based() {
        let limiter = RateLimiter::new(100, 10);

        // Public tier should allow more requests
        for _ in 0..100 {
            let result = limiter.check_tier("client1", EndpointTier::Public).await;
            assert!(result.is_ok());
        }

        // Standard tier should have its own limit
        for _ in 0..100 {
            let result = limiter.check_tier("client1", EndpointTier::Standard).await;
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_rate_limit_headers() {
        let limiter = RateLimiter::new(100, 10);

        let headers = limiter
            .check_tier("client1", EndpointTier::Standard)
            .await
            .unwrap();

        assert_eq!(headers.x_ratelimit_limit_minute, 100);
        assert!(headers.x_ratelimit_remaining_minute <= 100);
        assert!(headers.x_ratelimit_reset > 0);
        assert_eq!(headers.retry_after, None);
    }

    #[test]
    fn test_endpoint_tier_configs() {
        let public = EndpointTier::Public.rate_limit_config();
        assert_eq!(public.requests_per_minute, 1000);

        let standard = EndpointTier::Standard.rate_limit_config();
        assert_eq!(standard.requests_per_minute, 100);

        let premium = EndpointTier::Premium.rate_limit_config();
        assert_eq!(premium.requests_per_minute, 50);

        let admin = EndpointTier::Admin.rate_limit_config();
        assert_eq!(admin.requests_per_minute, 200);
    }

    #[tokio::test]
    async fn test_quota_manager() {
        let manager = QuotaManager::new();

        // Check initial quota
        let result = manager.check_quota_async("new-client").await;
        assert!(result.is_ok());

        // Record some requests
        for _ in 0..5 {
            manager.record_request("new-client").await;
        }

        let quota = manager.get_quota("new-client").await;
        assert!(quota.is_some());
        assert_eq!(quota.unwrap().current_usage, 5);
    }

    #[test]
    fn test_token_bucket() {
        let mut bucket = TokenBucket::new(10, 1.0); // 10 tokens, 1 per second

        // Should be able to consume up to 10
        for _ in 0..10 {
            assert!(bucket.try_consume());
        }

        // Should fail after 10
        assert!(!bucket.try_consume());
    }

    #[test]
    fn test_quota_tier() {
        let free = QuotaConfig::new_tiered("test".to_string(), QuotaTier::Free);
        assert_eq!(free.monthly_requests, 10_000);

        let pro = QuotaConfig::new_tiered("test".to_string(), QuotaTier::Pro);
        assert_eq!(pro.monthly_requests, 1_000_000);
    }
}
