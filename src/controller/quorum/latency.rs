//! Consensus latency tracking and statistics

use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};

/// Tracks consensus latency measurements for validators
pub struct ConsensusLatencyTracker {
    window_size: usize,
    measurements: HashMap<String, VecDeque<LatencyMeasurement>>,
}

/// A single latency measurement
#[derive(Clone, Debug)]
pub struct LatencyMeasurement {
    pub ledger_seq: u64,
    pub timestamp: DateTime<Utc>,
    pub latency_ms: u64,
}

/// Statistical summary of latency measurements
#[derive(Clone, Debug)]
pub struct LatencyStats {
    pub mean_ms: f64,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub variance: f64,
}

impl ConsensusLatencyTracker {
    /// Create a new latency tracker with the specified window size
    pub fn new(window_size: usize) -> Self {
        Self {
            window_size,
            measurements: HashMap::new(),
        }
    }

    /// Record a latency measurement for a validator
    pub fn record_latency(&mut self, validator: &str, ledger: u64, latency_ms: u64) {
        let measurement = LatencyMeasurement {
            ledger_seq: ledger,
            timestamp: Utc::now(),
            latency_ms,
        };

        let measurements = self
            .measurements
            .entry(validator.to_string())
            .or_default();

        measurements.push_back(measurement);

        // Maintain window size
        while measurements.len() > self.window_size {
            measurements.pop_front();
        }
    }

    /// Get statistical summary for a validator
    pub fn get_stats(&self, validator: &str) -> Option<LatencyStats> {
        let measurements = self.measurements.get(validator)?;

        if measurements.is_empty() {
            return None;
        }

        let mut latencies: Vec<u64> = measurements.iter().map(|m| m.latency_ms).collect();
        latencies.sort_unstable();

        let mean = latencies.iter().sum::<u64>() as f64 / latencies.len() as f64;

        let median = if latencies.len().is_multiple_of(2) {
            let mid = latencies.len() / 2;
            (latencies[mid - 1] + latencies[mid]) as f64 / 2.0
        } else {
            latencies[latencies.len() / 2] as f64
        };

        let p95_idx = ((latencies.len() as f64) * 0.95) as usize;
        let p95 = latencies.get(p95_idx.min(latencies.len() - 1)).copied().unwrap_or(0) as f64;

        let p99_idx = ((latencies.len() as f64) * 0.99) as usize;
        let p99 = latencies.get(p99_idx.min(latencies.len() - 1)).copied().unwrap_or(0) as f64;

        // Calculate variance
        let variance = latencies
            .iter()
            .map(|&x| {
                let diff = x as f64 - mean;
                diff * diff
            })
            .sum::<f64>()
            / latencies.len() as f64;

        Some(LatencyStats {
            mean_ms: mean,
            median_ms: median,
            p95_ms: p95,
            p99_ms: p99,
            variance,
        })
    }

    /// Get variance across all validators
    pub fn get_variance_across_validators(&self) -> f64 {
        let mut all_means = Vec::new();

        for validator in self.measurements.keys() {
            if let Some(stats) = self.get_stats(validator) {
                all_means.push(stats.mean_ms);
            }
        }

        if all_means.is_empty() {
            return 0.0;
        }

        let overall_mean = all_means.iter().sum::<f64>() / all_means.len() as f64;

        let variance = all_means
            .iter()
            .map(|&mean| {
                let diff = mean - overall_mean;
                diff * diff
            })
            .sum::<f64>()
            / all_means.len() as f64;

        variance
    }

    /// Get the number of measurements for a validator
    pub fn measurement_count(&self, validator: &str) -> usize {
        self.measurements
            .get(validator)
            .map(|m| m.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_tracker_creation() {
        let tracker = ConsensusLatencyTracker::new(100);
        assert_eq!(tracker.window_size, 100);
    }

    #[test]
    fn test_record_latency() {
        let mut tracker = ConsensusLatencyTracker::new(100);
        tracker.record_latency("V1", 1, 100);
        tracker.record_latency("V1", 2, 150);

        assert_eq!(tracker.measurement_count("V1"), 2);
    }

    #[test]
    fn test_window_size_enforcement() {
        let mut tracker = ConsensusLatencyTracker::new(3);
        
        for i in 0..5 {
            tracker.record_latency("V1", i, 100);
        }

        // Should only keep last 3 measurements
        assert_eq!(tracker.measurement_count("V1"), 3);
    }

    #[test]
    fn test_get_stats() {
        let mut tracker = ConsensusLatencyTracker::new(100);
        tracker.record_latency("V1", 1, 100);
        tracker.record_latency("V1", 2, 200);
        tracker.record_latency("V1", 3, 300);

        let stats = tracker.get_stats("V1").unwrap();
        assert_eq!(stats.mean_ms, 200.0);
        assert_eq!(stats.median_ms, 200.0);
    }

    #[test]
    fn test_empty_stats() {
        let tracker = ConsensusLatencyTracker::new(100);
        assert!(tracker.get_stats("V1").is_none());
    }
}
