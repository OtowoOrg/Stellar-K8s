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
//! Prometheus metrics for the fork detection sidecar.

use std::sync::atomic::{AtomicI64, AtomicU64};

use once_cell::sync::Lazy;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;

/// Labels for fork detector metrics.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ForkDetectorLabels {
    pub network: String,
    pub node_id: String,
}

/// Sync confidence gauge (0.0–1.0 encoded as integer millipercent: 0–1000).
/// Use `sync_confidence / 1000.0` to get the float value in Grafana.
pub static SYNC_CONFIDENCE: Lazy<Family<ForkDetectorLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Number of consecutive diverging ledgers.
pub static CONSECUTIVE_DIVERGING_LEDGERS: Lazy<Family<ForkDetectorLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Total fork alerts fired.
pub static FORK_ALERTS_TOTAL: Lazy<Family<ForkDetectorLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

/// Total poll errors (local + anchors combined).
pub static POLL_ERRORS_TOTAL: Lazy<Family<ForkDetectorLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

/// Current local ledger sequence.
pub static LOCAL_LEDGER_SEQUENCE: Lazy<Family<ForkDetectorLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Number of responding anchors in the last poll.
pub static RESPONDING_ANCHORS: Lazy<Family<ForkDetectorLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Build and return a fresh Prometheus registry with all fork detector metrics registered.
pub fn build_registry() -> Registry {
    let mut registry = Registry::default();

    registry.register(
        "stellar_fork_detector_sync_confidence",
        "Sync confidence as millipercent (0–1000). Divide by 1000 for 0.0–1.0 float. \
         Fraction of anchor nodes that agree with the local ledger hash.",
        SYNC_CONFIDENCE.clone(),
    );
    registry.register(
        "stellar_fork_detector_consecutive_diverging_ledgers",
        "Number of consecutive ledger cycles where the local hash diverged from the anchor majority.",
        CONSECUTIVE_DIVERGING_LEDGERS.clone(),
    );
    registry.register(
        "stellar_fork_detector_fork_alerts_total",
        "Total number of fork alerts fired (divergence persisted beyond threshold).",
        FORK_ALERTS_TOTAL.clone(),
    );
    registry.register(
        "stellar_fork_detector_poll_errors_total",
        "Total number of failed polls across local node and all anchors.",
        POLL_ERRORS_TOTAL.clone(),
    );
    registry.register(
        "stellar_fork_detector_local_ledger_sequence",
        "Latest ledger sequence observed from the local Stellar Core node.",
        LOCAL_LEDGER_SEQUENCE.clone(),
    );
    registry.register(
        "stellar_fork_detector_responding_anchors",
        "Number of anchor nodes that responded successfully in the last poll cycle.",
        RESPONDING_ANCHORS.clone(),
    );

    registry
}
