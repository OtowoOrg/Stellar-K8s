//! Scheduling metrics and performance tracking.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchedulingMetrics {
    pub total_scheduled: u64,
    pub total_failed: u64,
    pub total_preemptions: u64,
    pub scheduling_latency_p50_ms: f64,
    pub scheduling_latency_p95_ms: f64,
    pub scheduling_latency_p99_ms: f64,
    pub cost_savings_usd: f64,
    pub last_updated: Option<DateTime<Utc>>,
}

pub struct SchedulingMetricsCollector {
    latencies_ms: Vec<f64>,
    metrics: SchedulingMetrics,
}

impl SchedulingMetricsCollector {
    pub fn new() -> Self {
        Self {
            latencies_ms: Vec::new(),
            metrics: SchedulingMetrics::default(),
        }
    }

    pub fn record_scheduling_success(&mut self, latency_ms: f64) {
        self.metrics.total_scheduled += 1;
        self.latencies_ms.push(latency_ms);
        self.recompute_percentiles();
        self.metrics.last_updated = Some(Utc::now());
    }

    pub fn record_scheduling_failure(&mut self) {
        self.metrics.total_failed += 1;
        self.metrics.last_updated = Some(Utc::now());
    }

    pub fn record_preemption(&mut self) {
        self.metrics.total_preemptions += 1;
    }

    pub fn record_cost_saving(&mut self, saved_usd: f64) {
        self.metrics.cost_savings_usd += saved_usd;
    }

    pub fn snapshot(&self) -> SchedulingMetrics {
        self.metrics.clone()
    }

    fn recompute_percentiles(&mut self) {
        if self.latencies_ms.is_empty() {
            return;
        }
        let mut sorted = self.latencies_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        self.metrics.scheduling_latency_p50_ms = percentile(&sorted, 50.0);
        self.metrics.scheduling_latency_p95_ms = percentile(&sorted, 95.0);
        self.metrics.scheduling_latency_p99_ms = percentile(&sorted, 99.0);
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

impl Default for SchedulingMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}
