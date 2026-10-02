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
//! Prometheus metrics definitions for the SCP eBPF sniffer.
//!
//! All metrics are exposed on the sniffer's own exporter port (default 9436)
//! so the operator's existing `monitor_ebpf_metrics` scraper (`:9435`) is not
//! affected.  The stellar-operator Prometheus scrape configuration should add
//! a job for this port alongside the existing eBPF exporter job.
//!
//! # Metric naming convention
//!
//! All metrics are prefixed with `scp_sniffer_` to prevent collision with
//! the generic `ebpf_*` metrics already scraped from the ebpf-exporter.
//!
//! # Label sets
//!
//! - `peer_ip`   — the remote Stellar node's IP address
//! - `event_type` — `scp_inbound`, `tcp_rst`, `retransmit`, `handshake_fail`

use std::sync::Arc;

use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};
use tracing::debug;

use crate::user::types::ScpEvent;

// ─── Label types ─────────────────────────────────────────────────────────────

/// Per-peer, per-event-type label set for event counters.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ScpEventLabels {
    /// Remote peer IP address (dotted decimal).
    pub peer_ip: String,
    /// Event type string (scp_inbound | tcp_rst | retransmit | handshake_fail).
    pub event_type: String,
}

/// Per-peer label set for drop / latency gauges.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct PeerLabels {
    /// Remote peer IP address (dotted decimal).
    pub peer_ip: String,
}

// ─── Registry wrapper ─────────────────────────────────────────────────────────

/// Central holder for all SCP sniffer Prometheus metrics.
///
/// Clone-safe via inner `Arc`; each background task holds a clone.
#[derive(Clone)]
pub struct ScpSnifferMetrics {
    inner: Arc<Inner>,
}

struct Inner {
    /// Number of SCP events observed, labelled by peer IP and event type.
    pub scp_events_total: Family<ScpEventLabels, Counter>,
    /// Total TCP RST packets per peer on port 11625.
    pub scp_rst_total: Family<PeerLabels, Counter>,
    /// Total TCP handshake failures (RST during SYN phase) per peer.
    pub scp_handshake_fail_total: Family<PeerLabels, Counter>,
    /// Total TCP retransmit events per peer.
    pub scp_retransmit_total: Family<PeerLabels, Counter>,
    /// Current count of unique SCP peers observed since startup.
    pub scp_peer_count: Gauge,
    /// Current count of peers classified as degraded (drop_rate > 5 %).
    pub scp_degraded_peer_count: Gauge,
    /// Perf ring-buffer events lost (kernel dropped before userspace could read).
    pub scp_lost_events_total: Counter,
    /// Inter-event gap histogram (nanoseconds) — proxy for SCP message latency.
    pub scp_event_gap_ns: Family<PeerLabels, Histogram>,
    /// Overall SCP network health score (0–100).
    pub scp_health_score: Gauge,
}

impl ScpSnifferMetrics {
    /// Create and register all metrics with the given [`Registry`].
    pub fn new(registry: &mut Registry) -> Self {
        let scp_events_total = Family::<ScpEventLabels, Counter>::default();
        registry.register(
            "scp_sniffer_events",
            "Total SCP events observed by the eBPF sniffer, by peer and event type",
            scp_events_total.clone(),
        );

        let scp_rst_total = Family::<PeerLabels, Counter>::default();
        registry.register(
            "scp_sniffer_rst",
            "Total TCP RST packets on SCP connections (port 11625), by peer",
            scp_rst_total.clone(),
        );

        let scp_handshake_fail_total = Family::<PeerLabels, Counter>::default();
        registry.register(
            "scp_sniffer_handshake_fail",
            "Total SCP TCP handshake failures (RST during SYN phase), by peer",
            scp_handshake_fail_total.clone(),
        );

        let scp_retransmit_total = Family::<PeerLabels, Counter>::default();
        registry.register(
            "scp_sniffer_retransmit",
            "Total TCP retransmit events on SCP connections, by peer",
            scp_retransmit_total.clone(),
        );

        let scp_peer_count = Gauge::default();
        registry.register(
            "scp_sniffer_peer_count",
            "Current number of unique SCP peers observed since sniffer start",
            scp_peer_count.clone(),
        );

        let scp_degraded_peer_count = Gauge::default();
        registry.register(
            "scp_sniffer_degraded_peer_count",
            "Number of SCP peers currently classified as degraded (drop rate > 5%)",
            scp_degraded_peer_count.clone(),
        );

        let scp_lost_events_total = Counter::default();
        registry.register(
            "scp_sniffer_lost_events",
            "Total BPF perf ring-buffer events dropped before userspace could read them",
            scp_lost_events_total.clone(),
        );

        // Bucket boundaries in nanoseconds: 100µs → 100ms range covers typical
        // SCP inter-message gaps and highlights latency outliers.
        let buckets = vec![
            100_000.0,   // 100 µs
            500_000.0,   // 500 µs
            1_000_000.0, // 1 ms
            5_000_000.0, // 5 ms
            10_000_000.0, // 10 ms
            50_000_000.0, // 50 ms
            100_000_000.0, // 100 ms
        ];
        let scp_event_gap_ns =
            Family::<PeerLabels, Histogram>::new_with_constructor(move || {
                Histogram::new(buckets.clone().into_iter())
            });
        registry.register(
            "scp_sniffer_event_gap_ns",
            "Histogram of time gaps between consecutive SCP events per peer (nanoseconds)",
            scp_event_gap_ns.clone(),
        );

        let scp_health_score = Gauge::default();
        registry.register(
            "scp_sniffer_health_score",
            "Overall SCP network health score (0 = fully degraded, 100 = healthy)",
            scp_health_score.clone(),
        );

        Self {
            inner: Arc::new(Inner {
                scp_events_total,
                scp_rst_total,
                scp_handshake_fail_total,
                scp_retransmit_total,
                scp_peer_count,
                scp_degraded_peer_count,
                scp_lost_events_total,
                scp_event_gap_ns,
                scp_health_score,
            }),
        }
    }

    /// Record a single decoded [`ScpEvent`] into the relevant counters.
    pub fn record_event(&self, event: &ScpEvent) {
        let peer_label = PeerLabels {
            peer_ip: event.peer_ip.clone(),
        };
        let event_label = ScpEventLabels {
            peer_ip: event.peer_ip.clone(),
            event_type: event.event_type.to_string(),
        };

        self.inner
            .scp_events_total
            .get_or_create(&event_label)
            .inc();

        match event.event_type {
            crate::user::types::EventType::TcpRst => {
                self.inner.scp_rst_total.get_or_create(&peer_label).inc();
            }
            crate::user::types::EventType::HandshakeFail => {
                self.inner
                    .scp_handshake_fail_total
                    .get_or_create(&peer_label)
                    .inc();
            }
            crate::user::types::EventType::Retransmit => {
                self.inner
                    .scp_retransmit_total
                    .get_or_create(&peer_label)
                    .inc();
            }
            _ => {}
        }

        debug!(
            peer_ip = %event.peer_ip,
            event_type = %event.event_type,
            "scp_sniffer: recorded event"
        );
    }

    /// Record lost events from the perf ring buffer.
    pub fn record_lost(&self, count: u64) {
        self.inner.scp_lost_events_total.inc_by(count);
    }

    /// Record an inter-event gap (nanoseconds) for a given peer.
    pub fn record_gap_ns(&self, peer_ip: &str, gap_ns: f64) {
        let label = PeerLabels {
            peer_ip: peer_ip.to_owned(),
        };
        self.inner
            .scp_event_gap_ns
            .get_or_create(&label)
            .observe(gap_ns);
    }

    /// Update the current peer count gauge.
    pub fn set_peer_count(&self, count: i64) {
        self.inner.scp_peer_count.set(count);
    }

    /// Update the current degraded-peer-count gauge.
    pub fn set_degraded_peer_count(&self, count: i64) {
        self.inner.scp_degraded_peer_count.set(count);
    }

    /// Update the overall health score gauge.
    pub fn set_health_score(&self, score: i64) {
        self.inner.scp_health_score.set(score);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_register_without_panic() {
        let mut registry = Registry::default();
        let _m = ScpSnifferMetrics::new(&mut registry);
        // Encode to text; should not panic.
        let mut buf = String::new();
        prometheus_client::encoding::text::encode(&mut buf, &registry).unwrap();
        assert!(buf.contains("scp_sniffer_events"));
        assert!(buf.contains("scp_sniffer_rst"));
        assert!(buf.contains("scp_sniffer_health_score"));
    }
}
