//! Fork Detection Sidecar
//!
//! Monitors the local Stellar Core ledger hash and compares it in real-time
//! against multiple public anchor nodes to detect potential network forks.
//!
//! # Architecture
//!
//! - Polls the local Stellar Core node (`/info`) every `poll_interval_secs`.
//! - Concurrently polls 3+ public anchor nodes.
//! - Compares the local hash against the anchor majority hash at the same ledger sequence.
//! - If divergence persists for more than `divergence_threshold_ledgers` consecutive ledgers,
//!   fires an alert (log + Prometheus metric + Kubernetes Event).
//! - Exports `stellar_fork_detector_sync_confidence` as a Prometheus gauge (0.0–1.0).

pub mod detector;
pub mod metrics;
pub mod types;

pub use detector::run_fork_detector;
pub use types::ForkDetectorConfig;
