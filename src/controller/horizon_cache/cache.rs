//! Multi-tier cache: L1 memory, L2 Redis, L3 CDN.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lru::LruCache;
use serde::{Deserialize, Serialize};

/// Cache layer identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CacheLayer {
    L1Memory,
    L2Redis,
    L3Cdn,
}

/// Multi-tier Horizon cache configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HorizonCacheConfig {
    #[serde(default = "default_l1_capacity")]
    pub l1_capacity: usize,
    #[serde(default = "default_l1_ttl_secs")]
    pub l1_ttl_secs: u64,
    #[serde(default = "default_l2_enabled")]
    pub l2_redis_enabled: bool,
    #[serde(default = "default_redis_url")]
    pub l2_redis_url: String,
    #[serde(default = "default_l2_ttl_secs")]
    pub l2_ttl_secs: u64,
    #[serde(default = "default_l3_enabled")]
    pub l3_cdn_enabled: bool,
    #[serde(default = "default_cdn_prefix")]
    pub l3_cdn_prefix: String,
    #[serde(default = "default_l3_ttl_secs")]
    pub l3_ttl_secs: u64,
    #[serde(default = "default_compression")]
    pub compression_enabled: bool,
}

fn default_l1_capacity() -> usize {
    4096
}
fn default_l1_ttl_secs() -> u64 {
    60
}
fn default_l2_enabled() -> bool {
    true
}
fn default_redis_url() -> String {
    "redis://horizon-redis:6379".to_string()
}
fn default_l2_ttl_secs() -> u64 {
    300
}
fn default_l3_enabled() -> bool {
    false
}
fn default_cdn_prefix() -> String {
    "https://cdn.horizon.example.com/cache".to_string()
}
fn default_l3_ttl_secs() -> u64 {
    3600
}
fn default_compression() -> bool {
    true
}

impl Default for HorizonCacheConfig {
    fn default() -> Self {
        Self {
            l1_capacity: default_l1_capacity(),
            l1_ttl_secs: default_l1_ttl_secs(),
            l2_redis_enabled: default_l2_enabled(),
            l2_redis_url: default_redis_url(),
            l2_ttl_secs: default_l2_ttl_secs(),
            l3_cdn_enabled: default_l3_enabled(),
            l3_cdn_prefix: default_cdn_prefix(),
            l3_ttl_secs: default_l3_ttl_secs(),
            compression_enabled: default_compression(),
        }
    }
}

/// Cache entry with metadata.
#[derive(Debug, Clone)]
struct CacheEntry {
    data: Vec<u8>,
    created: Instant,
    ttl: Duration,
    layer: CacheLayer,
}

/// Cache hit/miss statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub l1_hits: u64,
    pub l1_misses: u64,
    pub l2_hits: u64,
    pub l2_misses: u64,
    pub l3_hits: u64,
    pub l3_misses: u64,
    pub total_puts: u64,
    pub total_evictions: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let hits = self.l1_hits + self.l2_hits + self.l3_hits;
        let total = hits + self.l1_misses;
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }
}

/// In-memory L2 simulation (production uses Redis client).
struct L2Store {
    data: Mutex<HashMap<String, CacheEntry>>,
}

impl L2Store {
    fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        let map = self.data.lock().unwrap();
        map.get(key).and_then(|e| {
            if e.created.elapsed() < e.ttl {
                Some(e.data.clone())
            } else {
                None
            }
        })
    }

    fn put(&self, key: &str, data: Vec<u8>, ttl: Duration) {
        self.data.lock().unwrap().insert(
            key.to_string(),
            CacheEntry {
                data,
                created: Instant::now(),
                ttl,
                layer: CacheLayer::L2Redis,
            },
        );
    }

    fn evict(&self, key: &str) {
        self.data.lock().unwrap().remove(key);
    }
}

/// L3 CDN simulation (production uses CDN API).
struct L3Store {
    prefix: String,
    data: Mutex<HashMap<String, CacheEntry>>,
}

impl L3Store {
    fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            data: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        let cdn_key = format!("{}/{}", self.prefix, key);
        let map = self.data.lock().unwrap();
        map.get(&cdn_key).and_then(|e| {
            if e.created.elapsed() < e.ttl {
                Some(e.data.clone())
            } else {
                None
            }
        })
    }

    fn put(&self, key: &str, data: Vec<u8>, ttl: Duration) {
        let cdn_key = format!("{}/{}", self.prefix, key);
        self.data.lock().unwrap().insert(
            cdn_key,
            CacheEntry {
                data,
                created: Instant::now(),
                ttl,
                layer: CacheLayer::L3Cdn,
            },
        );
    }

    fn evict(&self, key: &str) {
        let cdn_key = format!("{}/{}", self.prefix, key);
        self.data.lock().unwrap().remove(&cdn_key);
    }
}

/// Multi-tier Horizon query cache.
pub struct HorizonCache {
    l1: Mutex<LruCache<String, CacheEntry>>,
    l2: L2Store,
    l3: L3Store,
    config: HorizonCacheConfig,
    stats: Mutex<CacheStats>,
}

impl HorizonCache {
    pub fn new(config: HorizonCacheConfig) -> Self {
        let capacity = NonZeroUsize::new(config.l1_capacity.max(1)).unwrap();
        Self {
            l1: Mutex::new(LruCache::new(capacity)),
            l2: L2Store::new(),
            l3: L3Store::new(&config.l3_cdn_prefix),
            config,
            stats: Mutex::new(CacheStats::default()),
        }
    }

    /// Look up a cached query response. Checks L1 → L2 → L3.
    pub fn get(&self, key: &str) -> Option<(Vec<u8>, CacheLayer)> {
        // L1
        {
            let mut l1 = self.l1.lock().unwrap();
            if let Some(entry) = l1.get(key) {
                if entry.created.elapsed() < entry.ttl {
                    self.stats.lock().unwrap().l1_hits += 1;
                    return Some((entry.data.clone(), CacheLayer::L1Memory));
                }
            }
        }
        self.stats.lock().unwrap().l1_misses += 1;

        // L2
        if self.config.l2_redis_enabled {
            if let Some(data) = self.l2.get(key) {
                self.stats.lock().unwrap().l2_hits += 1;
                // Promote to L1
                self.put_l1(key, data.clone(), Duration::from_secs(self.config.l1_ttl_secs));
                return Some((data, CacheLayer::L2Redis));
            }
            self.stats.lock().unwrap().l2_misses += 1;
        }

        // L3
        if self.config.l3_cdn_enabled {
            if let Some(data) = self.l3.get(key) {
                self.stats.lock().unwrap().l3_hits += 1;
                self.put_l1(key, data.clone(), Duration::from_secs(self.config.l1_ttl_secs));
                if self.config.l2_redis_enabled {
                    self.l2.put(
                        key,
                        data.clone(),
                        Duration::from_secs(self.config.l2_ttl_secs),
                    );
                }
                return Some((data, CacheLayer::L3Cdn));
            }
            self.stats.lock().unwrap().l3_misses += 1;
        }

        None
    }

    /// Store a query response in all enabled cache layers.
    pub fn put(&self, key: &str, data: Vec<u8>) {
        self.put_l1(
            key,
            data.clone(),
            Duration::from_secs(self.config.l1_ttl_secs),
        );
        if self.config.l2_redis_enabled {
            self.l2
                .put(key, data.clone(), Duration::from_secs(self.config.l2_ttl_secs));
        }
        if self.config.l3_cdn_enabled {
            self.l3
                .put(key, data, Duration::from_secs(self.config.l3_ttl_secs));
        }
        self.stats.lock().unwrap().total_puts += 1;
    }

    fn put_l1(&self, key: &str, data: Vec<u8>, ttl: Duration) {
        self.l1.lock().unwrap().put(
            key.to_string(),
            CacheEntry {
                data,
                created: Instant::now(),
                ttl,
                layer: CacheLayer::L1Memory,
            },
        );
    }

    /// Evict a key from all cache layers.
    pub fn evict(&self, key: &str) {
        self.l1.lock().unwrap().pop(key);
        if self.config.l2_redis_enabled {
            self.l2.evict(key);
        }
        if self.config.l3_cdn_enabled {
            self.l3.evict(key);
        }
        self.stats.lock().unwrap().total_evictions += 1;
    }

    /// Evict all keys matching a ledger sequence prefix.
    pub fn evict_by_prefix(&self, prefix: &str) {
        let keys: Vec<String> = self
            .l1
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            self.evict(&key);
        }
    }

    pub fn stats(&self) -> CacheStats {
        self.stats.lock().unwrap().clone()
    }

    pub fn config(&self) -> &HorizonCacheConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l1_hit_after_put() {
        let cache = HorizonCache::new(HorizonCacheConfig {
            l2_redis_enabled: false,
            l3_cdn_enabled: false,
            ..Default::default()
        });
        cache.put("accounts/GABC", b"response".to_vec());
        let (data, layer) = cache.get("accounts/GABC").unwrap();
        assert_eq!(data, b"response");
        assert_eq!(layer, CacheLayer::L1Memory);
    }

    #[test]
    fn l2_promotion_to_l1() {
        let cache = HorizonCache::new(HorizonCacheConfig {
            l1_capacity: 1,
            l2_redis_enabled: true,
            l3_cdn_enabled: false,
            ..Default::default()
        });
        cache.put("key-a", b"aaa".to_vec());
        cache.put("key-b", b"bbb".to_vec());
        // key-a evicted from L1 but still in L2
        let (data, layer) = cache.get("key-a").unwrap();
        assert_eq!(data, b"aaa");
        assert_eq!(layer, CacheLayer::L2Redis);
    }

    #[test]
    fn evict_by_prefix() {
        let cache = HorizonCache::new(HorizonCacheConfig::default());
        cache.put("ledger:100:accounts", b"a".to_vec());
        cache.put("ledger:100:payments", b"p".to_vec());
        cache.put("ledger:200:accounts", b"b".to_vec());
        cache.evict_by_prefix("ledger:100:");
        assert!(cache.get("ledger:100:accounts").is_none());
        assert!(cache.get("ledger:200:accounts").is_some());
    }

    #[test]
    fn bench_horizon_cache_speedup() {
        use std::time::{Duration, Instant};

        let cache = HorizonCache::new(HorizonCacheConfig {
            l2_redis_enabled: false,
            l3_cdn_enabled: false,
            ..Default::default()
        });

        let simulate_db = || -> Vec<u8> {
            std::thread::sleep(Duration::from_millis(5));
            vec![0xAB; 512]
        };

        let key = "accounts:GABC123";
        const ITERATIONS: u32 = 20;

        let cold_start = Instant::now();
        for _ in 0..ITERATIONS {
            if cache.get(key).is_none() {
                cache.put(key, simulate_db());
            }
        }
        let cold_elapsed = cold_start.elapsed();

        let warm_start = Instant::now();
        for _ in 0..ITERATIONS {
            let _ = cache.get(key);
        }
        let warm_elapsed = warm_start.elapsed();

        let speedup = cold_elapsed.as_secs_f64() / warm_elapsed.as_secs_f64();
        eprintln!(
            "[bench_horizon_cache] cold={cold_elapsed:?} warm={warm_elapsed:?} speedup={speedup:.0}x"
        );
        assert!(speedup > 2.0, "expected >2x speedup, got {speedup:.1}x");
    }
}
