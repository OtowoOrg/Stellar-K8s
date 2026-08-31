// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
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
