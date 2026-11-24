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

//! Firewall Engine — orchestrates the eBPF interceptor, XDR analyzer,
//! blacklist manager, and Prometheus metrics server.
//!
//! The engine runs four concurrent async tasks:
//!
//! 1. **eBPF packet interceptor** — binds to the SCP port, captures packet
//!    metadata, and sends events on an internal channel.
//! 2. **Analysis loop** — reads from the channel, calls [`XdrAnalyzer`],
//!    and routes high-severity results to the blacklist manager.
//! 3. **Expiry pruner** — periodically removes expired blacklist entries and
//!    updates Kubernetes NetworkPolicy.
//! 4. **Metrics server** — serves the Prometheus `/metrics` endpoint.

use crate::{
    analyzer::{AnalyzerConfig, ThreatLevel, XdrAnalyzer},
    blacklist::{BlacklistManager, BlacklistReason},
    ebpf::PacketInterceptor,
    metrics::FirewallMetrics,
};
use anyhow::Result;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Firewall configuration, populated from CLI args.
#[derive(Debug, Clone)]
pub struct FirewallConfig {
    /// TCP port for SCP traffic (default 11625).
    pub scp_port: u16,
    /// Packets-per-second per IP that triggers CRITICAL flood verdict.
    pub flood_pps_threshold: u32,
    /// Seconds a blacklisted IP stays blocked.
    pub blacklist_duration_secs: u64,
    /// Kubernetes namespace for NetworkPolicy enforcement.
    pub namespace: String,
    /// Prometheus metrics bind address.
    pub metrics_addr: String,
    /// Malformed-payload count before HIGH threat escalation.
    pub malformed_threshold: u32,
    /// Rapid-handshake-failure count before HIGH threat escalation.
    pub handshake_fail_threshold: u32,
    /// Analysis sliding window duration (seconds).
    pub window_secs: u64,
}

impl Default for FirewallConfig {
    fn default() -> Self {
        Self {
            scp_port: 11625,
            flood_pps_threshold: 100,
            blacklist_duration_secs: 300,
            namespace: "stellar".into(),
            metrics_addr: "0.0.0.0:9090".into(),
            malformed_threshold: 10,
            handshake_fail_threshold: 5,
            window_secs: 10,
        }
    }
}

/// Internal channel buffer size.
const CHANNEL_BUFFER: usize = 8192;

/// Blacklist expiry pruning interval.
const PRUNE_INTERVAL_SECS: u64 = 30;

/// Main firewall engine.
pub struct FirewallEngine {
    config: FirewallConfig,
    metrics: FirewallMetrics,
    analyzer: XdrAnalyzer,
    blacklist: BlacklistManager,
}

impl FirewallEngine {
    /// Build the engine.  Attempts to create a Kubernetes client from the
    /// in-cluster service account (or `KUBECONFIG` / `~/.kube/config`).
    /// If no Kubernetes access is available, NetworkPolicy enforcement is
    /// disabled but the firewall continues operating.
    pub async fn new(config: FirewallConfig) -> Result<Self> {
        let kube_client = match kube::Client::try_default().await {
            Ok(c) => {
                info!("Kubernetes client initialised — NetworkPolicy enforcement enabled");
                Some(c)
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "Kubernetes client unavailable — NetworkPolicy enforcement disabled"
                );
                None
            }
        };

        let analyzer_config = AnalyzerConfig {
            flood_pps_threshold: config.flood_pps_threshold as f64,
            malformed_threshold: config.malformed_threshold,
            handshake_fail_threshold: config.handshake_fail_threshold,
            window: Duration::from_secs(config.window_secs),
        };

        let analyzer = XdrAnalyzer::new(analyzer_config);
        let metrics = FirewallMetrics::new();
        let blacklist = BlacklistManager::new(
            kube_client,
            config.namespace.clone(),
            config.blacklist_duration_secs,
            config.scp_port,
            false, // iptables opt-in; set to true if NET_ADMIN is available
        );

        Ok(Self {
            config,
            metrics,
            analyzer,
            blacklist,
        })
    }

    /// Start the firewall engine.  Runs until a fatal error occurs.
    pub async fn run(self) -> Result<()> {
        let (tx, rx) = mpsc::channel(CHANNEL_BUFFER);

        // Task 1: eBPF / userspace packet interceptor.
        let interceptor = PacketInterceptor::new(self.config.scp_port, tx);
        let interceptor_handle = tokio::spawn(async move {
            if let Err(e) = interceptor.run().await {
                warn!(error = %e, "Packet interceptor exited");
            }
        });

        // Task 2: Analysis loop.
        let analyzer_handle = {
            let analyzer = self.analyzer.clone();
            let blacklist = self.blacklist.clone();
            let metrics = self.metrics.clone();
            tokio::spawn(analysis_loop(analyzer, blacklist, metrics, rx))
        };

        // Task 3: Expiry pruner.
        let pruner_handle = {
            let blacklist = self.blacklist.clone();
            let metrics = self.metrics.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(PRUNE_INTERVAL_SECS));
                loop {
                    interval.tick().await;
                    match blacklist.prune_expired().await {
                        Ok(pruned) => {
                            if pruned > 0 {
                                info!(pruned, "Expired blacklist entries pruned");
                            }
                            metrics.set_blacklisted_ips(blacklist.active_count() as i64);
                        }
                        Err(e) => warn!(error = %e, "Blacklist pruner error"),
                    }
                }
            })
        };

        // Task 4: Metrics server.
        let metrics_handle = {
            let metrics = self.metrics.clone();
            let addr = self.config.metrics_addr.clone();
            tokio::spawn(async move {
                if let Err(e) = metrics.serve(addr).await {
                    warn!(error = %e, "Metrics server exited");
                }
            })
        };

        info!("Firewall engine started — all tasks running");

        // Wait for the first task to exit (they should run indefinitely).
        tokio::select! {
            _ = interceptor_handle => warn!("Packet interceptor task exited"),
            _ = analyzer_handle    => warn!("Analysis loop task exited"),
            _ = pruner_handle      => warn!("Expiry pruner task exited"),
            _ = metrics_handle     => warn!("Metrics server task exited"),
        }

        Ok(())
    }
}

/// The core analysis loop: reads packets from the channel, analyzes them, and
/// routes high-severity results to the blacklist manager.
async fn analysis_loop(
    analyzer: XdrAnalyzer,
    blacklist: BlacklistManager,
    metrics: FirewallMetrics,
    mut rx: mpsc::Receiver<crate::ebpf::RawPacket>,
) {
    info!("Analysis loop started");

    while let Some(packet) = rx.recv().await {
        let src_ip = packet.src_addr.ip();
        let start = Instant::now();

        // Skip already-blacklisted IPs immediately (sub-microsecond check).
        if blacklist.is_blacklisted(src_ip) {
            continue;
        }

        let result = analyzer.analyze(&packet);
        let elapsed = start.elapsed().as_secs_f64();

        metrics.record_packet();
        metrics.record_analysis_duration(elapsed);

        if result.threat_level > ThreatLevel::None {
            metrics.record_threat(&result.threat_level.to_string());
        }

        if result.threat_level == ThreatLevel::Critical {
            metrics.record_flood_event();
        }

        if result.malformed_count > 0 {
            metrics.record_malformed_packet();
        }

        if result.handshake_fail_count > 0 {
            metrics.record_handshake_failure();
        }

        // Blacklist on HIGH or CRITICAL.
        if result.threat_level >= ThreatLevel::High {
            let reason = if result.threat_level == ThreatLevel::Critical {
                BlacklistReason::Flood
            } else if result.handshake_fail_count > 0 {
                BlacklistReason::HandshakeFailure
            } else {
                BlacklistReason::MalformedPayload
            };

            match blacklist
                .blacklist(src_ip, reason, result.threat_level, 0)
                .await
            {
                Ok(true) => {
                    metrics.record_blacklist_event("added");
                    metrics.set_blacklisted_ips(blacklist.active_count() as i64);
                    metrics.record_network_policy_update();
                }
                Ok(false) => {} // already blacklisted
                Err(e) => warn!(error = %e, %src_ip, "Failed to blacklist IP"),
            }
        }
    }

    info!("Analysis loop: channel closed, exiting");
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::XdrAnalyzer;
    use crate::blacklist::BlacklistManager;
    use crate::ebpf::RawPacket;
    use crate::metrics::FirewallMetrics;
    use bytes::Bytes;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use tokio::sync::mpsc;

    fn make_valid_xdr_bytes(discriminant: u32, len: u32) -> Bytes {
        let mut v = Vec::new();
        v.extend_from_slice(&(0x8000_0000u32 | len).to_be_bytes());
        v.extend_from_slice(&discriminant.to_be_bytes());
        v.extend(vec![0u8; len as usize]);
        Bytes::from(v)
    }

    #[tokio::test]
    async fn test_analysis_loop_clean_traffic() {
        let (tx, rx) = mpsc::channel(64);

        let analyzer = XdrAnalyzer::new(AnalyzerConfig::default());
        let blacklist = BlacklistManager::new(None, "stellar", 60, 11625, false);
        let metrics = FirewallMetrics::new();

        let blacklist_clone = blacklist.clone();
        tokio::spawn(analysis_loop(analyzer, blacklist_clone, metrics, rx));

        // Send clean SCP TX packets.
        let payload = make_valid_xdr_bytes(5, 64);
        for i in 0..5u8 {
            let pkt = RawPacket {
                src_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, i)), 50000),
                dst_port: 11625,
                payload: payload.clone(),
                captured_at_ns: 0,
                truncated: false,
            };
            tx.send(pkt).await.unwrap();
        }
        drop(tx);

        // Brief wait; blacklist should be empty.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(blacklist.active_count(), 0);
    }

    #[tokio::test]
    async fn test_analysis_loop_flood_triggers_blacklist() {
        let (tx, rx) = mpsc::channel(1024);

        let analyzer = XdrAnalyzer::new(AnalyzerConfig {
            flood_pps_threshold: 5.0,
            window: Duration::from_secs(10),
            ..Default::default()
        });
        let blacklist = BlacklistManager::new(None, "stellar", 60, 11625, false);
        let metrics = FirewallMetrics::new();

        let blacklist_clone = blacklist.clone();
        tokio::spawn(analysis_loop(analyzer, blacklist_clone, metrics, rx));

        let src = IpAddr::V4(Ipv4Addr::new(192, 168, 99, 99));
        let payload = make_valid_xdr_bytes(5, 64);

        // Flood with 200 packets from the same IP.
        for _ in 0..200 {
            let pkt = RawPacket {
                src_addr: SocketAddr::new(src, 9999),
                dst_port: 11625,
                payload: payload.clone(),
                captured_at_ns: 0,
                truncated: false,
            };
            tx.send(pkt).await.unwrap();
        }

        // Allow the async loop time to process.
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(
            blacklist.is_blacklisted(src),
            "Flooding IP should be blacklisted"
        );
    }

    #[test]
    fn test_firewall_config_default() {
        let cfg = FirewallConfig::default();
        assert_eq!(cfg.scp_port, 11625);
        assert_eq!(cfg.flood_pps_threshold, 100);
        assert_eq!(cfg.blacklist_duration_secs, 300);
    }
}
