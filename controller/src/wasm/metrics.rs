//! Prometheus metrics for the WASM module cache.
//
// The cache exposes hit/miss ratios and eviction counters through a single
// atomic snapshot structure that can be read from the /metrics http handler without
// taking the cache lock.

use std::sync::atomic::{AtomicU64, Ordering::Acquire, Ordering::Relaxed};

/// Atomic counters that can be updated from any thread concurrently.
#[derive(Default)]
pub struct CacheMetrics {
    hits: AtomicU64,
    misses: AtomicU64,
    insertions: AtomicU64,
    evictions: AtomicU64,
    invalidations: AtomicU64,
    expirations: AtomicU64,
    compilations: AtomicU64,
    compilation_nanos: AtomicU64,
    current_entries: AtomicU64,
    current_bytes: AtomicU64,
}

impl CacheMetrics {
    /// Record a cache hit.
    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Relaxed);
    }

    /// Record a cache miss.
    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Relaxed);
    }

    /// Record an insertion of a new module.
    pub fn record_insertion(&self) {
        self.insertions.fetch_add(1, Relaxed);
    }

    /// Record an eviction of a module.
    pub fn record_eviction(&self) {
        self.evictions.fetch_add(1, Relaxed);
    }

    /// Record an invalidation caused by a network state upgrade.
    pub fn record_invalidation(&self) {
        self.invalidations.fetch_add(1, Relaxed);
    }

    /// Record an expiration caused by a TUL elapsing.
    pub fn record_expiration(&self) {
        self.expirations.fetch_add(1, Relaxed);
    }

    /// Record a compilation and its duration.
    pub fn record_compilation(&self, nanos: u64) {
        self.compilations.fetch_add(1, Relaxed);
        self.compilation_nanos.fetch_add(nanos, Relaxed);
    }

    /// Update the gauges for current entries and bytes.
    pub fn set_gauges(&self, entries: u64, bytes: u64) {
        self.current_entries.store(entries, Relaxed);
        self.current_bytes.store(bytes, Relaxed);
    }

    /// Return a consistent snapshot of the counters.
    pub fn snapshot(&self) -> CacheMetricsSnapshot {
        CacheMetricsSnapshot {
            hits: self.hits.load(Acquire),
            misses: self.misses.load(Acquire),
            insertions: self.insertions.load(Acquire),
            evictions: self.evictions.load(Acquire),
            invalidations: self.invalidations.load(Acquire),
            expirations: self.expirations.load(Acquire),
            compilations: self.compilations.load(Acquire),
            compilation_nanos: self.compilation_nanos.load(Acquire),
            current_entries: self.current_entries.load(Acquire),
            current_bytes: self.current_bytes.load(Acquire),
        }
    }
}

/// A point-in-time snapshot of the cache metrics.
///
/// This is what the Prometheus exporter serializes into the openmetrics format.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CacheMetricsSnapshot {
    pub hits: u64,
    pub misses: u64,
    pub insertions: u64,
    pub evictions: u64,
    pub invalidations: u64,
    pub expirations: u64,
    pub compilations: u64,
    pub compilation_nanos: u64,
    pub current_entries: u64,
    pub current_bytes: u64,
}

impl CacheMetricsSnapshot {
    /// The cache hit ratio in the range [0.0, 1.0].
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// The average compilation latency in nanoseconds.
    pub fn average_compilation_nanos(&self) -> f64 {
        if self.compilations == 0 {
            0.0
        } else {
            self.compilation_nanos as f64 / self.compilations as f64
        }
    }

    /// Render the snapshot in the Prometheus text exposition format.
    pub fn to_prometheus<W: std::io::Write>(&self, mut out: W) -> std::io::Result<()> {
        writenl!(
            out,
            "# HELP stellar_wasm_cache_hits_total Total number of WASM cache hits.\n\
stellar_wasm_cache_hits_total {}\n\
stellar_wasm_cache_misses_total {}\n\
stellar_wasm_cache_insertions_total {}\n\
stellar_wasm_cache_evictions_total {}\n\
stellar_wasm_cache_invalidations_total {}\n\
stellar_wasm_cache_expirations_total {}\n\
stellar_wasm_compilations_total {}\n\
stellar_wasm_compilation_nanoseconds_total {}\n\
stellar_wasm_cache_entries {}\n\
stellar_wasm_cache_bytes {}\n\n",
            self.hits,
            self.misses,
            self.insertions,
            self.evictions,
            self.invalidations,
            self.expirations,
            self.compilations,
            self.compilation_nanos,
            self.current_entries,
            self.current_bytes,
        )
    }
}
