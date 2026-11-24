//! Bounded LRU + TTL cache engine with fail-open semantics.
//!
//! This is the core caching logic for issue #4. It is deliberately kept
//! free of any Wasm/host-boundary code (no `extern "C"`, no serde) so it
//! can be unit tested with plain `cargo test` on the host target — the
//! thin Wasm entry point in `lib.rs` is a separate, much smaller layer on
//! top of this.
//!
//! ## Design notes relevant to "fail-open"
//!
//! - The cache never reads a system clock itself. Every call takes an
//!   explicit `now` (a caller-supplied Unix timestamp, or any monotonically
//!   non-decreasing counter). This keeps the engine deterministic and
//!   testable, and avoids relying on WASI clock imports being available in
//!   whatever host environment eventually embeds this plugin.
//! - Capacity is capped hard at [`MAX_CAPACITY`] regardless of what a
//!   caller (eventually: a ConfigMap-driven `PluginConfig`) requests, so a
//!   misconfigured huge value cannot grow the cache's memory footprint
//!   without bound inside the sandbox's limited linear memory.
//! - A `capacity` of `0` puts the cache into "disabled" mode: `get` always
//!   reports a miss and `put` is a no-op. This is the cache's own
//!   lowest-level fail-open switch, used by the `lib.rs` entry point when
//!   it decides (or is told, e.g. via config) to bypass caching entirely.

use std::collections::HashMap;
use std::hash::Hash;

/// Hard ceiling on cache capacity, independent of whatever `capacity` a
/// caller requests. Chosen conservatively relative to the plugin runtime's
/// default `max_memory_bytes` (16MB, see `src/webhook/types.rs`), leaving
/// the overwhelming majority of that budget for the Wasm/WASI runtime
/// itself rather than this crate's own bookkeeping.
pub const MAX_CAPACITY: usize = 10_000;

/// Cache configuration. Mirrors the two knobs the issue calls out as
/// needing to be ConfigMap-configurable: TTL and size.
#[derive(Debug, Clone, Copy)]
pub struct CacheConfig {
    /// Maximum number of entries. `0` disables the cache (always miss).
    /// Clamped to [`MAX_CAPACITY`].
    pub capacity: usize,
    /// Time-to-live for an entry, in seconds. `0` means entries never
    /// expire on their own (they can still be evicted for capacity).
    pub ttl_seconds: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            capacity: 1024,
            ttl_seconds: 30,
        }
    }
}

struct Entry<V> {
    value: V,
    inserted_at: u64,
}

/// A bounded, TTL-aware LRU cache.
///
/// Recency is tracked with an explicit `Vec<K>` ordered most-recently-used
/// first. This is O(n) per touch rather than O(1), which is a deliberate
/// simplicity-over-throughput tradeoff appropriate for this slice of the
/// issue (see the crate README for what's explicitly out of scope) — a
/// production version handling real RPC load would swap this for an
/// intrusive doubly-linked-list-plus-hashmap ("linked hash map") structure
/// without changing this struct's public API.
pub struct LruTtlCache<K, V> {
    capacity: usize,
    ttl_seconds: u64,
    entries: HashMap<K, Entry<V>>,
    recency: Vec<K>,
}

/// Outcome of a [`LruTtlCache::get`] call.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup<V> {
    Hit(V),
    Miss,
}

impl<K, V> LruTtlCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    pub fn new(config: CacheConfig) -> Self {
        let capacity = config.capacity.min(MAX_CAPACITY);
        Self {
            capacity,
            ttl_seconds: config.ttl_seconds,
            entries: HashMap::with_capacity(capacity),
            recency: Vec::with_capacity(capacity),
        }
    }

    /// Whether this cache is in disabled ("always fail open") mode.
    pub fn is_disabled(&self) -> bool {
        self.capacity == 0
    }

    // Not called by `lib.rs` today (only by this module's own tests) —
    // kept public as the natural hook a future metrics/observability
    // integration would use to report current cache occupancy.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up `key` as of time `now`. Expired entries are evicted lazily
    /// on lookup rather than proactively, so a disabled or idle cache does
    /// no background work.
    pub fn get(&mut self, key: &K, now: u64) -> Lookup<V> {
        if self.is_disabled() {
            return Lookup::Miss;
        }

        let expired = match self.entries.get(key) {

            None => return Lookup::Miss,
        };

        if expired {
            self.remove(key);
            return Lookup::Miss;
        }

        self.touch(key);
        Lookup::Hit(self.entries.get(key).expect("just touched").value.clone())
    }

    /// Insert or refresh `key` -> `value` as of time `now`. A no-op when
    /// the cache is disabled. Evicts the least-recently-used entry first
    /// if inserting a new key would exceed capacity.
    pub fn put(&mut self, key: K, value: V, now: u64) {
        if self.is_disabled() {
            return;
        }

        if self.entries.contains_key(&key) {

            self.touch(&key);
            return;
        }

        if self.entries.len() >= self.capacity {
            self.evict_lru();
        }


        self.recency.insert(0, key);
    }

    fn touch(&mut self, key: &K) {
        if let Some(pos) = self.recency.iter().position(|k| k == key) {
            let k = self.recency.remove(pos);
            self.recency.insert(0, k);
        }
    }

    fn remove(&mut self, key: &K) {
        self.entries.remove(key);
        if let Some(pos) = self.recency.iter().position(|k| k == key) {
            self.recency.remove(pos);
        }
    }

    fn evict_lru(&mut self) {
        if let Some(lru_key) = self.recency.pop() {
            self.entries.remove(&lru_key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(capacity: usize, ttl_seconds: u64) -> LruTtlCache<String, String> {

    }

    #[test]
    fn miss_on_empty_cache() {
        let mut c = cache(2, 60);
        assert_eq!(c.get(&"a".to_string(), 0), Lookup::Miss);
    }

    #[test]
    fn put_then_get_hits() {
        let mut c = cache(2, 60);
        c.put("a".into(), "value-a".into(), 0);
        assert_eq!(c.get(&"a".to_string(), 1), Lookup::Hit("value-a".into()));
    }

    #[test]
    fn lru_eviction_removes_least_recently_used() {
        let mut c = cache(2, 0);
        c.put("a".into(), "1".into(), 0);
        c.put("b".into(), "2".into(), 0);
        // Touch "a" so "b" becomes the least-recently-used entry.
        assert_eq!(c.get(&"a".to_string(), 0), Lookup::Hit("1".into()));
        c.put("c".into(), "3".into(), 0);


        assert_eq!(c.get(&"a".to_string(), 0), Lookup::Hit("1".into()));
        assert_eq!(c.get(&"c".to_string(), 0), Lookup::Hit("3".into()));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn ttl_expiry_evicts_on_next_lookup() {
        let mut c = cache(10, 30);
        c.put("a".into(), "1".into(), 100);

    }

    #[test]
    fn zero_ttl_means_entries_never_expire_on_their_own() {
        let mut c = cache(10, 0);
        c.put("a".into(), "1".into(), 0);
        assert_eq!(c.get(&"a".to_string(), 1_000_000), Lookup::Hit("1".into()));
    }

    #[test]
    fn zero_capacity_disables_the_cache() {
        let mut c = cache(0, 60);
        assert!(c.is_disabled());
        c.put("a".into(), "1".into(), 0);
        assert_eq!(c.get(&"a".to_string(), 0), Lookup::Miss);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn capacity_is_clamped_to_max_capacity() {
        let c: LruTtlCache<String, String> = LruTtlCache::new(CacheConfig {
            capacity: MAX_CAPACITY * 100,
            ttl_seconds: 60,
        });
        assert_eq!(c.capacity, MAX_CAPACITY);
    }

    #[test]
    fn re_putting_an_existing_key_refreshes_its_ttl_and_value() {
        let mut c = cache(10, 30);
        c.put("a".into(), "1".into(), 0);
        c.put("a".into(), "2".into(), 20);
        // At t=45 the *original* insert (t=0) would have expired, but the
        // refresh at t=20 should keep it alive until t=50.
        assert_eq!(c.get(&"a".to_string(), 45), Lookup::Hit("2".into()));

    }
}
