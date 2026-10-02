//! WebAssembly caching layer for the Stellar operator.
//
// This module provides an in-memory, concurrency-safe LRU cache for pre-compiled
// Wasmtime modules. It is designed to be shared across all worker threads of a
// single RPC node and to be invalidated instantaneously when a contract's underlying
// network state is upgraded or altered.

pub mod metrics;
pub mod eviction;
pub mod cache_layer;

pub use cache_layer::{CacheKey, CachedModule, WasmCache, WasmCacheConfig, WasmCacheError};
pub use eviction::{EvictionPolicy, LruEvictionPolicy};
pub use metrics::{CacheMetrics, CacheMetricsSnapshot};
