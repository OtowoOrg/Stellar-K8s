//! Peer uptime tracking for quorum optimization.
//!
//! We treat "uptime" as the fraction of nodes in the observed validator set
//! that currently report a given peer as connected/authenticated.

use std::collections::{HashMap, VecDeque};

/// Tracks rolling uptime ratio samples for quorum peers.
///
/// Each reconcile run adds one sample per peer. The sample is the fraction
/// of pods that see that peer in a connected/authenticated state.
#[derive(Clone, Debug)]
pub struct PeerUptimeTracker {
    window_size: usize,
    samples: HashMap<String, VecDeque<f64>>,
}

impl PeerUptimeTracker {
    pub fn new(window_size: usize) -> Self {
        Self {
            window_size,
            samples: HashMap::new(),
        }
    }

    /// Record one uptime ratio sample for a peer.
    ///
    /// `uptime_ratio` is clamped into [0.0, 1.0].
    pub fn record_uptime_ratio(&mut self, peer: &str, uptime_ratio: f64) {
        let ratio = uptime_ratio.clamp(0.0, 1.0);
        let samples = self.samples.entry(peer.to_string()).or_default();
        samples.push_back(ratio);

        // Maintain rolling window.
        while samples.len() > self.window_size {
            samples.pop_front();
        }
    }

    /// Get the mean uptime ratio over the rolling window.
    pub fn get_mean_uptime(&self, peer: &str) -> Option<f64> {
        let samples = self.samples.get(peer)?;
        if samples.is_empty() {
            return None;
        }
        let sum = samples.iter().sum::<f64>();
        Some(sum / samples.len() as f64)
    }

    pub fn mean_uptime_count(&self, peer: &str) -> usize {
        self.samples.get(peer).map(|s| s.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_mean() {
        let mut t = PeerUptimeTracker::new(10);
        t.record_uptime_ratio("p1", 1.0);
        t.record_uptime_ratio("p1", 0.5);

        let mean = t.get_mean_uptime("p1").unwrap();
        assert!((mean - 0.75).abs() < 1e-9);
        assert_eq!(t.mean_uptime_count("p1"), 2);
    }

    #[test]
    fn test_window_enforcement() {
        let mut t = PeerUptimeTracker::new(3);
        for _ in 0..5 {
            t.record_uptime_ratio("p1", 0.9);
        }
        assert_eq!(t.mean_uptime_count("p1"), 3);
    }

    #[test]
    fn test_clamps_ratio() {
        let mut t = PeerUptimeTracker::new(3);
        t.record_uptime_ratio("p1", 2.0);
        t.record_uptime_ratio("p2", -1.0);
        assert_eq!(t.get_mean_uptime("p1").unwrap(), 1.0);
        assert_eq!(t.get_mean_uptime("p2").unwrap(), 0.0);
    }
}
