//! Gateway module: HTTP proxy middleware for Soroban RPC nodes.
//!
//! Provides request rate-limiting, IP tracking, and dynamic throttling based
//! on live CPU utilisation metrics.
//!
//! # Modules
//!
//! - [`ratelimit`]: sliding-window per-IP tracker + CPU-aware engine

pub mod ratelimit;

pub use ratelimit::{
    extract_client_ip, EngineMetrics, RateLimitConfig, RateLimitDecision, RateLimitEngine,
    SlidingWindowTracker,
};
