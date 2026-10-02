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
//! Operator-side integration for the SCP eBPF network sniffer.
//!
//! # Overview
//!
//! This module is the bridge between the kernel-level `scp_trace` BPF program
//! and the rest of the stellar-operator.  It is responsible for:
//!
//! 1. **Scraping** the sniffer's Prometheus exporter (port 9436) for raw SCP
//!    drop counters, following the same pattern as the existing
//!    `monitor_ebpf_metrics` function in `sidecar.rs` that polls port 9435.
//!
//! 2. **Maintaining** an in-memory [`ScpSnifferStore`] — an `Arc<RwLock<...>>`
//!    that is shared directly with the axum handler state so that REST API
//!    responses never require an extra HTTP round-trip.
//!
//! 3. **Emitting** Kubernetes `Event` objects when a peer's drop rate crosses
//!    the degraded threshold or when a handshake failure is detected, giving
//!    operators immediate visibility in `kubectl describe stellarnode`.
//!
//! 4. **Classifying** each drop event as a cloud-provider network fault
//!    (TCP RST without any SCP application error) vs. a Stellar protocol
//!    fault (connection established but SCP diverges), enabling the
//!    "Definition of Done" — isolating cloud faults from protocol faults.
//!
//! # Wiring
//!
//! `src/security/mod.rs` declares `pub mod ebpf_sniffer` and re-exports the
//! public surface.  `src/lib.rs` already declares `pub mod security`, so no
//! changes are needed there.
//!
//! The operator main function (or controller startup) should call
//! [`ScpSnifferMonitor::spawn`] once per node host, passing in the
//! Kubernetes client and the shared store handle.
//!
//! The REST API server receives the store handle via
//! [`ScpSnifferStore::new_shared`] and passes it through axum `Extension`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::Utc;
use k8s_openapi::api::core::v1::{Event, ObjectReference};
use kube::api::{Api, ObjectMeta, PostParams};
use kube::Client;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::telemetry::inject_trace_headers;

// ─── Constants ────────────────────────────────────────────────────────────────

/// Default SCP sniffer Prometheus exporter port.
pub const SNIFFER_EXPORTER_PORT: u16 = 9436;

/// How often the operator polls the sniffer exporter.
const POLL_INTERVAL_SECS: u64 = 10;

/// A peer is considered degraded when its RST drop rate exceeds this threshold.
const DEGRADED_DROP_RATE_PCT: f64 = 5.0;

/// Minimum RST count before raising a K8s Event (avoids noise from transient spikes).
const MIN_RST_FOR_EVENT: u64 = 3;

// ─── Shared store ─────────────────────────────────────────────────────────────

/// Per-peer network health snapshot stored by the operator integration layer.
///
/// This struct is intentionally independent of the `security/ebpf-sniffer`
/// crate so that the operator does not need to compile the sniffer crate.
/// The data is populated by parsing Prometheus text format scraped from port 9436.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeerNetworkStats {
    /// Remote peer IP address.
    pub peer_ip: String,
    /// Total TCP RST packets observed by the sniffer for this peer.
    pub rst_total: u64,
    /// TCP RSTs during the handshake phase (cryptographic failures).
    pub handshake_fail_total: u64,
    /// Total TCP retransmit events.
    pub retransmit_total: u64,
    /// Total inbound SCP segments.
    pub packets_rx: u64,
    /// Computed drop rate as a percentage.
    pub drop_rate_pct: f64,
    /// Whether this peer is currently classified as degraded.
    pub is_degraded: bool,
    /// ISO-8601 timestamp of the last scrape that included data for this peer.
    pub last_updated: String,
}

/// Overall SCP network health state maintained by the operator.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScpNetworkHealthState {
    /// ISO-8601 timestamp of the most recent successful scrape.
    pub last_scraped: String,
    /// Whether the sniffer exporter is reachable.
    pub sniffer_available: bool,
    /// Overall health score scraped directly from `scp_sniffer_health_score`.
    pub health_score: u8,
    /// Total unique peers observed.
    pub total_peers: usize,
    /// Number of peers currently degraded.
    pub degraded_peers: usize,
    /// Per-peer statistics, keyed by peer IP.
    pub peers: HashMap<String, PeerNetworkStats>,
}

/// Thread-safe wrapper around [`ScpNetworkHealthState`] shared between the
/// background monitor task and the REST API handlers.
#[derive(Debug, Default, Clone)]
pub struct ScpSnifferStore {
    inner: Arc<RwLock<ScpNetworkHealthState>>,
}

impl ScpSnifferStore {
    /// Create a new, empty store.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(ScpNetworkHealthState::default())),
        }
    }

    /// Wrap an existing `Arc<RwLock<...>>` (e.g. passed in from controller startup).
    pub fn new_shared() -> Arc<RwLock<ScpNetworkHealthState>> {
        Arc::new(RwLock::new(ScpNetworkHealthState::default()))
    }

    /// Read a snapshot of the current state.
    pub fn snapshot(&self) -> ScpNetworkHealthState {
        self.inner
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Replace the entire state with a newly scraped value.
    pub fn update(&self, state: ScpNetworkHealthState) {
        if let Ok(mut g) = self.inner.write() {
            *g = state;
        }
    }
}

// ─── Monitor task ─────────────────────────────────────────────────────────────

/// Background task that polls the sniffer exporter and emits K8s Events.
pub struct ScpSnifferMonitor {
    /// Kubernetes API client.
    client: Client,
    /// Namespace of the StellarNode pod being monitored.
    namespace: String,
    /// Name of the StellarNode resource (used as subject of K8s Events).
    node_name: String,
    /// Shared store written by this task and read by the REST API handlers.
    store: ScpSnifferStore,
    /// Sniffer exporter base URL (default `http://localhost:9436`).
    exporter_url: String,
    /// HTTP client with trace injection.
    http: reqwest::Client,
    /// Tracks which peers have already had an Event raised in this run (avoids
    /// flooding the K8s event stream with duplicate events).
    alerted_peers: HashMap<String, u64>,
}

impl ScpSnifferMonitor {
    /// Construct a new monitor.
    ///
    /// # Arguments
    ///
    /// - `client`      — Kubernetes API client (cheaply cloneable).
    /// - `namespace`   — namespace of the StellarNode being observed.
    /// - `node_name`   — name of the StellarNode CR (used in K8s Events).
    /// - `store`       — shared [`ScpSnifferStore`] (also held by REST handlers).
    /// - `exporter_port` — port for the sniffer Prometheus exporter
    ///                     (defaults to [`SNIFFER_EXPORTER_PORT`]).
    pub fn new(
        client: Client,
        namespace: String,
        node_name: String,
        store: ScpSnifferStore,
        exporter_port: Option<u16>,
    ) -> Self {
        let port = exporter_port.unwrap_or(SNIFFER_EXPORTER_PORT);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        Self {
            client,
            namespace,
            node_name,
            store,
            exporter_url: format!("http://localhost:{port}/metrics"),
            http,
            alerted_peers: HashMap::new(),
        }
    }

    /// Spawn the monitor loop as a Tokio background task and return the
    /// [`ScpSnifferStore`] handle for the REST API to use.
    ///
    /// ```rust,ignore
    /// let store = ScpSnifferMonitor::spawn(
    ///     client.clone(), "stellar".into(), "my-validator".into(), None
    /// );
    /// // Pass `store` into axum router Extension.
    /// ```
    pub fn spawn(
        client: Client,
        namespace: String,
        node_name: String,
        exporter_port: Option<u16>,
    ) -> ScpSnifferStore {
        let store = ScpSnifferStore::new();
        let monitor = Self::new(
            client,
            namespace,
            node_name,
            store.clone(),
            exporter_port,
        );
        tokio::spawn(async move { monitor.run().await });
        store
    }

    /// Main polling loop.  Runs until the Tokio runtime is dropped.
    pub async fn run(mut self) {
        info!(
            node = %self.node_name,
            url  = %self.exporter_url,
            "SCP sniffer monitor starting"
        );

        loop {
            if let Err(e) = self.poll_and_update().await {
                warn!(
                    node = %self.node_name,
                    error = %e,
                    "SCP sniffer monitor: scrape failed"
                );
                // Mark sniffer as unavailable in the store.
                let mut snap = self.store.snapshot();
                snap.sniffer_available = false;
                snap.last_scraped = Utc::now().to_rfc3339();
                self.store.update(snap);
            }

            sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
        }
    }

    /// Scrape the exporter, parse counters, update the store, and emit K8s
    /// Events for degraded peers.
    async fn poll_and_update(&mut self) -> anyhow::Result<()> {
        let mut headers = reqwest::header::HeaderMap::new();
        inject_trace_headers(&mut headers);

        let resp = self
            .http
            .get(&self.exporter_url)
            .headers(headers)
            .send()
            .await?;

        let text = resp.text().await?;
        let state = parse_prometheus_text(&text);

        // Identify newly degraded or worsened peers.
        for (ip, peer) in &state.peers {
            if !peer.is_degraded {
                continue;
            }

            let prev_rst = self.alerted_peers.get(ip).copied().unwrap_or(0);
            if peer.rst_total >= MIN_RST_FOR_EVENT && peer.rst_total > prev_rst {
                self.alerted_peers.insert(ip.clone(), peer.rst_total);
                if let Err(e) = self.emit_drop_event(peer).await {
                    warn!(
                        peer_ip = %ip,
                        error   = %e,
                        "SCP sniffer: failed to emit K8s Event for degraded peer"
                    );
                }
            }
        }

        self.store.update(state);
        debug!(node = %self.node_name, "SCP sniffer store updated");
        Ok(())
    }

    /// Emit a Kubernetes `Warning` Event on the StellarNode resource describing
    /// which peer IP is dropping SCP connections and why.
    ///
    /// The event message intentionally distinguishes:
    /// - **Cloud-provider network fault** — TCP RST with no SCP error
    ///   (firewall block, cloud provider ACL, or route black-hole).
    /// - **Stellar protocol fault** — handshake fails after TCP established
    ///   (TLS mismatch, incompatible SCP version, wrong network passphrase).
    async fn emit_drop_event(&self, peer: &PeerNetworkStats) -> anyhow::Result<()> {
        let events: Api<Event> = Api::namespaced(self.client.clone(), &self.namespace);
        let now = Utc::now();

        // Classify the fault type.
        let (reason, fault_class, recommendation) = if peer.handshake_fail_total > 0 {
            (
                "ScpHandshakeFail",
                "Stellar protocol fault (handshake failure)",
                "Check TLS certificates, SCP network passphrase, and stellar-core version \
                 compatibility with the remote peer.",
            )
        } else {
            (
                "ScpPeerDrop",
                "Cloud-provider network fault (TCP RST without SCP error)",
                "Verify firewall rules, security group ACLs, and cloud-provider network \
                 policies for outbound TCP port 11625 to the affected peer IP.",
            )
        };

        let message = format!(
            "eBPF sniffer detected SCP connection drops to peer {peer_ip}: \
             {rst} TCP RST(s), {hf} handshake failure(s), drop rate {dr:.1}%. \
             Classification: {fault_class}. Recommendation: {recommendation}",
            peer_ip    = peer.peer_ip,
            rst        = peer.rst_total,
            hf         = peer.handshake_fail_total,
            dr         = peer.drop_rate_pct,
            fault_class = fault_class,
            recommendation = recommendation,
        );

        let event = Event {
            metadata: ObjectMeta {
                generate_name: Some(format!("{}-scp-drop-", self.node_name)),
                namespace: Some(self.namespace.clone()),
                ..ObjectMeta::default()
            },
            involved_object: ObjectReference {
                kind: Some("StellarNode".to_string()),
                name: Some(self.node_name.clone()),
                namespace: Some(self.namespace.clone()),
                api_version: Some("stellar.org/v1alpha1".to_string()),
                ..ObjectReference::default()
            },
            reason: Some(reason.to_string()),
            message: Some(message),
            type_: Some("Warning".to_string()),
            first_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(now)),
            last_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(now)),
            ..Event::default()
        };

        events.create(&PostParams::default(), &event).await
            .map_err(|e| anyhow::anyhow!("K8s Event create failed: {e}"))?;

        info!(
            node     = %self.node_name,
            peer_ip  = %peer.peer_ip,
            rst      = peer.rst_total,
            reason   = reason,
            "SCP sniffer: emitted K8s drop event"
        );
        Ok(())
    }
}

// ─── Prometheus text parser ───────────────────────────────────────────────────

/// Parse a subset of Prometheus text exposition format to extract the SCP
/// sniffer counters we care about.
///
/// Only `scp_sniffer_*` metric families are processed; all others are ignored.
/// This is intentionally minimal — we only need the counters that feed the
/// store and the K8s Event emission logic.
fn parse_prometheus_text(text: &str) -> ScpNetworkHealthState {
    let mut state = ScpNetworkHealthState {
        last_scraped: Utc::now().to_rfc3339(),
        sniffer_available: true,
        ..Default::default()
    };

    let mut peers: HashMap<String, PeerNetworkStats> = HashMap::new();

    for line in text.lines() {
        // Skip comments and empty lines.
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }

        // Parse health score gauge (no labels).
        if let Some(val) = parse_labelless_metric(line, "scp_sniffer_health_score") {
            state.health_score = val.min(100.0) as u8;
            continue;
        }

        // Parse per-peer counters with {peer_ip="..."} label.
        if let Some((metric, peer_ip, value)) = parse_peer_metric(line) {
            let entry = peers
                .entry(peer_ip.clone())
                .or_insert_with(|| PeerNetworkStats {
                    peer_ip: peer_ip.clone(),
                    last_updated: state.last_scraped.clone(),
                    ..Default::default()
                });

            match metric {
                "scp_sniffer_rst_total" => entry.rst_total = value as u64,
                "scp_sniffer_handshake_fail_total" => {
                    entry.handshake_fail_total = value as u64;
                }
                "scp_sniffer_retransmit_total" => entry.retransmit_total = value as u64,
                "scp_sniffer_events_total" => {
                    // event_type="scp_inbound" label presence means RX traffic.
                    if line.contains("event_type=\"scp_inbound\"") {
                        entry.packets_rx = value as u64;
                    }
                }
                _ => {}
            }
        }
    }

    // Compute derived fields for each peer.
    let now = state.last_scraped.clone();
    for entry in peers.values_mut() {
        let denom = entry.packets_rx.max(1) as f64;
        entry.drop_rate_pct = (entry.rst_total as f64 / denom) * 100.0;
        entry.is_degraded = entry.drop_rate_pct > DEGRADED_DROP_RATE_PCT
            || entry.handshake_fail_total > 0;
        entry.last_updated = now.clone();
    }

    state.total_peers = peers.len();
    state.degraded_peers = peers.values().filter(|p| p.is_degraded).count();
    state.peers = peers;

    state
}

/// Parse a metric line with no labels: `metric_name value`.
fn parse_labelless_metric(line: &str, name: &str) -> Option<f64> {
    let line = line.trim();
    if !line.starts_with(name) {
        return None;
    }
    // Ensure we match the full metric name (no substring match).
    let rest = &line[name.len()..];
    if !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    rest.trim().parse::<f64>().ok()
}

/// Parse a metric line with `peer_ip` label:
/// `metric_name{peer_ip="1.2.3.4",...} value`
///
/// Returns `(metric_base_name, peer_ip, value)`.
fn parse_peer_metric(line: &str) -> Option<(&str, String, f64)> {
    // Split on the first '{'.
    let brace = line.find('{')?;
    let metric_name = line[..brace].trim();

    // Only process scp_sniffer_ metrics.
    if !metric_name.starts_with("scp_sniffer_") {
        return None;
    }

    // Extract peer_ip label value.
    let labels_end = line.find('}')?;
    let labels_str = &line[brace + 1..labels_end];
    let peer_ip = extract_label_value(labels_str, "peer_ip")?;

    // Value is the token after the closing brace.
    let after_brace = line[labels_end + 1..].trim();
    // May have a timestamp after the value; take only the first token.
    let value_str = after_brace.split_whitespace().next()?;
    let value = value_str.parse::<f64>().ok()?;

    Some((metric_name, peer_ip, value))
}

/// Extract a specific label value from a Prometheus labels string.
/// E.g. `extract_label_value("peer_ip=\"1.2.3.4\",event_type=\"tcp_rst\"", "peer_ip")`
/// returns `Some("1.2.3.4")`.
fn extract_label_value(labels: &str, key: &str) -> Option<String> {
    let search = format!("{key}=\"");
    let start = labels.find(&search)? + search.len();
    let end = labels[start..].find('"')? + start;
    Some(labels[start..end].to_owned())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PROM: &str = r#"
# HELP scp_sniffer_health_score Overall SCP network health score
# TYPE scp_sniffer_health_score gauge
scp_sniffer_health_score 85
# HELP scp_sniffer_rst_total Total TCP RST packets on SCP connections
# TYPE scp_sniffer_rst_total counter
scp_sniffer_rst_total{peer_ip="10.0.1.13"} 12
# HELP scp_sniffer_handshake_fail_total Total SCP TCP handshake failures
# TYPE scp_sniffer_handshake_fail_total counter
scp_sniffer_handshake_fail_total{peer_ip="10.0.1.13"} 3
scp_sniffer_handshake_fail_total{peer_ip="10.0.1.10"} 0
# HELP scp_sniffer_retransmit_total Total TCP retransmit events on SCP connections
# TYPE scp_sniffer_retransmit_total counter
scp_sniffer_retransmit_total{peer_ip="10.0.1.11"} 5
# HELP scp_sniffer_events_total Total SCP events observed
# TYPE scp_sniffer_events_total counter
scp_sniffer_events_total{peer_ip="10.0.1.10",event_type="scp_inbound"} 200
scp_sniffer_events_total{peer_ip="10.0.1.13",event_type="scp_inbound"} 50
scp_sniffer_events_total{peer_ip="10.0.1.13",event_type="tcp_rst"} 12
"#;

    #[test]
    fn parse_health_score() {
        let state = parse_prometheus_text(SAMPLE_PROM);
        assert_eq!(state.health_score, 85);
        assert!(state.sniffer_available);
    }

    #[test]
    fn parse_peer_rst() {
        let state = parse_prometheus_text(SAMPLE_PROM);
        let peer = state.peers.get("10.0.1.13").expect("peer 10.0.1.13");
        assert_eq!(peer.rst_total, 12);
        assert_eq!(peer.handshake_fail_total, 3);
        assert!(peer.is_degraded);
    }

    #[test]
    fn parse_peer_rx() {
        let state = parse_prometheus_text(SAMPLE_PROM);
        let peer = state.peers.get("10.0.1.10").expect("peer 10.0.1.10");
        assert_eq!(peer.packets_rx, 200);
    }

    #[test]
    fn drop_rate_computed() {
        let state = parse_prometheus_text(SAMPLE_PROM);
        let peer = state.peers.get("10.0.1.13").expect("peer 10.0.1.13");
        // rst=12, rx=50 → 24 %
        assert!((peer.drop_rate_pct - 24.0).abs() < 0.01);
    }

    #[test]
    fn degraded_peer_count() {
        let state = parse_prometheus_text(SAMPLE_PROM);
        // 10.0.1.13 has rst + handshake failures → degraded
        assert!(state.degraded_peers >= 1);
    }

    #[test]
    fn extract_label_value_basic() {
        let labels = r#"peer_ip="10.0.0.1",event_type="tcp_rst""#;
        assert_eq!(
            extract_label_value(labels, "peer_ip"),
            Some("10.0.0.1".into())
        );
        assert_eq!(
            extract_label_value(labels, "event_type"),
            Some("tcp_rst".into())
        );
        assert_eq!(extract_label_value(labels, "missing"), None);
    }

    #[test]
    fn labelless_metric_parse() {
        assert_eq!(
            parse_labelless_metric("scp_sniffer_health_score 72", "scp_sniffer_health_score"),
            Some(72.0)
        );
        // No partial name match
        assert_eq!(
            parse_labelless_metric(
                "scp_sniffer_health_score_extra 1",
                "scp_sniffer_health_score"
            ),
            None
        );
    }
}
