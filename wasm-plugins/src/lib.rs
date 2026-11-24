//! Wasm-compatible bounded fail-open cache for Soroban RPC state reads.
//!
//! This library also provides a Wasm-based Quorum Set Validation Engine for
//! Stellar SCP configurations.

pub mod cache;
pub mod quorum_eval;

pub use cache::{
    CacheConfig, CacheError, CacheStats, StateCache, DEFAULT_MAX_BYTES, DEFAULT_MAX_ENTRIES,
    DEFAULT_TTL_SECS, MAX_CACHE_BYTES, MAX_CACHE_ENTRIES,
};
pub use quorum_eval::{
    QuorumSetConfig, QuorumValidator, ValidationError, ValidationMetadata, ValidationPolicy,
    ValidationResult,
};

#[cfg(target_arch = "wasm32")]
#[path = "wasm.rs"]
mod wasm;

#[cfg(target_arch = "wasm32")]
mod quorum_eval_wasm;

#[cfg(target_arch = "wasm32")]
pub use wasm::*;

#[cfg(target_arch = "wasm32")]
pub use quorum_eval_wasm::*;
