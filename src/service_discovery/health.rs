//! Health scoring and tracking for discovered services

use serde::{Deserialize, Serialize};

/// Composite health score for a service (0.0 = dead, 1.0 = perfect)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthScore {
    /// Overall score [0.0, 1.0]
    pub score: f64,
    /// Number of consecutive successful health checks
    pub consecutive_successes: u32,
    /// Number of consecutive failures
    pub consecutive_failures: u32,
    /// Error rate over the last N checks (0.0 to 1.0)
    pub error_rate: f64,
    /// Latency p95 in milliseconds
    pub latency_p95_ms: f64,
}

impl Default for HealthScore {
    fn default() -> Self {
        Self {
            score: 1.0,
            consecutive_successes: 0,
            consecutive_failures: 0,
            error_rate: 0.0,
            latency_p95_ms: 0.0,
        }
    }
}

impl HealthScore {
    pub fn is_healthy(&self) -> bool {
        self.score >= 0.5 && self.consecutive_failures < 3
    }

    pub fn is_degraded(&self) -> bool {
        self.score >= 0.2 && self.score < 0.5
    }

    /// Compute a new score from raw inputs
    pub fn compute(error_rate: f64, latency_p95_ms: f64) -> Self {
        // Penalise errors heavily; latency penalty is softer
        let error_penalty = error_rate.min(1.0);
        let latency_penalty = (latency_p95_ms / 5_000.0).min(0.5);
        let score = (1.0 - error_penalty - latency_penalty).max(0.0);
        Self {
            score,
            consecutive_successes: 0,
            consecutive_failures: 0,
            error_rate,
            latency_p95_ms,
        }
    }
}

/// Sliding-window health tracker for a single service
pub struct HealthTracker {
    window: Vec<bool>,
    latencies_ms: Vec<f64>,
    max_window: usize,
}

impl HealthTracker {
    pub fn new(window_size: usize) -> Self {
        Self {
            window: Vec::with_capacity(window_size),
            latencies_ms: Vec::with_capacity(window_size),
            max_window: window_size,
        }
    }

    pub fn record(&mut self, success: bool, latency_ms: f64) {
        if self.window.len() >= self.max_window {
            self.window.remove(0);
            self.latencies_ms.remove(0);
        }
        self.window.push(success);
        self.latencies_ms.push(latency_ms);
    }

    pub fn score(&self) -> HealthScore {
        if self.window.is_empty() {
            return HealthScore::default();
        }
        let failures = self.window.iter().filter(|&&s| !s).count();
        let error_rate = failures as f64 / self.window.len() as f64;
        let p95 = self.percentile_latency(95.0);
        let consecutive_failures = self.window.iter().rev().take_while(|&&s| !s).count() as u32;
        let consecutive_successes = self.window.iter().rev().take_while(|&&s| s).count() as u32;
        let mut score = HealthScore::compute(error_rate, p95);
        score.consecutive_failures = consecutive_failures;
        score.consecutive_successes = consecutive_successes;
        score
    }

    fn percentile_latency(&self, pct: f64) -> f64 {
        if self.latencies_ms.is_empty() {
            return 0.0;
        }
        let mut sorted = self.latencies_ms.clone();
        sorted.sort_by(f64::total_cmp);
        let idx = ((pct / 100.0) * sorted.len() as f64).ceil() as usize;
        sorted[idx.min(sorted.len()) - 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_healthy_score() {
        let score = HealthScore::compute(0.0, 50.0);
        assert!(score.is_healthy());
        assert!((score.score - 0.99).abs() < 0.01);
    }

    #[test]
    fn test_unhealthy_high_error_rate() {
        let score = HealthScore::compute(0.9, 100.0);
        assert!(!score.is_healthy());
    }

    #[test]
    fn test_tracker_sliding_window() {
        let mut tracker = HealthTracker::new(5);
        for _ in 0..5 {
            tracker.record(true, 30.0);
        }
        let score = tracker.score();
        assert!(score.is_healthy());
        assert_eq!(score.error_rate, 0.0);
    }
}
