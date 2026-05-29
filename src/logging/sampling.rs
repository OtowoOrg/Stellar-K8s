//! Intelligent Log Sampling
//!
//! Provides logic to reduce log volume while preserving important events
//! and maintaining contextual traces.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{Level, Metadata};

/// Sampling configuration
pub struct SamplingConfig {
    /// Default sampling rate for INFO logs (0.0 to 1.0)
    pub info_rate: f64,
    /// Default sampling rate for DEBUG logs (0.0 to 1.0)
    pub debug_rate: f64,
    /// Always sample these targets regardless of level
    pub priority_targets: Vec<String>,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            info_rate: 0.5,
            debug_rate: 0.1,
            priority_targets: vec!["stellar_k8s::controller::reconciler".to_string()],
        }
    }
}

pub struct Sampler {
    config: SamplingConfig,
    counter: Arc<AtomicU64>,
}

impl Sampler {
    pub fn new(config: SamplingConfig) -> Self {
        Self {
            config,
            counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Determines if a log event should be sampled (kept).
    pub fn should_sample(&self, metadata: &Metadata<'_>) -> bool {
        // Always keep ERROR and WARN
        if *metadata.level() <= Level::WARN {
            return true;
        }

        // Check priority targets
        if self.config.priority_targets.iter().any(|t| metadata.target().starts_with(t)) {
            return true;
        }

        let rate = match *metadata.level() {
            Level::INFO => self.config.info_rate,
            Level::DEBUG | Level::TRACE => self.config.debug_rate,
            _ => 1.0,
        };

        if rate >= 1.0 {
            return true;
        }
        if rate <= 0.0 {
            return false;
        }

        // Deterministic sampling based on a counter
        let count = self.counter.fetch_add(1, Ordering::Relaxed);
        let threshold = (u64::MAX as f64 * rate) as u64;
        
        // Use a simple hash-like approach for the counter to avoid streaks
        let hash = count.wrapping_mul(0x517cc1b727220a95);
        hash < threshold
    }
}
