//! Advanced Cache Management with Distributed Caching and Invalidation
//!
//! Provides multi-tier caching with distributed cache support, intelligent invalidation,
//! and cache warming strategies.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  Advanced Cache Management System                        │
//! ├─────────────────────────────────────────────────────────┤
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐   │
//! │  │ L1 Cache     │  │ L2 Cache     │  │ L3 Cache     │   │
//! │  │ (In-Memory)  │  │ (Redis)      │  │ (CDN/Remote) │   │
//! │  └──────────────┘  └──────────────┘  └──────────────┘   │
//! │         │                 │                 │             │
//! │         └─────────────────┴─────────────────┘             │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Cache Invalidation      │                      │
//! │         │ (Event-driven)          │                      │
//! │         └────────────┬────────────┘                      │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Cache Warming           │                      │
//! │         │ (Predictive)            │                      │
//! │         └────────────┬────────────┘                      │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Cache Metrics           │                      │
//! │         │ (Hit/Miss/Eviction)     │                      │
//! │         └────────────────────────┘                      │
//! └─────────────────────────────────────────────────────────┘
//! ```

pub mod cache;
pub mod distributed;
pub mod invalidation;
pub mod warming;
pub mod metrics;

pub use cache::{Cache, CacheConfig, CacheEntry};
pub use distributed::{DistributedCache, DistributedCacheConfig};
pub use invalidation::{CacheInvalidator, InvalidationStrategy};
pub use warming::{CacheWarmer, WarmingStrategy};
pub use metrics::{CacheMetrics, CacheStatistics};

use std::sync::Arc;
use tracing::info;

/// Advanced Cache Management System Configuration
#[derive(Clone, Debug)]
pub struct CacheSystemConfig {
    pub l1_config: cache::CacheConfig,
    pub l2_config: distributed::DistributedCacheConfig,
    pub invalidation_config: invalidation::InvalidationConfig,
    pub warming_config: warming::WarmingConfig,
}

impl Default for CacheSystemConfig {
    fn default() -> Self {
        Self {
            l1_config: Default::default(),
            l2_config: Default::default(),
            invalidation_config: Default::default(),
            warming_config: Default::default(),
        }
    }
}

/// Advanced Cache Management System
pub struct CacheManagementSystem {
    /// L1 in-memory cache
    l1_cache: Arc<Cache>,
    /// L2 distributed cache
    l2_cache: Arc<DistributedCache>,
    /// Cache invalidator
    invalidator: Arc<CacheInvalidator>,
    /// Cache warmer
    warmer: Arc<CacheWarmer>,
    /// Cache metrics
    metrics: Arc<CacheMetrics>,
}

impl CacheManagementSystem {
    /// Create a new cache management system
    pub async fn new(config: CacheSystemConfig) -> crate::error::Result<Self> {
        info!("Initializing Advanced Cache Management System");

        let l1_cache = Arc::new(Cache::new(config.l1_config).await?);
        let l2_cache = Arc::new(DistributedCache::new(config.l2_config).await?);
        let invalidator = Arc::new(CacheInvalidator::new(config.invalidation_config).await?);
        let warmer = Arc::new(CacheWarmer::new(config.warming_config).await?);
        let metrics = Arc::new(CacheMetrics::new());

        Ok(Self {
            l1_cache,
            l2_cache,
            invalidator,
            warmer,
            metrics,
        })
    }

    /// Get value from cache (multi-tier lookup)
    pub async fn get(&self, key: &str) -> crate::error::Result<Option<Vec<u8>>> {
        // Try L1 first
        if let Some(value) = self.l1_cache.get(key).await? {
            self.metrics.record_hit("l1").await;
            return Ok(Some(value));
        }

        self.metrics.record_miss("l1").await;

        // Try L2
        if let Some(value) = self.l2_cache.get(key).await? {
            self.metrics.record_hit("l2").await;
            // Promote to L1
            let _ = self.l1_cache.set(key, value.clone(), None).await;
            return Ok(Some(value));
        }

        self.metrics.record_miss("l2").await;
        Ok(None)
    }

    /// Set value in cache (multi-tier)
    pub async fn set(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<std::time::Duration>,
    ) -> crate::error::Result<()> {
        // Set in both L1 and L2
        self.l1_cache.set(key, value.clone(), ttl).await?;
        self.l2_cache.set(key, value, ttl).await?;
        Ok(())
    }

    /// Invalidate cache entry
    pub async fn invalidate(&self, key: &str) -> crate::error::Result<()> {
        self.l1_cache.delete(key).await?;
        self.l2_cache.delete(key).await?;
        self.metrics.record_invalidation().await;
        Ok(())
    }

    /// Invalidate by pattern
    pub async fn invalidate_pattern(&self, pattern: &str) -> crate::error::Result<usize> {
        let count = self.invalidator.invalidate_pattern(pattern).await?;
        Ok(count)
    }

    /// Warm cache with predictive entries
    pub async fn warm_cache(&self) -> crate::error::Result<usize> {
        let count = self.warmer.warm_cache().await?;
        Ok(count)
    }

    /// Get cache statistics
    pub async fn get_statistics(&self) -> crate::error::Result<CacheSystemStatistics> {
        let l1_stats = self.l1_cache.get_statistics().await?;
        let l2_stats = self.l2_cache.get_statistics().await?;
        let metrics = self.metrics.get_statistics().await?;

        Ok(CacheSystemStatistics {
            l1_statistics: l1_stats,
            l2_statistics: l2_stats,
            metrics,
        })
    }

    /// Get L1 cache
    pub fn l1_cache(&self) -> Arc<Cache> {
        self.l1_cache.clone()
    }

    /// Get L2 cache
    pub fn l2_cache(&self) -> Arc<DistributedCache> {
        self.l2_cache.clone()
    }

    /// Get invalidator
    pub fn invalidator(&self) -> Arc<CacheInvalidator> {
        self.invalidator.clone()
    }

    /// Get warmer
    pub fn warmer(&self) -> Arc<CacheWarmer> {
        self.warmer.clone()
    }

    /// Get metrics
    pub fn metrics(&self) -> Arc<CacheMetrics> {
        self.metrics.clone()
    }
}

/// Cache system statistics
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CacheSystemStatistics {
    pub l1_statistics: cache::CacheStatistics,
    pub l2_statistics: distributed::DistributedCacheStatistics,
    pub metrics: metrics::MetricsStatistics,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_system_creation() {
        let config = CacheSystemConfig::default();
        let system = CacheManagementSystem::new(config).await.unwrap();
        
        let stats = system.get_statistics().await.unwrap();
        assert_eq!(stats.l1_statistics.entries, 0);
    }
}
