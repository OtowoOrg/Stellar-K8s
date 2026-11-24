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

//! Integration tests for the P2P Gossip Threat Detection Firewall.
//!
//! These tests exercise the full pipeline from raw packet ingestion through
//! XDR analysis to IP blacklisting, validating:
//!
//! - Flood detection + blacklisting within 2 seconds (issue requirement).
//! - Malformed XDR payload detection and escalation.
//! - Rapid handshake failure detection.
//! - Blacklist entry expiry.
//! - Legitimate traffic is never blacklisted.

use bytes::Bytes;
use p2p_firewall::{
    analyzer::{AnalyzerConfig, XdrAnalyzer},
    blacklist::{BlacklistManager, BlacklistReason},
    ebpf::RawPacket,
    metrics::FirewallMetrics,
    ThreatLevel,
};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};

// ── Helpers ────────────────────────────────────────────────────────────────────

fn make_xdr(discriminant: u32, payload_len: u32) -> Bytes {
    let mut v: Vec<u8> = Vec::new();
    let raw_len: u32 = 0x8000_0000 | payload_len;
    v.extend_from_slice(&raw_len.to_be_bytes());
    v.extend_from_slice(&discriminant.to_be_bytes());
    v.extend(vec![0xABu8; payload_len as usize]);
    Bytes::from(v)
}

fn make_hello() -> Bytes {
    make_xdr(1, 64) // discriminant 1 = Hello
}

fn make_tx() -> Bytes {
    make_xdr(5, 128) // discriminant 5 = Transaction
}

fn malformed_payload() -> Bytes {
    // Only 3 bytes — too short for XDR header
    Bytes::from(vec![0xFF, 0x00, 0x00])
}

fn make_packet(ip: [u8; 4], port: u16, payload: Bytes) -> RawPacket {
    RawPacket {
        src_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port),
        dst_port: 11625,
        payload,
        captured_at_ns: 0,
        truncated: false,
    }
}

// ── Integration tests ──────────────────────────────────────────────────────────

/// Verify that a flood from a rogue IP is detected and blacklisted within 2 s.
#[tokio::test]
async fn test_flood_blacklisted_within_2s() {
    let config = AnalyzerConfig {
        flood_pps_threshold: 50.0,
        window: Duration::from_secs(5),
        malformed_threshold: 100,
        handshake_fail_threshold: 100,
    };
    let analyzer = XdrAnalyzer::new(config);
    let blacklist = BlacklistManager::new(None, "stellar", 60, 11625, false);

    let rogue_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));
    let src = SocketAddr::new(rogue_ip, 1234);
    let payload = make_tx();

    let start = Instant::now();
    let mut blacklisted_at = None;

    // Send packets until blacklisted or 2-second deadline exceeded.
    for _ in 0..500 {
        let pkt = RawPacket {
            src_addr: src,
            dst_port: 11625,
            payload: payload.clone(),
            captured_at_ns: 0,
            truncated: false,
        };

        let result = analyzer.analyze(&pkt);

        if result.threat_level >= ThreatLevel::High && blacklisted_at.is_none() {
            let _ = blacklist
                .blacklist(rogue_ip, BlacklistReason::Flood, result.threat_level, 0)
                .await;

            if blacklist.is_blacklisted(rogue_ip) {
                blacklisted_at = Some(Instant::now());
                break;
            }
        }

        if start.elapsed() > Duration::from_secs(2) {
            break;
        }
    }

    let elapsed = blacklisted_at
        .map(|t| t.duration_since(start))
        .expect("Rogue IP should have been blacklisted");

    assert!(
        elapsed < Duration::from_secs(2),
        "Blacklisting took {elapsed:?}, must be < 2 seconds"
    );
    assert!(blacklist.is_blacklisted(rogue_ip));
}

/// Verify that malformed XDR payload triggers detection and blacklisting.
#[tokio::test]
async fn test_malformed_xdr_blacklisted() {
    let config = AnalyzerConfig {
        malformed_threshold: 5,
        flood_pps_threshold: 10000.0, // high flood threshold so only malformed triggers
        ..Default::default()
    };
    let analyzer = XdrAnalyzer::new(config);
    let blacklist = BlacklistManager::new(None, "stellar", 60, 11625, false);

    let bad_ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9));
    let bad_payload = malformed_payload();

    let mut blacklisted = false;
    for _ in 0..20 {
        let pkt = make_packet([198, 51, 100, 9], 9999, bad_payload.clone());
        let result = analyzer.analyze(&pkt);

        if result.threat_level >= ThreatLevel::High {
            let _ = blacklist
                .blacklist(
                    bad_ip,
                    BlacklistReason::MalformedPayload,
                    result.threat_level,
                    0,
                )
                .await;
            blacklisted = true;
            break;
        }
    }

    assert!(blacklisted, "Malformed-payload IP should be blacklisted");
    assert!(blacklist.is_blacklisted(bad_ip));
}

/// Verify that rapid handshake failures trigger blacklisting.
#[tokio::test]
async fn test_handshake_failure_blacklisted() {
    let config = AnalyzerConfig {
        handshake_fail_threshold: 3,
        flood_pps_threshold: 10000.0,
        ..Default::default()
    };
    let analyzer = XdrAnalyzer::new(config);
    let blacklist = BlacklistManager::new(None, "stellar", 60, 11625, false);

    let rogue_ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 42));
    let hello_payload = make_hello();

    let mut blacklisted = false;
    for _ in 0..10 {
        let pkt = make_packet([198, 51, 100, 42], 54321, hello_payload.clone());
        let result = analyzer.analyze(&pkt);

        if result.threat_level >= ThreatLevel::High {
            let _ = blacklist
                .blacklist(
                    rogue_ip,
                    BlacklistReason::HandshakeFailure,
                    result.threat_level,
                    0,
                )
                .await;
            blacklisted = true;
            break;
        }
    }

    assert!(blacklisted, "Handshake-failure IP should be blacklisted");
    assert!(blacklist.is_blacklisted(rogue_ip));
}

/// Verify that legitimate SCP traffic is never blacklisted.
#[tokio::test]
async fn test_legitimate_traffic_not_blacklisted() {
    let analyzer = XdrAnalyzer::new(AnalyzerConfig {
        flood_pps_threshold: 1000.0, // very high threshold
        ..Default::default()
    });
    let blacklist = BlacklistManager::new(None, "stellar", 60, 11625, false);

    let legit_peers = [[10, 0, 0, 1], [10, 0, 0, 2], [10, 0, 0, 3]];

    for peer in &legit_peers {
        let payload = make_tx();
        for _ in 0..20 {
            let pkt = make_packet(*peer, 40000, payload.clone());
            let result = analyzer.analyze(&pkt);
            assert!(
                result.threat_level < ThreatLevel::High,
                "Legitimate peer {peer:?} should not be threatened"
            );
        }
    }

    assert_eq!(
        blacklist.active_count(),
        0,
        "No legitimate peers should be blacklisted"
    );
}

/// Verify blacklist expiry and pruning.
#[tokio::test]
async fn test_blacklist_entry_expires() {
    // 1-second blacklist duration for fast expiry test.
    let blacklist = BlacklistManager::new(None, "stellar", 1, 11625, false);

    let ip = IpAddr::V4(Ipv4Addr::new(10, 99, 99, 99));
    blacklist
        .blacklist(ip, BlacklistReason::Flood, ThreatLevel::Critical, 100)
        .await
        .unwrap();

    assert!(blacklist.is_blacklisted(ip));

    // Wait for expiry.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // is_blacklisted checks expiry inline.
    assert!(!blacklist.is_blacklisted(ip), "Entry should have expired");

    // Prune should remove expired entries.
    let pruned = blacklist.prune_expired().await.unwrap();
    assert_eq!(pruned, 1);
    assert_eq!(blacklist.active_count(), 0);
}

/// Verify multiple IPs can be blacklisted simultaneously.
#[tokio::test]
async fn test_multiple_ips_blacklisted() {
    let config = AnalyzerConfig {
        flood_pps_threshold: 10.0,
        window: Duration::from_secs(5),
        ..Default::default()
    };
    let analyzer = XdrAnalyzer::new(config);
    let blacklist = BlacklistManager::new(None, "stellar", 60, 11625, false);

    // Flood from 5 different IPs.
    let rogue_ips: Vec<[u8; 4]> = (10..15u8).map(|i| [10, 0, 0, i]).collect();

    for ip_bytes in &rogue_ips {
        let ip = IpAddr::V4(Ipv4Addr::from(*ip_bytes));
        let payload = make_tx();

        for _ in 0..100 {
            let pkt = make_packet(*ip_bytes, 9000, payload.clone());
            let result = analyzer.analyze(&pkt);
            if result.threat_level >= ThreatLevel::High {
                let _ = blacklist
                    .blacklist(ip, BlacklistReason::Flood, result.threat_level, 0)
                    .await;
                break;
            }
        }
    }

    assert_eq!(
        blacklist.active_count(),
        rogue_ips.len(),
        "All 5 rogue IPs should be blacklisted"
    );
}

/// Verify that metrics are emitted correctly through the pipeline.
#[test]
fn test_metrics_integration() {
    let metrics = FirewallMetrics::new();

    metrics.record_packet();
    metrics.record_packet();
    metrics.record_packet();
    metrics.record_threat("CRITICAL");
    metrics.record_threat("HIGH");
    metrics.record_flood_event();
    metrics.record_malformed_packet();
    metrics.record_blacklist_event("added");
    metrics.record_network_policy_update();
    metrics.set_blacklisted_ips(2);
    metrics.record_analysis_duration(0.000_045); // 45 µs — well under 1 ms

    let output = metrics.encode().expect("encode should succeed");

    assert!(output.contains("p2p_firewall_packets_total"));
    assert!(output.contains("p2p_firewall_threats_total"));
    assert!(output.contains("p2p_firewall_flood_events_total"));
    assert!(output.contains("p2p_firewall_malformed_packets_total"));
    assert!(output.contains("p2p_firewall_blacklist_events_total"));
    assert!(output.contains("p2p_firewall_network_policy_updates_total"));
    assert!(output.contains("p2p_firewall_blacklisted_ips"));
    assert!(output.contains("p2p_firewall_analysis_duration_seconds"));
}

/// Verify that analysis latency stays well under 1 ms per packet.
#[test]
fn test_analysis_latency_sub_millisecond() {
    let config = AnalyzerConfig::default();
    let analyzer = XdrAnalyzer::new(config);
    let payload = make_tx();
    let src = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 1)), 50000);

    let iterations = 1000;
    let start = Instant::now();

    for _ in 0..iterations {
        let pkt = RawPacket {
            src_addr: src,
            dst_port: 11625,
            payload: payload.clone(),
            captured_at_ns: 0,
            truncated: false,
        };
        let _ = analyzer.analyze(&pkt);
    }

    let total = start.elapsed();
    let avg_ns = total.as_nanos() / iterations;

    assert!(
        avg_ns < 1_000_000, // < 1 ms per packet
        "Average analysis latency {avg_ns} ns exceeds 1 ms budget"
    );

    println!(
        "[latency] {} packets in {:?} | avg {avg_ns} ns/packet",
        iterations, total
    );
}
