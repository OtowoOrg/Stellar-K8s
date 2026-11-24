//! Rate-limiting subsystem for the Soroban RPC gateway.
//!
//! Exposes two submodules:
//!
//! - [`window`]: per-IP sliding window request tracker
//! - [`engine`]: dynamic rate-limiter engine with CPU-aware thresholds

pub mod engine;
pub mod window;

pub use engine::{
    extract_client_ip, EngineMetrics, RateLimitConfig, RateLimitDecision, RateLimitEngine,
};
pub use window::SlidingWindowTracker;
