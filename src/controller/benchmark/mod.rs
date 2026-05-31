//! Benchmark controller module
//!
//! Reconciles `StellarBenchmark` resources by:
//! 1. Spinning up ephemeral load-generator pods.
//! 2. Waiting for them to complete.
//! 3. Collecting and aggregating metrics.
//! 4. Writing results to a `BenchmarkReport` CR or a `ConfigMap`.

pub mod collector;
pub mod pod_builder;
pub mod reconciler;

pub use reconciler::run_benchmark_controller;
