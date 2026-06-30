//! Pipeline throughput, latency, and error metrics.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Shared pipeline metrics counters.
#[derive(Clone, Default)]
pub struct PipelineMetrics {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    records_received: AtomicU64,
    records_transformed: AtomicU64,
    records_dlq: AtomicU64,
    transform_errors: AtomicU64,
    sink_successes: AtomicU64,
    sink_errors: AtomicU64,
    /// Cumulative processing latency in microseconds (for mean calculation)
    latency_us_total: AtomicU64,
    latency_samples: AtomicU64,
}

impl PipelineMetrics {
    pub fn record_received(&self) {
        self.inner.records_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_transformed(&self) {
        self.inner
            .records_transformed
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_dlq(&self) {
        self.inner.records_dlq.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_transform_error(&self) {
        self.inner.transform_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_sink_success(&self) {
        self.inner.sink_successes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_sink_error(&self) {
        self.inner.sink_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_latency(&self, start: Instant) {
        let us = start.elapsed().as_micros() as u64;
        self.inner.latency_us_total.fetch_add(us, Ordering::Relaxed);
        self.inner.latency_samples.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let samples = self.inner.latency_samples.load(Ordering::Relaxed);
        let mean_latency_us = self
            .inner
            .latency_us_total
            .load(Ordering::Relaxed)
            .checked_div(samples)
            .unwrap_or(0);
        MetricsSnapshot {
            records_received: self.inner.records_received.load(Ordering::Relaxed),
            records_transformed: self.inner.records_transformed.load(Ordering::Relaxed),
            records_dlq: self.inner.records_dlq.load(Ordering::Relaxed),
            transform_errors: self.inner.transform_errors.load(Ordering::Relaxed),
            sink_successes: self.inner.sink_successes.load(Ordering::Relaxed),
            sink_errors: self.inner.sink_errors.load(Ordering::Relaxed),
            mean_latency_us,
        }
    }
}

/// Point-in-time snapshot of pipeline metrics.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub records_received: u64,
    pub records_transformed: u64,
    pub records_dlq: u64,
    pub transform_errors: u64,
    pub sink_successes: u64,
    pub sink_errors: u64,
    /// Mean end-to-end processing latency in microseconds
    pub mean_latency_us: u64,
}
