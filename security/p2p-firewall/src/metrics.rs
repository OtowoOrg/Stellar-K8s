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

//! Prometheus metrics for the P2P Gossip Threat Detection Firewall.
//!
//! Exposes a `/metrics` HTTP endpoint at the configured address.  All metrics
//! follow the Stellar-K8s naming convention: `p2p_firewall_*`.
//!
//! | Metric | Type | Description |
//! |---|---|---|
//! | `p2p_firewall_packets_total` | Counter | Total SCP packets inspected |
//! | `p2p_firewall_threats_total` | Counter | Threats detected by level |
//! | `p2p_firewall_blacklisted_ips` | Gauge | Currently blacklisted IPs |
//! | `p2p_firewall_blacklist_events_total` | Counter | Blacklist add/remove events |
//! | `p2p_firewall_analysis_duration_seconds` | Histogram | Per-packet analysis latency |
//! | `p2p_firewall_malformed_packets_total` | Counter | Malformed XDR packets |
//! | `p2p_firewall_flood_events_total` | Counter | Flood events detected |
//! | `p2p_firewall_handshake_failures_total` | Counter | Rapid handshake failures |
//! | `p2p_firewall_network_policy_updates_total` | Counter | K8s NetworkPolicy sync operations |

use anyhow::Result;
use once_cell::sync::Lazy;
use prometheus_client::{
    encoding::{text::encode, EncodeLabelSet},
    metrics::{
        counter::Counter,
        family::Family,
        gauge::Gauge,
        histogram::{exponential_buckets, Histogram},
    },
    registry::Registry,
};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tracing::{error, info};

// ── Label types ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ThreatLabels {
    pub threat_level: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct BlacklistEventLabels {
    pub action: String, // "added" | "removed" | "expired"
}

// ── Metrics registry ───────────────────────────────────────────────────────────

/// All firewall Prometheus metrics, bundled for easy sharing across tasks.
#[derive(Clone)]
pub struct FirewallMetrics {
    inner: Arc<FirewallMetricsInner>,
}

struct FirewallMetricsInner {
    pub registry: Mutex<Registry>,

    // Counters
    pub packets_total: Counter,
    pub threats_total: Family<ThreatLabels, Counter>,
    pub blacklist_events_total: Family<BlacklistEventLabels, Counter>,
    pub malformed_packets_total: Counter,
    pub flood_events_total: Counter,
    pub handshake_failures_total: Counter,
    pub network_policy_updates_total: Counter,

    // Gauges
    pub blacklisted_ips: Gauge,

    // Histograms
    pub analysis_duration_seconds: Histogram,
}

impl FirewallMetrics {
    /// Create and register all metrics.
    pub fn new() -> Self {
        let mut registry = Registry::default();

        let packets_total = Counter::default();
        registry.register(
            "p2p_firewall_packets_total",
            "Total SCP packets inspected by the firewall",
            packets_total.clone(),
        );

        let threats_total = Family::<ThreatLabels, Counter>::default();
        registry.register(
            "p2p_firewall_threats_total",
            "Total threats detected, labelled by threat level",
            threats_total.clone(),
        );

        let blacklisted_ips = Gauge::default();
        registry.register(
            "p2p_firewall_blacklisted_ips",
            "Current number of blacklisted IP addresses",
            blacklisted_ips.clone(),
        );

        let blacklist_events_total = Family::<BlacklistEventLabels, Counter>::default();
        registry.register(
            "p2p_firewall_blacklist_events_total",
            "Total blacklist add/remove/expire events",
            blacklist_events_total.clone(),
        );

        let analysis_duration_seconds = Histogram::new(exponential_buckets(0.000_01, 2.0, 15));
        registry.register(
            "p2p_firewall_analysis_duration_seconds",
            "Per-packet XDR analysis duration in seconds",
            analysis_duration_seconds.clone(),
        );

        let malformed_packets_total = Counter::default();
        registry.register(
            "p2p_firewall_malformed_packets_total",
            "Total malformed XDR packets detected",
            malformed_packets_total.clone(),
        );

        let flood_events_total = Counter::default();
        registry.register(
            "p2p_firewall_flood_events_total",
            "Total packet flood events detected",
            flood_events_total.clone(),
        );

        let handshake_failures_total = Counter::default();
        registry.register(
            "p2p_firewall_handshake_failures_total",
            "Total rapid handshake failure events detected",
            handshake_failures_total.clone(),
        );

        let network_policy_updates_total = Counter::default();
        registry.register(
            "p2p_firewall_network_policy_updates_total",
            "Total Kubernetes NetworkPolicy sync operations performed",
            network_policy_updates_total.clone(),
        );

        Self {
            inner: Arc::new(FirewallMetricsInner {
                registry: Mutex::new(registry),
                packets_total,
                threats_total,
                blacklist_events_total,
                malformed_packets_total,
                flood_events_total,
                handshake_failures_total,
                network_policy_updates_total,
                blacklisted_ips,
                analysis_duration_seconds,
            }),
        }
    }

    // ── Record helpers ─────────────────────────────────────────────────────────

    pub fn record_packet(&self) {
        self.inner.packets_total.inc();
    }

    pub fn record_threat(&self, level: &str) {
        self.inner
            .threats_total
            .get_or_create(&ThreatLabels {
                threat_level: level.to_string(),
            })
            .inc();
    }

    pub fn set_blacklisted_ips(&self, count: i64) {
        self.inner.blacklisted_ips.set(count);
    }

    pub fn record_blacklist_event(&self, action: &str) {
        self.inner
            .blacklist_events_total
            .get_or_create(&BlacklistEventLabels {
                action: action.to_string(),
            })
            .inc();
    }

    pub fn record_analysis_duration(&self, seconds: f64) {
        self.inner.analysis_duration_seconds.observe(seconds);
    }

    pub fn record_malformed_packet(&self) {
        self.inner.malformed_packets_total.inc();
    }

    pub fn record_flood_event(&self) {
        self.inner.flood_events_total.inc();
    }

    pub fn record_handshake_failure(&self) {
        self.inner.handshake_failures_total.inc();
    }

    pub fn record_network_policy_update(&self) {
        self.inner.network_policy_updates_total.inc();
    }

    // ── Encoding ───────────────────────────────────────────────────────────────

    /// Encode all metrics into the Prometheus text exposition format.
    pub fn encode(&self) -> Result<String> {
        let registry = self
            .inner
            .registry
            .lock()
            .expect("metrics registry lock poisoned");
        let mut output = String::new();
        encode(&mut output, &registry).map_err(|e| anyhow::anyhow!("metrics encode error: {e}"))?;
        Ok(output)
    }

    /// Start a Prometheus `/metrics` HTTP server on `bind_addr`.
    pub async fn serve(self, bind_addr: String) -> Result<()> {
        let listener = TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| anyhow::anyhow!("metrics server bind failed on {bind_addr}: {e}"))?;

        info!(addr = %bind_addr, "Prometheus metrics server listening");

        loop {
            match listener.accept().await {
                Ok((mut stream, _peer)) => {
                    let metrics = self.clone();
                    tokio::spawn(async move {
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};

                        // Read and discard request.
                        let mut buf = vec![0u8; 4096];
                        let _ = stream.read(&mut buf).await;

                        match metrics.encode() {
                            Ok(body) => {
                                let response = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\n\r\n{}",
                                    body.len(),
                                    body
                                );
                                let _ = stream.write_all(response.as_bytes()).await;
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to encode metrics");
                                let _ = stream
                                    .write_all(b"HTTP/1.1 500 Internal Server Error\r\n\r\n")
                                    .await;
                            }
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "Metrics server accept() failed");
                }
            }
        }
    }
}

impl Default for FirewallMetrics {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_encode() {
        let metrics = FirewallMetrics::new();
        metrics.record_packet();
        metrics.record_packet();
        metrics.record_threat("CRITICAL");
        metrics.record_malformed_packet();
        metrics.set_blacklisted_ips(3);

        let output = metrics.encode().expect("encode should succeed");
        assert!(output.contains("p2p_firewall_packets_total"));
        assert!(output.contains("p2p_firewall_threats_total"));
        assert!(output.contains("p2p_firewall_blacklisted_ips"));
        assert!(output.contains("p2p_firewall_malformed_packets_total"));
    }

    #[test]
    fn test_counter_increments() {
        let metrics = FirewallMetrics::new();
        for _ in 0..5 {
            metrics.record_packet();
        }
        metrics.record_flood_event();
        metrics.record_handshake_failure();
        metrics.record_network_policy_update();

        let output = metrics.encode().unwrap();
        // Validate counters appear in output
        assert!(output.contains("p2p_firewall_flood_events_total"));
        assert!(output.contains("p2p_firewall_handshake_failures_total"));
        assert!(output.contains("p2p_firewall_network_policy_updates_total"));
    }

    #[test]
    fn test_analysis_duration_histogram() {
        let metrics = FirewallMetrics::new();
        metrics.record_analysis_duration(0.000_050); // 50 µs
        metrics.record_analysis_duration(0.000_100); // 100 µs
        metrics.record_analysis_duration(0.000_200); // 200 µs

        let output = metrics.encode().unwrap();
        assert!(output.contains("p2p_firewall_analysis_duration_seconds"));
        assert!(output.contains("_bucket"));
    }

    #[test]
    fn test_blacklist_gauge() {
        let metrics = FirewallMetrics::new();
        metrics.set_blacklisted_ips(0);
        metrics.set_blacklisted_ips(5);
        metrics.set_blacklisted_ips(2);

        let output = metrics.encode().unwrap();
        assert!(output.contains("p2p_firewall_blacklisted_ips"));
    }
}
