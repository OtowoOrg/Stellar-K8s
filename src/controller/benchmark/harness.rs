//! Reusable benchmark harness for operator reconciliation latency.
//!
//! Provides a generic harness that times repeated executions of a reconcile
//! closure and computes latency statistics (min, max, mean, p50, p95, p99).
//! The harness is intentionally side-effect free with respect to Kubernetes:
//! it accepts any async closure, making it usable in unit tests without a live
//! cluster.
//!
//! # Example
//!
//! ```rust,no_run
//! use stellar_k8s::controller::benchmark::harness::{HarnessConfig, ReconcileHarness};
//!
//! # tokio_test::block_on(async {
//! let harness = ReconcileHarness::new(HarnessConfig {
//!     iterations: 100,
//!     warmup_rounds: 5,
//! });
//!
//! let report = harness.run(|| async {
//!     // simulate one reconcile cycle
//!     tokio::time::sleep(std::time::Duration::from_micros(500)).await;
//! }).await;
//!
//! println!("p99 latency: {:.2} ms", report.p99_ms);
//! # });
//! ```

use std::future::Future;
use std::time::{Duration, Instant};

/// Configuration for [`ReconcileHarness`].
#[derive(Debug, Clone)]
pub struct HarnessConfig {
    /// Number of timed iterations to run after the warmup phase.
    pub iterations: usize,
    /// Number of untimed warmup calls before measurement begins.
    ///
    /// Warmup amortises JIT compilation, cache population, and other
    /// first-call overheads so they do not skew the reported statistics.
    pub warmup_rounds: usize,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            iterations: 100,
            warmup_rounds: 5,
        }
    }
}

/// Latency statistics produced by [`ReconcileHarness::run`].
#[derive(Debug, Clone, PartialEq)]
pub struct LatencyReport {
    /// Number of timed iterations included in the report.
    pub sample_count: usize,
    /// Minimum observed latency in milliseconds.
    pub min_ms: f64,
    /// Maximum observed latency in milliseconds.
    pub max_ms: f64,
    /// Arithmetic mean latency in milliseconds.
    pub mean_ms: f64,
    /// 50th percentile (median) latency in milliseconds.
    pub p50_ms: f64,
    /// 95th percentile latency in milliseconds.
    pub p95_ms: f64,
    /// 99th percentile latency in milliseconds.
    pub p99_ms: f64,
}

impl LatencyReport {
    /// Compute a report from a (mutable) collection of raw duration samples.
    ///
    /// The samples are sorted in place for percentile computation.
    pub fn compute(mut samples: Vec<Duration>) -> Self {
        let n = samples.len();
        if n == 0 {
            return Self {
                sample_count: 0,
                min_ms: 0.0,
                max_ms: 0.0,
                mean_ms: 0.0,
                p50_ms: 0.0,
                p95_ms: 0.0,
                p99_ms: 0.0,
            };
        }

        samples.sort_unstable();

        let to_ms = |d: &Duration| d.as_secs_f64() * 1_000.0;
        let total_ms: f64 = samples.iter().map(to_ms).sum();

        let percentile = |p: f64| -> f64 {
            let idx = ((n as f64 * p / 100.0).ceil() as usize)
                .saturating_sub(1)
                .min(n - 1);
            to_ms(&samples[idx])
        };

        Self {
            sample_count: n,
            min_ms: to_ms(&samples[0]),
            max_ms: to_ms(&samples[n - 1]),
            mean_ms: total_ms / n as f64,
            p50_ms: percentile(50.0),
            p95_ms: percentile(95.0),
            p99_ms: percentile(99.0),
        }
    }
}

/// Reusable harness for measuring operator reconciliation latency.
///
/// Accepts any async closure that represents a single reconcile cycle and
/// returns a [`LatencyReport`] summarising the observed wall-clock latency
/// distribution.
pub struct ReconcileHarness {
    config: HarnessConfig,
}

impl ReconcileHarness {
    /// Create a new harness with the given configuration.
    pub fn new(config: HarnessConfig) -> Self {
        Self { config }
    }

    /// Run the benchmark.
    ///
    /// The closure `f` is called `config.warmup_rounds` times (untimed) and
    /// then `config.iterations` times (timed). Returns a [`LatencyReport`]
    /// computed from the timed samples.
    pub async fn run<F, Fut>(&self, mut f: F) -> LatencyReport
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = ()>,
    {
        for _ in 0..self.config.warmup_rounds {
            f().await;
        }

        let mut samples = Vec::with_capacity(self.config.iterations);
        for _ in 0..self.config.iterations {
            let start = Instant::now();
            f().await;
            samples.push(start.elapsed());
        }

        LatencyReport::compute(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn harness_invokes_closure_correct_number_of_times() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let harness = ReconcileHarness::new(HarnessConfig {
            iterations: 10,
            warmup_rounds: 3,
        });

        harness
            .run(|| {
                let c = counter_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await;

        assert_eq!(counter.load(Ordering::SeqCst), 13, "warmup + iterations");
    }

    #[tokio::test]
    async fn report_has_correct_sample_count() {
        let harness = ReconcileHarness::new(HarnessConfig {
            iterations: 20,
            warmup_rounds: 0,
        });

        let report = harness.run(|| async {}).await;
        assert_eq!(report.sample_count, 20);
    }

    #[tokio::test]
    async fn percentiles_are_ordered() {
        let harness = ReconcileHarness::new(HarnessConfig {
            iterations: 50,
            warmup_rounds: 0,
        });

        let report = harness
            .run(|| async {
                tokio::time::sleep(Duration::from_micros(10)).await;
            })
            .await;

        assert!(report.min_ms <= report.p50_ms, "min <= p50");
        assert!(report.p50_ms <= report.p95_ms, "p50 <= p95");
        assert!(report.p95_ms <= report.p99_ms, "p95 <= p99");
        assert!(report.p99_ms <= report.max_ms, "p99 <= max");
    }

    #[test]
    fn compute_empty_samples_returns_zero_report() {
        let report = LatencyReport::compute(vec![]);
        assert_eq!(report.sample_count, 0);
        assert_eq!(report.min_ms, 0.0);
        assert_eq!(report.p99_ms, 0.0);
    }

    #[test]
    fn compute_single_sample() {
        let samples = vec![Duration::from_millis(42)];
        let report = LatencyReport::compute(samples);
        assert_eq!(report.sample_count, 1);
        assert!((report.min_ms - 42.0).abs() < 1e-6);
        assert!((report.max_ms - 42.0).abs() < 1e-6);
        assert!((report.p50_ms - 42.0).abs() < 1e-6);
        assert!((report.p99_ms - 42.0).abs() < 1e-6);
    }

    #[test]
    fn compute_known_distribution() {
        // 10 samples: 10ms, 20ms, ..., 100ms
        let samples: Vec<Duration> = (1..=10).map(|i| Duration::from_millis(i * 10)).collect();
        let report = LatencyReport::compute(samples);
        assert_eq!(report.sample_count, 10);
        assert!((report.min_ms - 10.0).abs() < 1e-6);
        assert!((report.max_ms - 100.0).abs() < 1e-6);
        // p50 = ceil(10 * 0.5) = 5th element (50ms)
        assert!((report.p50_ms - 50.0).abs() < 1e-6);
        // p95 = ceil(10 * 0.95) = 10th element (100ms)
        assert!((report.p95_ms - 100.0).abs() < 1e-6);
        // mean = 550 / 10 = 55ms
        assert!((report.mean_ms - 55.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn default_config_runs_without_error() {
        let harness = ReconcileHarness::new(HarnessConfig::default());
        let report = harness.run(|| async {}).await;
        assert_eq!(report.sample_count, 100);
        assert!(report.min_ms >= 0.0);
    }
}
