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

//! Load-testing harness for the P2P Gossip Threat Detection Firewall.
//!
//! Simulates a high-volume mixed-traffic scenario:
//! - **10 legitimate peers** sending well-formed SCP messages at 10 pps each.
//! - **5 rogue IPs** flooding at 200 pps (well above the 100 pps threshold).
//! - **3 malformed-payload attackers** sending garbage XDR.
//! - **2 handshake-fail attackers** rapidly re-sending Hello packets.
//!
//! Validates:
//! 1. All rogue IPs are blacklisted within 2 seconds.
//! 2. No legitimate peers are blacklisted.
//! 3. Average per-packet analysis latency stays below 1 ms (sub-millisecond budget).
//! 4. p99 latency stays below 5 ms.
//! 5. Throughput > 10,000 packets/second on a single analysis thread.
//!
//! Run with:
//! ```
//! cargo test -p p2p-firewall --test load_test -- --nocapture
//! ```

use bytes::Bytes;
use p2p_firewall::{
    analyzer::{AnalyzerConfig, XdrAnalyzer},
    blacklist::{BlacklistManager, BlacklistReason},
    ebpf::RawPacket,
    ThreatLevel,
};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};

// ── Helpers ────────────────────────────────────────────────────────────────────

fn make_xdr_bytes(discriminant: u32, len: u32) -> Bytes {
    let mut v = Vec::new();
    v.extend_from_slice(&(0x8000_0000u32 | len).to_be_bytes());
    v.extend_from_slice(&discriminant.to_be_bytes());
    v.extend(vec![0xCDu8; len as usize]);
    Bytes::from(v)
}

fn make_packet(ip: IpAddr, payload: Bytes) -> RawPacket {
    RawPacket {
        src_addr: SocketAddr::new(ip, 40000),
        dst_port: 11625,
        payload,
        captured_at_ns: 0,
        truncated: false,
    }
}

fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

struct LoadTestStats {
    total_packets: u64,
    rogue_blacklisted: usize,
    legit_blacklisted: usize,
    latencies_ns: Vec<u64>,
    elapsed: Duration,
    time_to_first_blacklist: Option<Duration>,
}

// ── Load test ─────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn load_test_p2p_firewall() {
    let config = AnalyzerConfig {
        flood_pps_threshold: 100.0,
        malformed_threshold: 10,
        handshake_fail_threshold: 5,
        window: Duration::from_secs(10),
    };

    let analyzer = XdrAnalyzer::new(config);
    let blacklist = BlacklistManager::new(None, "stellar", 60, 11625, false);

    // Traffic participants.
    let legit_peers: Vec<IpAddr> = (1..=10u8).map(|i| ip(10, 0, 0, i)).collect();
    let rogue_flood: Vec<IpAddr> = (1..=5u8).map(|i| ip(10, 0, 1, i)).collect();
    let malformed_attackers: Vec<IpAddr> = (1..=3u8).map(|i| ip(10, 0, 2, i)).collect();
    let handshake_fail_attackers: Vec<IpAddr> = (1..=2u8).map(|i| ip(10, 0, 3, i)).collect();

    let valid_payload = make_xdr_bytes(5, 128); // Transaction
    let malformed_payload = Bytes::from(vec![0xFF, 0x00, 0x01]); // too short
    let hello_payload = make_xdr_bytes(1, 64); // Hello

    let mut total_packets: u64 = 0;
    let mut latencies_ns: Vec<u64> = Vec::with_capacity(200_000);
    let mut time_to_first_blacklist: Option<Duration> = None;
    let overall_start = Instant::now();

    // Simulate 20 rounds, each round = 1 "tick"
    // Legit: 10 pps × 10 peers = 100 pkt/tick
    // Flood: 200 pps × 5 rogues = 1000 pkt/tick
    // Malformed: 15 pps × 3 = 45 pkt/tick
    // Handshake-fail: 10 pps × 2 = 20 pkt/tick
    // Total ≈ 1165 pkt/tick × 20 ticks ≈ 23,300 packets

    for _round in 0..20 {
        // Legitimate traffic (10 packets per peer per round).
        for peer in &legit_peers {
            for _ in 0..10 {
                let pkt = make_packet(*peer, valid_payload.clone());
                let t = Instant::now();
                let _r = analyzer.analyze(&pkt);
                latencies_ns.push(t.elapsed().as_nanos() as u64);
                total_packets += 1;
            }
        }

        // Flood traffic (200 packets per rogue per round).
        for rogue in &rogue_flood {
            for _ in 0..200 {
                let pkt = make_packet(*rogue, valid_payload.clone());
                let t = Instant::now();
                let result = analyzer.analyze(&pkt);
                latencies_ns.push(t.elapsed().as_nanos() as u64);
                total_packets += 1;

                if result.threat_level >= ThreatLevel::High && !blacklist.is_blacklisted(*rogue) {
                    let elapsed_at_detection = overall_start.elapsed();
                    let _ = blacklist
                        .blacklist(*rogue, BlacklistReason::Flood, result.threat_level, 0)
                        .await;

                    if time_to_first_blacklist.is_none() {
                        time_to_first_blacklist = Some(elapsed_at_detection);
                    }
                }
            }
        }

        // Malformed-payload traffic (15 packets per attacker per round).
        for attacker in &malformed_attackers {
            for _ in 0..15 {
                let pkt = make_packet(*attacker, malformed_payload.clone());
                let t = Instant::now();
                let result = analyzer.analyze(&pkt);
                latencies_ns.push(t.elapsed().as_nanos() as u64);
                total_packets += 1;

                if result.threat_level >= ThreatLevel::High && !blacklist.is_blacklisted(*attacker)
                {
                    let _ = blacklist
                        .blacklist(
                            *attacker,
                            BlacklistReason::MalformedPayload,
                            result.threat_level,
                            0,
                        )
                        .await;
                }
            }
        }

        // Handshake-failure traffic (10 packets per attacker per round).
        for attacker in &handshake_fail_attackers {
            for _ in 0..10 {
                let pkt = make_packet(*attacker, hello_payload.clone());
                let t = Instant::now();
                let result = analyzer.analyze(&pkt);
                latencies_ns.push(t.elapsed().as_nanos() as u64);
                total_packets += 1;

                if result.threat_level >= ThreatLevel::High && !blacklist.is_blacklisted(*attacker)
                {
                    let _ = blacklist
                        .blacklist(
                            *attacker,
                            BlacklistReason::HandshakeFailure,
                            result.threat_level,
                            0,
                        )
                        .await;
                }
            }
        }
    }

    let elapsed = overall_start.elapsed();

    // ── Compute statistics ──────────────────────────────────────────────────────

    latencies_ns.sort_unstable();
    let avg_ns = latencies_ns.iter().sum::<u64>() / latencies_ns.len() as u64;
    let p50_ns = latencies_ns[latencies_ns.len() * 50 / 100];
    let p95_ns = latencies_ns[latencies_ns.len() * 95 / 100];
    let p99_ns = latencies_ns[latencies_ns.len() * 99 / 100];
    let max_ns = *latencies_ns.last().unwrap();
    let throughput_pps = total_packets as f64 / elapsed.as_secs_f64();

    let rogue_blacklisted = rogue_flood
        .iter()
        .filter(|ip| blacklist.is_blacklisted(**ip))
        .count();
    let legit_blacklisted = legit_peers
        .iter()
        .filter(|ip| blacklist.is_blacklisted(**ip))
        .count();
    let malformed_blacklisted = malformed_attackers
        .iter()
        .filter(|ip| blacklist.is_blacklisted(**ip))
        .count();
    let handshake_blacklisted = handshake_fail_attackers
        .iter()
        .filter(|ip| blacklist.is_blacklisted(**ip))
        .count();

    // ── Print load-test report ──────────────────────────────────────────────────

    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║       P2P Firewall Load-Test Report                              ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  Traffic Summary                                                 ║");
    println!("║    Total packets analyzed : {total_packets:<10}                    ║");
    println!(
        "║    Total elapsed time     : {:?}                         ║",
        elapsed
    );
    println!("║    Throughput             : {throughput_pps:.0} pkt/s                    ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  Analysis Latency (per-packet)                                   ║");
    println!(
        "║    Average   : {avg_ns:>8} ns  ({:.3} ms)                        ║",
        avg_ns as f64 / 1e6
    );
    println!(
        "║    P50       : {p50_ns:>8} ns  ({:.3} ms)                        ║",
        p50_ns as f64 / 1e6
    );
    println!(
        "║    P95       : {p95_ns:>8} ns  ({:.3} ms)                        ║",
        p95_ns as f64 / 1e6
    );
    println!(
        "║    P99       : {p99_ns:>8} ns  ({:.3} ms)                        ║",
        p99_ns as f64 / 1e6
    );
    println!(
        "║    Max       : {max_ns:>8} ns  ({:.3} ms)                        ║",
        max_ns as f64 / 1e6
    );
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  Threat Detection                                                ║");
    println!(
        "║    Flood IPs blacklisted      : {}/{:<3}                            ║",
        rogue_blacklisted,
        rogue_flood.len()
    );
    println!(
        "║    Malformed IPs blacklisted  : {}/{:<3}                            ║",
        malformed_blacklisted,
        malformed_attackers.len()
    );
    println!(
        "║    Handshake-fail blacklisted : {}/{:<3}                            ║",
        handshake_blacklisted,
        handshake_fail_attackers.len()
    );
    println!(
        "║    Legitimate peers blocked   : {}/{:<3}  (should be 0)             ║",
        legit_blacklisted,
        legit_peers.len()
    );
    if let Some(t) = time_to_first_blacklist {
        println!(
            "║    First blacklist at         : {:?}                           ║",
            t
        );
    }
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();

    // ── Assertions ──────────────────────────────────────────────────────────────

    assert_eq!(
        legit_blacklisted, 0,
        "No legitimate peers should be blacklisted"
    );

    assert_eq!(
        rogue_blacklisted,
        rogue_flood.len(),
        "All flood rogue IPs must be blacklisted"
    );

    // Flood detection must trigger within 2 seconds (issue requirement).
    if let Some(first_bl) = time_to_first_blacklist {
        assert!(
            first_bl < Duration::from_secs(2),
            "First blacklist occurred at {first_bl:?}, must be < 2 seconds"
        );
    } else {
        panic!("No IP was ever blacklisted — flood detection failed");
    }

    // Average latency must be sub-millisecond.
    assert!(
        avg_ns < 1_000_000,
        "Average analysis latency {avg_ns} ns exceeds 1 ms budget"
    );

    // P99 must be under 5 ms.
    assert!(
        p99_ns < 5_000_000,
        "P99 analysis latency {p99_ns} ns exceeds 5 ms"
    );

    // Throughput must exceed 10,000 pkt/s.
    assert!(
        throughput_pps > 10_000.0,
        "Throughput {throughput_pps:.0} pkt/s is below 10,000 pkt/s minimum"
    );
}
