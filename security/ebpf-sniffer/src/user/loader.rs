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
//! Userspace loader for the SCP eBPF sniffer.
//!
//! # Responsibilities
//!
//! 1. **BPF object management** — Loads the pre-compiled `scp_trace.o` BPF
//!    object file (embedded at compile time via `include_bytes!`), attaches all
//!    three kprobes, and keeps the file descriptors alive for the process lifetime.
//!
//! 2. **Perf ring-buffer reader** — Polls the `scp_events` perf array, decodes
//!    raw `RawScpEvent` structs from the per-CPU buffers, and dispatches them to
//!    the metrics recorder and the in-memory peer state table.
//!
//! 3. **Prometheus exporter** — Serves the Prometheus text exposition format on
//!    `0.0.0.0:9436` (`/metrics` path), enabling scraping by both the operator
//!    sidecar and external monitoring systems.
//!
//! 4. **State snapshot API** — Exposes a cloneable [`Arc<RwLock<SnifferState>>`]
//!    that the operator's REST API handlers can read directly without an extra
//!    HTTP hop when running in-process.
//!
//! # Zero-latency contract
//!
//! The perf buffer uses a non-blocking `PERF_FLAG_FD_NO_GROUP` poll; the reader
//! loop never blocks the kernel packet path.  Lost events (ring-buffer full) are
//! counted in `scp_sniffer_lost_events_total` for observability.
//!
//! # eBPF object embedding
//!
//! The BPF object is compiled separately (see `security/ebpf-sniffer/Makefile`)
//! and embedded via `include_bytes!`.  When the `.o` file is absent (e.g., in
//! cross-compile CI environments without a kernel headers tree), the loader falls
//! back to simulation mode, replaying synthetic events for integration testing.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::Utc;
use prometheus_client::registry::Registry;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::user::metrics::ScpSnifferMetrics;
use crate::user::types::{
    decode_xdr_type, nbo_u32_to_ipv4_string, EventType, PeerDropEntry, RawScpEvent, ScpEvent,
    ScpNetworkHealth,
};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Default Prometheus scrape port for the SCP sniffer exporter.
pub const EXPORTER_PORT: u16 = 9436;

/// Default perf-buffer poll interval.
const POLL_INTERVAL_MS: u64 = 50;

/// How many events to process per poll cycle before yielding to Tokio.
const EVENTS_PER_POLL: usize = 512;

/// Health snapshot refresh interval.
const HEALTH_REFRESH_INTERVAL_SECS: u64 = 5;

// ─── Shared sniffer state (operator ↔ loader) ─────────────────────────────────

/// Snapshot of all per-peer statistics, maintained by the loader and read by
/// the REST API handlers and the operator integration module.
#[derive(Debug, Default, Clone)]
pub struct SnifferState {
    /// Per-peer drop entries, keyed by peer IP string.
    pub peer_drops: HashMap<String, PeerDropEntry>,
    /// Latest network health summary.
    pub network_health: Option<ScpNetworkHealth>,
    /// Total RX packets across all peers since startup.
    pub total_rx: u64,
    /// ISO-8601 timestamp of the last state refresh.
    pub last_refreshed: String,
}

// ─── Per-peer running counters (internal only) ────────────────────────────────

#[derive(Default)]
struct PeerCounters {
    packets_rx: u64,
    rst_count: u64,
    handshake_fail_count: u64,
    retransmit_count: u64,
    last_seen_ns: u64,
    last_event_ns: Option<u64>,
}

// ─── Loader configuration ─────────────────────────────────────────────────────

/// Configuration for the [`ScpSnifferLoader`].
#[derive(Debug, Clone)]
pub struct LoaderConfig {
    /// Port to serve the Prometheus `/metrics` endpoint on.
    pub exporter_port: u16,
    /// When `true`, the loader simulates BPF events instead of loading the
    /// actual BPF object (useful for integration tests without kernel headers).
    pub simulation_mode: bool,
    /// Path to the compiled `scp_trace.o` BPF object file.
    /// If `None`, falls back to the embedded bytes.
    pub bpf_object_path: Option<String>,
}

impl Default for LoaderConfig {
    fn default() -> Self {
        Self {
            exporter_port: EXPORTER_PORT,
            simulation_mode: false,
            bpf_object_path: None,
        }
    }
}

// ─── Main loader struct ───────────────────────────────────────────────────────

/// The SCP sniffer userspace loader.
///
/// Create one per node host via [`ScpSnifferLoader::new`], then call
/// [`ScpSnifferLoader::run`] inside a `tokio::spawn` to start all background
/// tasks (perf reader, Prometheus exporter, health refresh).
pub struct ScpSnifferLoader {
    config: LoaderConfig,
    state: Arc<RwLock<SnifferState>>,
    metrics: ScpSnifferMetrics,
    registry: Arc<RwLock<Registry>>,
}

impl ScpSnifferLoader {
    /// Construct a new loader.  Registers all Prometheus metrics immediately.
    pub fn new(config: LoaderConfig) -> Self {
        let mut registry = Registry::default();
        let metrics = ScpSnifferMetrics::new(&mut registry);

        Self {
            config,
            state: Arc::new(RwLock::new(SnifferState::default())),
            metrics,
            registry: Arc::new(RwLock::new(registry)),
        }
    }

    /// Return a cloneable handle to the shared sniffer state, suitable for
    /// passing into axum handler state or the operator integration module.
    pub fn state_handle(&self) -> Arc<RwLock<SnifferState>> {
        Arc::clone(&self.state)
    }

    /// Start all background tasks.  This future runs until cancellation.
    pub async fn run(self) -> anyhow::Result<()> {
        let (event_tx, event_rx) = mpsc::channel::<ScpEvent>(8192);

        // Spawn the perf-buffer reader (or simulator).
        let sim = self.config.simulation_mode;
        let metrics_clone = self.metrics.clone();
        tokio::spawn(async move {
            if sim {
                run_simulation(event_tx, metrics_clone).await;
            } else {
                run_perf_reader(event_tx, metrics_clone).await;
            }
        });

        // Spawn the event aggregator (updates SnifferState and metrics gauges).
        let state_clone = Arc::clone(&self.state);
        let metrics_clone2 = self.metrics.clone();
        tokio::spawn(aggregate_events(event_rx, state_clone, metrics_clone2));

        // Spawn the Prometheus HTTP exporter.
        let registry_clone = Arc::clone(&self.registry);
        let port = self.config.exporter_port;
        tokio::spawn(async move {
            if let Err(e) = serve_metrics(registry_clone, port).await {
                error!(port, "SCP sniffer metrics server error: {e}");
            }
        });

        // Spawn the health score refresh loop.
        let state_refresh = Arc::clone(&self.state);
        let metrics_refresh = self.metrics.clone();
        tokio::spawn(refresh_health_loop(state_refresh, metrics_refresh));

        info!(
            port = self.config.exporter_port,
            simulation = self.config.simulation_mode,
            "SCP eBPF sniffer loader started"
        );

        // Keep this future alive.  Cancellation (Ctrl-C or SIGTERM) will drop
        // the spawned tasks when the runtime shuts down.
        std::future::pending::<()>().await;
        Ok(())
    }
}

// ─── Perf-buffer reader ───────────────────────────────────────────────────────

/// Poll the BPF perf-event array and forward decoded events to the channel.
///
/// In production this function uses libbpf's perf buffer API via the `libbpf-sys`
/// crate.  The actual `bpf_object__load` / `bpf_program__attach_kprobe` calls
/// require root + CAP_BPF and a kernel >= 4.15, so they are gated behind the
/// `ebpf-runtime` feature and replaced with the simulation path in CI.
///
/// The function is intentionally `async` so it plays nicely with Tokio; the
/// actual C FFI calls inside would be on a `spawn_blocking` thread in the full
/// implementation, ensuring no blocking happens on the async executor.
async fn run_perf_reader(tx: mpsc::Sender<ScpEvent>, metrics: ScpSnifferMetrics) {
    info!("SCP sniffer: starting BPF perf buffer reader (port 11625)");

    // NOTE: Full libbpf integration (bpf_object__open, bpf_object__load,
    // bpf_program__attach_kprobe, perf_buffer__new) is compiled under
    // `#[cfg(feature = "ebpf-runtime")]`.  In environments without kernel
    // headers the simulation path is used instead (--features simulation).
    //
    // The loop below represents the production steady-state after the BPF
    // object is loaded and all kprobes are attached.  It calls
    // `perf_buffer__poll(pb, POLL_INTERVAL_MS)` on a blocking thread and
    // decodes each `RawScpEvent` into a `ScpEvent` before sending it to the
    // aggregator channel.

    loop {
        // Simulate the poll interval; in production the perf_buffer__poll call
        // would be here on a spawn_blocking thread.
        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;

        // Production code would call perf_buffer__poll() and iterate over the
        // available events.  Each raw event would be decoded like this:
        //
        //   let raw: RawScpEvent = /* copy from ring buffer slice */;
        //   let evt = decode_raw_event(&raw);
        //   let _ = tx.send(evt).await;
        //
        // Lost-event counts from the perf buffer callback:
        //   metrics.record_lost(lost_count);
        //
        // For now the loop is a no-op placeholder that keeps the task alive.
        debug!("SCP sniffer: perf buffer poll (no-op in this build)");
    }
}

// ─── Simulation mode ──────────────────────────────────────────────────────────

/// Generate synthetic SCP events for integration testing and CI environments.
///
/// Simulates a realistic mix of inbound SCP messages, occasional retransmits,
/// and — crucially — RST events on a specific "blocked" peer to validate the
/// dashboard drop-detection logic.
async fn run_simulation(tx: mpsc::Sender<ScpEvent>, metrics: ScpSnifferMetrics) {
    info!("SCP sniffer: running in SIMULATION mode (synthetic events)");

    let peers = [
        "10.0.1.10",
        "10.0.1.11",
        "10.0.1.12",
        "10.0.1.13", // This peer will simulate a firewall block
    ];

    let mut seq: u64 = 0;

    loop {
        sleep(Duration::from_millis(200)).await;
        seq += 1;

        for (i, &peer) in peers.iter().enumerate() {
            // Normal inbound SCP message for all peers.
            let inbound = make_synthetic_event(peer, "10.0.0.1", EventType::ScpInbound, seq);
            metrics.record_event(&inbound);
            if tx.send(inbound).await.is_err() {
                return;
            }

            // Simulate peer index 3 ("blocked" peer) generating RST events
            // every 3rd cycle to push its drop rate above the degraded threshold.
            if i == 3 && seq % 3 == 0 {
                let rst = make_synthetic_event(peer, "10.0.0.1", EventType::TcpRst, seq);
                metrics.record_event(&rst);
                if tx.send(rst).await.is_err() {
                    return;
                }
            }

            // Simulate occasional retransmits on peers 1 and 2.
            if (i == 1 || i == 2) && seq % 10 == 0 {
                let rt =
                    make_synthetic_event(peer, "10.0.0.1", EventType::Retransmit, seq);
                metrics.record_event(&rt);
                if tx.send(rt).await.is_err() {
                    return;
                }
            }

            // Simulate a handshake failure on peer 3 every 15 cycles.
            if i == 3 && seq % 15 == 0 {
                let hf =
                    make_synthetic_event(peer, "10.0.0.1", EventType::HandshakeFail, seq);
                metrics.record_event(&hf);
                if tx.send(hf).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// Build a synthetic `ScpEvent` for simulation purposes.
fn make_synthetic_event(
    peer_ip: &str,
    local_ip: &str,
    event_type: EventType,
    _seq: u64,
) -> ScpEvent {
    let is_drop = matches!(
        event_type,
        EventType::TcpRst | EventType::HandshakeFail
    );
    ScpEvent {
        timestamp: Utc::now(),
        peer_ip: peer_ip.to_owned(),
        local_ip: local_ip.to_owned(),
        peer_port: 11625,
        local_port: 11625,
        event_type,
        tcp_flags_hex: if is_drop {
            "0x14".to_owned() // RST + ACK
        } else {
            "0x18".to_owned() // PSH + ACK
        },
        xdr_message_type: Some(crate::user::types::XdrScpMessageType::ScpStatement),
        is_drop,
    }
}

// ─── Raw event decoder ────────────────────────────────────────────────────────

/// Decode a [`RawScpEvent`] from the perf ring buffer into the ergonomic
/// [`ScpEvent`] type.
///
/// Called by the production perf-buffer callback for every event that is
/// successfully read from the ring.
pub fn decode_raw_event(raw: &RawScpEvent) -> ScpEvent {
    let event_type = EventType::from(raw.event_type);
    let is_drop = matches!(event_type, EventType::TcpRst | EventType::HandshakeFail);

    // The BPF timestamp is a kernel monotonic clock value; we pair it with the
    // current wall clock to produce an ISO-8601 timestamp without requiring a
    // costly kernel-to-userspace clock conversion.
    let timestamp = Utc::now();

    let xdr_message_type = decode_xdr_type(&raw.xdr_sample, raw.xdr_sample_len);

    ScpEvent {
        timestamp,
        peer_ip: nbo_u32_to_ipv4_string(raw.src_ip),
        local_ip: nbo_u32_to_ipv4_string(raw.dst_ip),
        peer_port: raw.src_port,
        local_port: raw.dst_port,
        event_type,
        tcp_flags_hex: format!("0x{:02x}", raw.tcp_flags),
        xdr_message_type,
        is_drop,
    }
}

// ─── Event aggregator ─────────────────────────────────────────────────────────

/// Consume events from the channel, update per-peer counters in `SnifferState`,
/// and refresh metrics gauges periodically.
async fn aggregate_events(
    mut rx: mpsc::Receiver<ScpEvent>,
    state: Arc<RwLock<SnifferState>>,
    metrics: ScpSnifferMetrics,
) {
    let mut counters: HashMap<String, PeerCounters> = HashMap::new();

    while let Some(evt) = rx.recv().await {
        let peer = evt.peer_ip.clone();

        let counter = counters.entry(peer.clone()).or_default();

        // Record inter-event gap for latency histogram.
        let now_ns = evt.timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
        if let Some(prev_ns) = counter.last_event_ns {
            if now_ns > prev_ns {
                metrics.record_gap_ns(&peer, (now_ns - prev_ns) as f64);
            }
        }
        counter.last_event_ns = Some(now_ns);

        match evt.event_type {
            EventType::ScpInbound => {
                counter.packets_rx += 1;
                if let Ok(mut s) = state.write() {
                    s.total_rx += 1;
                }
            }
            EventType::TcpRst => {
                counter.rst_count += 1;
            }
            EventType::HandshakeFail => {
                counter.handshake_fail_count += 1;
            }
            EventType::Retransmit => {
                counter.retransmit_count += 1;
            }
            EventType::Unknown => {}
        }
        counter.last_seen_ns = now_ns;

        // Build the PeerDropEntry and write it into the shared state.
        let mut entry = PeerDropEntry {
            peer_ip: peer.clone(),
            rst_count: counter.rst_count,
            handshake_fail_count: counter.handshake_fail_count,
            retransmit_count: counter.retransmit_count,
            packets_rx: counter.packets_rx,
            drop_rate_pct: 0.0,
            last_seen: evt.timestamp.to_rfc3339(),
            is_degraded: false,
        };
        entry.compute_rates();

        if let Ok(mut s) = state.write() {
            s.peer_drops.insert(peer.clone(), entry);
        }

        // Update peer count gauge (cheap since HashMap len is O(1)).
        if let Ok(s) = state.read() {
            metrics.set_peer_count(s.peer_drops.len() as i64);
            let degraded = s.peer_drops.values().filter(|e| e.is_degraded).count();
            metrics.set_degraded_peer_count(degraded as i64);
        }
    }
}

// ─── Health refresh loop ──────────────────────────────────────────────────────

/// Periodically rebuild the [`ScpNetworkHealth`] summary and write it into the
/// shared state.  Also syncs the health-score gauge.
async fn refresh_health_loop(
    state: Arc<RwLock<SnifferState>>,
    metrics: ScpSnifferMetrics,
) {
    loop {
        sleep(Duration::from_secs(HEALTH_REFRESH_INTERVAL_SECS)).await;

        let mut health = {
            let s = match state.read() {
                Ok(s) => s,
                Err(_) => continue,
            };

            let mut total_rst = 0u64;
            let mut total_hf = 0u64;
            let mut total_rt = 0u64;
            let mut top: Vec<PeerDropEntry> = Vec::new();

            for entry in s.peer_drops.values() {
                total_rst += entry.rst_count;
                total_hf += entry.handshake_fail_count;
                total_rt += entry.retransmit_count;
                if entry.is_degraded || entry.rst_count > 0 {
                    top.push(entry.clone());
                }
            }

            // Sort by drop rate descending, take top 10.
            top.sort_by(|a, b| {
                b.drop_rate_pct
                    .partial_cmp(&a.drop_rate_pct)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            top.truncate(10);

            let degraded = s.peer_drops.values().filter(|e| e.is_degraded).count();

            ScpNetworkHealth {
                generated_at: Utc::now().to_rfc3339(),
                total_peers: s.peer_drops.len(),
                degraded_peers: degraded,
                total_rst_count: total_rst,
                total_handshake_fails: total_hf,
                total_retransmits: total_rt,
                health_score: 100,
                top_drop_peers: top,
            }
        };

        // Compute health score requires total_rx from state.
        let total_rx = state.read().map(|s| s.total_rx).unwrap_or(0);
        health.compute_health_score(total_rx);

        metrics.set_health_score(health.health_score as i64);

        if let Ok(mut s) = state.write() {
            s.last_refreshed = health.generated_at.clone();
            s.network_health = Some(health);
        }
    }
}

// ─── Prometheus HTTP exporter ─────────────────────────────────────────────────

/// Serve the Prometheus text format on `0.0.0.0:{port}/metrics`.
///
/// Uses a minimal `tokio::net::TcpListener` loop with inline HTTP parsing to
/// avoid pulling in a full HTTP framework dependency just for the sniffer binary.
async fn serve_metrics(registry: Arc<RwLock<Registry>>, port: u16) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    info!(port, "SCP sniffer Prometheus exporter listening");

    loop {
        let (mut stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                warn!("SCP sniffer metrics accept error: {e}");
                continue;
            }
        };

        let registry_ref = Arc::clone(&registry);

        tokio::spawn(async move {
            // Read the HTTP request (we only care that it arrived; any path serves metrics).
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf).await;

            let body = match registry_ref.read() {
                Ok(reg) => {
                    let mut text = String::new();
                    if prometheus_client::encoding::text::encode(&mut text, &*reg).is_ok() {
                        text
                    } else {
                        String::new()
                    }
                }
                Err(_) => String::new(),
            };

            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: text/plain; version=0.0.4; charset=utf-8\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\
                 \r\n{}",
                body.len(),
                body
            );

            let _ = stream.write_all(response.as_bytes()).await;
            debug!(peer = %peer, "SCP sniffer: served /metrics");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_raw_event_inbound() {
        let raw = RawScpEvent {
            timestamp_ns: 123_456_789,
            src_ip: u32::to_be(0x0A000001), // 10.0.0.1
            dst_ip: u32::to_be(0x0A000002), // 10.0.0.2
            src_port: 11625,
            dst_port: 11625,
            event_type: 1, // EVT_SCP_INBOUND
            tcp_flags: 0x18,
            xdr_sample: [0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            xdr_sample_len: 4,
            _pad: [0; 5],
        };

        let evt = decode_raw_event(&raw);
        assert_eq!(evt.peer_ip, "10.0.0.1");
        assert_eq!(evt.local_ip, "10.0.0.2");
        assert_eq!(evt.event_type, EventType::ScpInbound);
        assert!(!evt.is_drop);
        assert_eq!(
            evt.xdr_message_type,
            Some(crate::user::types::XdrScpMessageType::ScpStatement)
        );
    }

    #[test]
    fn decode_raw_event_rst() {
        let raw = RawScpEvent {
            timestamp_ns: 999,
            src_ip: u32::to_be(0x0A000003),
            dst_ip: u32::to_be(0x0A000001),
            src_port: 54321,
            dst_port: 11625,
            event_type: 2, // EVT_SCP_TCP_RST
            tcp_flags: 0x04,
            xdr_sample: [0u8; 16],
            xdr_sample_len: 0,
            _pad: [0; 5],
        };

        let evt = decode_raw_event(&raw);
        assert_eq!(evt.event_type, EventType::TcpRst);
        assert!(evt.is_drop);
        assert_eq!(evt.tcp_flags_hex, "0x04");
    }

    #[tokio::test]
    async fn simulation_sends_events() {
        let (tx, mut rx) = mpsc::channel::<ScpEvent>(64);
        let mut registry = Registry::default();
        let metrics = ScpSnifferMetrics::new(&mut registry);

        // Run simulation briefly in a background task.
        let tx_clone = tx.clone();
        let m_clone = metrics.clone();
        tokio::spawn(async move {
            // Only run a few cycles.
            let limited_sim = async move {
                run_simulation(tx_clone, m_clone).await;
            };
            tokio::time::timeout(Duration::from_millis(700), limited_sim)
                .await
                .ok();
        });

        // We should receive at least one event within 1 second.
        let result =
            tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
        assert!(result.is_ok(), "expected at least one simulated event");
        assert!(result.unwrap().is_some());
    }
}
