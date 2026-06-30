//! L1 In-Memory Cache with LRU eviction

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};
use tracing::debug;

use crate::error::Result;

/// Cache configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CacheConfig {
    pub max_entries: usize,
    pub default_ttl_secs: u64,
    pub enable_compression: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            default_ttl_secs: 3600,
            enable_compression: false,
        }
    }
}

/// Cache entry
#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub value: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub access_count: u64,
    pub last_accessed_at: DateTime<Utc>,
}

impl CacheEntry {
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            Utc::now() > expires_at
        } else {
            false
        }
    }
}

/// L1 In-Memory Cache
pub struct Cache {
    config: CacheConfig,
    entries: tokio::sync::RwLock<HashMap<String, CacheEntry>>,
}

impl Cache {
    pub async fn new(config: CacheConfig) -> Result<Self> {
        debug!("Initializing L1 Cache");
        Ok(Self {
            config,
            entries: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let mut entries = self.entries.write().await;
        
        if let Some(entry) = entries.get_mut(key) {
            if entry.is_expired() {
                entries.remove(key);
                return Ok(None);
            }
            
            entry.access_count += 1;
            entry.last_accessed_at = Utc::now();
            return Ok(Some(entry.value.clone()));
        }
        
        Ok(None)
    }

    pub async fn set(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<()> {
        let mut entries = self.entries.write().await;

        if entries.len() >= self.config.max_entries {
            // Evict LRU entry
            if let Some(lru_key) = entries
                .iter()
                .min_by_key(|(_, e)| e.last_accessed_at)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&lru_key);
            }
        }

        let now = Utc::now();
        let expires_at = ttl.map(|t| now + t);

        entries.insert(
            key.to_string(),
            CacheEntry {
                value,
                created_at: now,
                expires_at,
                access_count: 0,
                last_accessed_at: now,
            },
        );

        Ok(())
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        let mut entries = self.entries.write().await;
        entries.remove(key);
        Ok(())
    }

    pub async fn clear(&self) -> Result<()> {
        let mut entries = self.entries.write().await;
        entries.clear();
        Ok(())
    }

    pub async fn get_statistics(&self) -> Result<CacheStatistics> {
        let entries = self.entries.read().await;
        
        let total_entries = entries.len();
        let expired_entries = entries.values().filter(|e| e.is_expired()).count();
        let total_accesses: u64 = entries.values().map(|e| e.access_count).sum();

        Ok(CacheStatistics {
            entries: total_entries,
            expired_entries,
            total_accesses,
            timestamp: Utc::now(),
        })
    }
}

/// Cache statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheStatistics {
    pub entries: usize,
    pub expired_entries: usize,
    pub total_accesses: u64,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_set_get() {
        let config = CacheConfig::default();
        let cache = Cache::new(config).await.unwrap();

        cache.set("key1", b"value1".to_vec(), None).await.unwrap();
        let value = cache.get("key1").await.unwrap();

        assert!(value.is_some());
        assert_eq!(value.unwrap(), b"value1".to_vec());
    }
}
