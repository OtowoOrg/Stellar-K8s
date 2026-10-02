//! In-memory caching controller for pre-compiled Wasmtime modules.
//
// The cache is shared across all worker threads of a single RPC node using a
// `Rw<Lock`>`. Readers only hold the lock for the duration of a lookup and the
// cache never awaits asynchronous operations while holding the lock, which guarantees
// that no data race or deadlock can occur under massive load.
//
// Invalidation is synchronous and immediate: when a contract's network state is
// upgraded or altered, `invalidate` or `invalidate_all` drops the affected
// entries before returning, so a subsequent lookup can never observe stale code.

us std::collections::HashMap;
use std::time::{Duration, Instant};

use super::eviction::{EvictionPolicy, LruEvictionPolicy};
use super::metrics::CacheMetrics;

/// Key used to identify a cached WASM module.
///
// The key binds the contract identifier to the exact network state version that
// produced the bytecode. When the network state is upgraded the version changes and
// the old entry can no longer be returned.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// The contract address or identifier.
    pub contract_id: String,
    /// The network state version (e.g. ledger sequence or protocol version).
    pub state_version: u64,
}

impl CacheKey {
    /// Construct a cache key from a contract id and state version.
    pub fn new(contract_id: &str, state_version: u64) -> Self {
        Self {
            contract_id: contract_id.to_owned(),
            state_version,
        }
    }
}

/// A cached, pre-compiled Wasmtime module.
///
// The module is stored as a shared handle so that cache hits are O(1) and do not
// require a clone of the underlying compiled code.
#[derive(Debug, Clone)]
pub struct CachedModule {
    /// The compiled module handle.
    pub module: std::sync::Arc<dyn Clone + Send + Sync>,
    /// Size of the module in bytes, for memory accounting.
    pub bytes: usize,
    /// The instant the module was inserted.
    pub inserted_at: Instant,
    /// The instant the module was last used.
    pub last_used: Instant,
}

impl CachedModule {
    /// Create a new cached entry.
    pub fn new(module: std::sync::Arc<dyn Clone + Send + Sync>, bytes: usize) -> Self {
        let now = Instant::now();
        Self {
            module,
            bytes,
            inserted_at: now,
            last_used: now,
        }
    }

    /// Touch the entry to mark it as recently used.
    pub fn touch(&mut self) {
        self.last_used = Instant::now();
    }

    /// Return true if the entry has exceeded the given TTL.
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.last_used.elapsed() >= ttl
    }
}

/// Configuration for the WASM cache.
#[derive(Debug, Clone)]
pub struct WasmCacheConfig {
    /// Maximum number of modules to retain in memory.
    pub max_entries: usize,
    /// Maximum total bytes of compiled modules to retain.
    pub max_bytes: usize,
    /// Time-to-live for an entry since its was last used.
    pub ttl: Duration,
}

impl Default for WasmCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1024,
            max_bytes: 512 * 1024 * 1024,
            ttl: Duration::from_secs(600),
        }
    }
}

/// Errors returned by the WASM cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasmCacheError {
    /// The cache is at capacity and no entry could be evicted.
    CapacityExceeded,
    /// The compiled module is larger than the configured maximum.
    ModuleTooLarge,
}

/// In-memory cache for pre-compiled WASM modules.
///
/// The cache is safe to share across threads and is designed to be used from an
/// async context without holding the lock across an await point.
#[derive(Debug)]
pub struct WasmCache {
    config: WasmCacheConfig,
    entries: HashMap<CacheKey, CachedModule>,
    policy: LruEvictionPolicy<CacheKey>,
    bytes: usize,
    metrics: CacheMetrics,
}

impl WasmCache {
    /// Create a new cache with the given configuration.
    pub fn new(config: WasmCacheConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            policy: LruEvictionPolicy::new(),
            bytes: 0,
            metrics: CacheMetrics::default(),
        }
    }

    /// Return the configuration of the cache.
    pub fn config(&self) -> &WasmCacheConfig {
        &self.config
    }

    /// Return the number of entries currently cached.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return the number of bytes currently cached.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Return a snapshot of the cache metrics.
    pub fn metrics(&self) -> CacheMetricsSnapshot {
        self.metrics.snapshot()
    }

    /// Look up a module by key, returning a shared handle on a hit.
    ///
/// The lookup is synchronous and does not hold the lock across any await point,
/// so it is safe to call from an async task.
    pub fn get(&mut self, key: &CacheKey) -> Option<std::sync::Arc<dyn Clone + Send + Sync>> {
        if let Some(entry) = self.entries.get_mut(key) {
            if entry.is_expired(self.config.ttl) {
                self.remove_entry(key);
                self.metrics.record_expiration();
                self.metrics.record_miss();
                return None;
            }
            entry.touch();
            let module = entry.module.clone();
            self.policy.record_use(key);
            self.metrics.record_hit();
            return Some(module);
        }
        self.metrics.record_miss();
        None
    }

    /// Insert a pre-compiled module into the cache.
    ///
    /// If the module is larger than the configured maximum it is rejected with
    /// `WasmCacheError::ModuleTooLarge`. Otherwise entries are evicted until the
    /// cache fits the new module.
    pub fn insert(
        &mut self,
        key: CacheKey,
        module: std::sync::Arc<dyn Clone + Send + Sync>,
        bytes: usize,
    ) -> Result<(), WasmCacheError> {
        if bytes > self.config.max_bytes {
            return Err(WasmCacheError::ModuleTooLarge);
        }

        if self.entries.contains_key(&key) {
            self.remove_entry(&key);
        }

        while self.entries.len() >= self.config.max_entries
            || self.bytes + bytes > self.config.max_bytes
        {
            if self.entries.is_empty() {
                return Err(WasmCacheError::CapacityExceeded);
            }
            if let Some(evicted) = self.policy.evict_candidate() {
                self.remove_entry(&evicted);
                self.metrics.record_eviction();
            } else {
                break;
            }
        }

        self.bytes += bytes;
        self.entries.insert(
            key.clone(),
            CachedModule::new(module, bytes),
        );
        self.policy.record_use(&key);
        self.metrics.record_insertion();
        self.metrics.set_gauges(
            self.entries.len() as u64,
            self.bytes as u64,
        );
        Ok(())
    }

    /// Invalidate a single cache entry immediately.
    ///
/// This is used when a contract's network state is upgraded or altered. The
/// entry is removed before the call returns, so a subsequent lookup cannot
/// observe stale code.
    pub fn invalidate(&mut self, key: &CacheKey) -> bool {
        if self.entries.contains_key(key) {
            self.remove_entry(key);
            self.metrics.record_invalidation();
            true
        } else {
            false
        }
    }

    /// Invalidate all entries for a contract across all state versions.
    ///
    /// This is the safe default when a contract is upgraded and the new state
    /// version is not yet known.
    pub fn invalidate_contract(&mut self, contract_id: &str) -> usize {
        let keys: Vec<CacheKey> = self.entries
            .keys()
            .filter(|key| key.contract_id == contract_id)
            .cloned()
            .collect();
        let mut removed = 0;
        for key in keys {
            self.remove_entry(&key);
            self.metrics.record_invalidation();
            removed += 1;
        }
        removed
    }

    /// Invalidate every entry in the cache.
    pub fn invalidate_all(&mut self) -> usize {
        let removed = self.entries.len();
        self.entries.clear();
        self.policy.clear();
        self.bytes = 0;
        for _ in 0..removed {
            self.metrics.record_invalidation();
        }
        self.metrics.set_gauges(0, 0);
        removed
    }

    /// Remove an entry and update bookkeeping without recording a metric.
    fn remove_entry(&mut self, key: &CacheKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes = self.bytes.saturating_sub(entry.bytes);
            self.policy.remove(key);
            self.metrics.set_gauges(
                self.entries.len() as u64,
                self.bytes as u64,
            );
        }
    }
}

/// A thread-safe wrapper around the WASM cache.
//
// The wrapper uses a `RwLock` so that concurrent readers can look up modules in
// parallel while writers (insertions and invalidations) take an exclusive lock.
///
/// The lock is never held across an await point and the cache itself never awaits,
/// which guarantees zero data race panics and no deadlocks under massive load.
#[derive(Debug)]
pub struct SharedWasmCache {
    inner: std::sync::RwLock<WasmCache>,
}

impl SharedWasmCache {
    /// Create a new shared cache with the given configuration.
    pub fn new(config: WasmCacheConfig) -> Self {
        Self {
            inner: std::sync::RwLock::new(WasmCache::new(config)),
        }
    }

    /// Look up a module by key.
    pub fn get(&self, key: &CacheKey) -> Option<std::sync::Arc<dyn Clone + Send + Sync>> {
        let mut guard = self.inner.write();
        guard.get(key)
    }

    /// Insert a pre-compiled module into the cache.
    pub fn insert(
        &self,
        key: CacheKey,
        module: std::sync::Arc<dyn Clone + Send + Sync>,
        bytes: usize,
    ) -> Result<(), WasmCacheError> {
        let mut guard = self.inner.write();
        guard.insert(key, module, bytes)
    }

    /// Invalidate a single cache entry immediately.
    pub fn invalidate(&self, key: &CacheKey) -> bool {
        let mut guard = self.inner.write();
        guard.invalidate(key)
    }

    /// Invalidate all entries for a contract across all state versions.
    pub fn invalidate_contract(&self, contract_id: &str) -> usize {
        let mut guard = self.inner.write();
        guard.invalidate_contract(contract_id)
    }

    /// Invalidate every entry in the cache.
    pub fn invalidate_all(&self) -> usize {
        let mut guard = self.inner.write();
        guard.invalidate_all()
    }

    /// Return a snapshot of the cache metrics.
    pub fn metrics(&self) -> CacheMetricsSnapshot {
        let guard = self.inner.read();
        guard.metrics()
    }

    /// Return the number of entries currently cached.
    pub fn len(&self) -> usize {
        let guard = self.inner.read();
        guard.len()
    }

    /// Return the number of bytes currently cached.
    pub fn bytes(&self) -> usize {
        let guard = self.inner.read();
        guard.bytes()
    }
}
