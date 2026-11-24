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

//! Rust-based XDR payload and heuristic threat analyzer (`analyzer.rs`).
//!
//! Consumes [`RawPacket`] events from the eBPF interceptor and applies a
//! multi-stage detection pipeline:
//!
//! 1. **XDR structure validation** — Stellar encodes all SCP messages with
//!    XDR (RFC 4506).  Every SCP message begins with a 4-byte big-endian
//!    length field followed by a 4-byte discriminant identifying the envelope
//!    type.  Packets that violate these invariants are flagged as malformed.
//!
//! 2. **Rate / flood detection** — A sliding window counter per source IP
//!    tracks packets-per-second.  IPs exceeding the configured threshold are
//!    classified as flooding.
//!
//! 3. **Handshake failure detection** — SCP peers begin with a `Hello`
//!    envelope (discriminant `0x00000001`).  An IP that sends repeated
//!    `Hello` messages without completing the exchange is considered to be
//!    failing handshakes rapidly.
//!
//! 4. **Payload entropy / anomaly heuristics** — Very low or very high byte
//!    entropy indicates garbage or encrypted-junk payloads not consistent
//!    with well-formed XDR.
//!
//! The analyzer runs in O(1) time per packet using hash maps and circular
//! counters, keeping latency well under the 1 ms budget.

use crate::ebpf::RawPacket;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use thiserror::Error;
use tracing::{debug, warn};

// ── XDR SCP envelope discriminants ────────────────────────────────────────────

/// SCP `Hello` envelope type (handshake initiation).
const XDR_DISC_HELLO: u32 = 1;
/// SCP `Auth` envelope (post-hello authentication).
const XDR_DISC_AUTH: u32 = 2;
/// SCP `Error` envelope.
const XDR_DISC_ERROR: u32 = 3;
/// SCP `SendMore` envelope.
const XDR_DISC_SEND_MORE: u32 = 4;
/// SCP `Transaction` envelope.
const XDR_DISC_TX: u32 = 5;
/// SCP `StellarMessage` envelope range (max known discriminant).
const XDR_DISC_MAX_KNOWN: u32 = 20;
/// Minimum valid XDR header: 4-byte length + 4-byte discriminant.
const XDR_MIN_HEADER_LEN: usize = 8;
/// Maximum sane SCP message size (64 KiB).
const XDR_MAX_MSG_LEN: u32 = 65536;

// ── Public types ───────────────────────────────────────────────────────────────

/// Severity classification of a detected threat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub enum ThreatLevel {
    /// Normal traffic.
    None,
    /// Suspicious but not immediately actionable.
    Low,
    /// Likely malicious; elevated monitoring.
    Medium,
    /// Definitely malicious; blacklist immediately.
    High,
    /// Flood / DoS; emergency blacklist.
    Critical,
}

impl std::fmt::Display for ThreatLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThreatLevel::None => write!(f, "NONE"),
            ThreatLevel::Low => write!(f, "LOW"),
            ThreatLevel::Medium => write!(f, "MEDIUM"),
            ThreatLevel::High => write!(f, "HIGH"),
            ThreatLevel::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// Structured result of analyzing a single [`RawPacket`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    /// Source IP address.
    pub src_ip: IpAddr,
    /// Overall threat classification.
    pub threat_level: ThreatLevel,
    /// Human-readable reason(s) for the classification.
    pub reasons: Vec<String>,
    /// Current packets-per-second estimate for this IP.
    pub pps: f64,
    /// Number of malformed payloads seen from this IP in the window.
    pub malformed_count: u32,
    /// Number of failed handshakes seen from this IP in the window.
    pub handshake_fail_count: u32,
    /// UTC timestamp of the analysis.
    pub analyzed_at: String,
}

/// Normalised packet metadata extracted from a [`RawPacket`].
#[derive(Debug, Clone)]
pub struct PacketInfo {
    pub src_ip: IpAddr,
    pub payload_len: usize,
    pub xdr_discriminant: Option<u32>,
    pub xdr_length: Option<u32>,
    pub is_malformed: bool,
    pub is_hello: bool,
}

/// Errors from the analysis pipeline.
#[derive(Debug, Error)]
pub enum AnalyzerError {
    #[error("payload too short for XDR header ({0} bytes)")]
    PayloadTooShort(usize),
    #[error("XDR length field {0} exceeds maximum {1}")]
    XdrLengthOverflow(u32, u32),
    #[error("unknown XDR discriminant {0}")]
    UnknownDiscriminant(u32),
}

// ── Per-IP state ───────────────────────────────────────────────────────────────

/// Sliding-window statistics tracked per source IP.
#[derive(Debug)]
struct IpStats {
    /// Timestamps of recent packets (used for pps calculation).
    packet_times: Vec<Instant>,
    /// Count of malformed payloads in the current window.
    malformed_count: u32,
    /// Count of failed / repeated handshakes in the current window.
    handshake_fail_count: u32,
    /// Last time this IP sent a `Hello` (for handshake-fail detection).
    last_hello_at: Option<Instant>,
    /// Whether the handshake completed (Auth received).
    handshake_complete: bool,
}

impl IpStats {
    fn new() -> Self {
        Self {
            packet_times: Vec::with_capacity(256),
            malformed_count: 0,
            handshake_fail_count: 0,
            last_hello_at: None,
            handshake_complete: false,
        }
    }

    /// Evict timestamps older than `window`.
    fn evict_old(&mut self, window: Duration) {
        let cutoff = Instant::now()
            .checked_sub(window)
            .unwrap_or_else(Instant::now);
        self.packet_times.retain(|t| *t > cutoff);
    }

    /// Packets per second over the window.
    fn pps(&self, window: Duration) -> f64 {
        let window_secs = window.as_secs_f64();
        if window_secs > 0.0 {
            self.packet_times.len() as f64 / window_secs
        } else {
            0.0
        }
    }
}

// ── Analyzer ──────────────────────────────────────────────────────────────────

/// Configuration for [`XdrAnalyzer`].
#[derive(Debug, Clone)]
pub struct AnalyzerConfig {
    /// Packets-per-second per IP that triggers a CRITICAL flood alert.
    pub flood_pps_threshold: f64,
    /// Malformed payload count per window before HIGH threat.
    pub malformed_threshold: u32,
    /// Failed handshake count per window before HIGH threat.
    pub handshake_fail_threshold: u32,
    /// Duration of the sliding window.
    pub window: Duration,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            flood_pps_threshold: 100.0,
            malformed_threshold: 10,
            handshake_fail_threshold: 5,
            window: Duration::from_secs(10),
        }
    }
}

/// XDR payload analyzer with per-IP state tracking.
#[derive(Clone)]
pub struct XdrAnalyzer {
    config: AnalyzerConfig,
    /// Per-IP statistics.  Wrapped in Arc<Mutex> so the analyzer can be
    /// cheaply cloned and shared across async tasks.
    state: Arc<Mutex<HashMap<IpAddr, IpStats>>>,
}

impl XdrAnalyzer {
    /// Create a new analyzer with the given configuration.
    pub fn new(config: AnalyzerConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Analyze a raw captured packet and return a [`AnalysisResult`].
    ///
    /// This function is designed to complete in **well under 1 ms** on modern
    /// hardware (no I/O, no allocation on the hot path beyond Vec push).
    pub fn analyze(&self, packet: &RawPacket) -> AnalysisResult {
        let src_ip = packet.src_addr.ip();

        // Parse XDR header (zero-copy).
        let info = parse_xdr_header(&packet.payload);

        let mut reasons = Vec::new();
        let mut threat = ThreatLevel::None;

        // Update per-IP state under lock (held briefly).
        let (pps, malformed_count, handshake_fail_count) = {
            let mut state = self.state.lock().expect("analyzer state lock poisoned");
            let stats = state.entry(src_ip).or_insert_with(IpStats::new);

            stats.evict_old(self.config.window);
            stats.packet_times.push(Instant::now());

            // Track malformed payloads.
            if info.is_malformed {
                stats.malformed_count += 1;
            }

            // Track handshake failures: if we see repeated Hello without Auth.
            if info.is_hello {
                if let Some(last) = stats.last_hello_at {
                    if last.elapsed() < Duration::from_secs(5) && !stats.handshake_complete {
                        stats.handshake_fail_count += 1;
                    }
                }
                stats.last_hello_at = Some(Instant::now());
                stats.handshake_complete = false;
            } else if info.xdr_discriminant == Some(XDR_DISC_AUTH) {
                stats.handshake_complete = true;
            }

            let pps = stats.pps(self.config.window);
            (pps, stats.malformed_count, stats.handshake_fail_count)
        };

        // ── Detection rules (in ascending severity order) ───────────────────

        // Rule 1: malformed XDR payload.
        if info.is_malformed {
            reasons.push("malformed XDR payload".into());
            threat = threat.max(ThreatLevel::Medium);
        }

        // Rule 2: malformed threshold exceeded → escalate to HIGH.
        if malformed_count >= self.config.malformed_threshold {
            reasons.push(format!(
                "malformed payload count ({malformed_count}) exceeds threshold ({})",
                self.config.malformed_threshold
            ));
            threat = threat.max(ThreatLevel::High);
        }

        // Rule 3: rapid handshake failures.
        if handshake_fail_count >= self.config.handshake_fail_threshold {
            reasons.push(format!(
                "rapid handshake failures ({handshake_fail_count}) exceed threshold ({})",
                self.config.handshake_fail_threshold
            ));
            threat = threat.max(ThreatLevel::High);
        }

        // Rule 4: flood detection (PPS over threshold).
        if pps >= self.config.flood_pps_threshold {
            reasons.push(format!(
                "packet flood detected: {pps:.1} pps exceeds threshold ({:.1})",
                self.config.flood_pps_threshold
            ));
            threat = threat.max(ThreatLevel::Critical);
        }

        // Rule 5: low entropy garbage payload (not a valid XDR start).
        if let Some(entropy) = byte_entropy(&packet.payload) {
            if entropy < 0.5 && packet.payload.len() > 32 {
                reasons.push(format!(
                    "low payload entropy ({entropy:.2}) suggests garbage data"
                ));
                threat = threat.max(ThreatLevel::Medium);
            }
        }

        if threat > ThreatLevel::None {
            warn!(
                %src_ip,
                %threat,
                pps = %format!("{pps:.1}"),
                malformed = malformed_count,
                handshake_fails = handshake_fail_count,
                reasons = ?reasons,
                "Threat detected on SCP port"
            );
        } else {
            debug!(%src_ip, pps = %format!("{pps:.1}"), "Packet analyzed: clean");
        }

        AnalysisResult {
            src_ip,
            threat_level: threat,
            reasons,
            pps,
            malformed_count,
            handshake_fail_count,
            analyzed_at: Utc::now().to_rfc3339(),
        }
    }

    /// Reset per-IP state (useful in tests).
    pub fn reset(&self) {
        self.state.lock().expect("lock poisoned").clear();
    }

    /// Return a snapshot of the per-IP stats (for debugging / metrics).
    pub fn ip_count(&self) -> usize {
        self.state.lock().expect("lock poisoned").len()
    }
}

// ── XDR parsing helpers ────────────────────────────────────────────────────────

/// Parse the XDR framing header from raw bytes and return a [`PacketInfo`].
///
/// Stellar's TCP-framed XDR uses:
///   - Bytes 0–3: big-endian message length (highest bit = "last fragment").
///   - Bytes 4–7: big-endian discriminant (SCP envelope type).
pub fn parse_xdr_header(payload: &[u8]) -> PacketInfo {
    // Placeholder: we derive src_ip from the caller; fill with 0.0.0.0 here.
    use std::net::Ipv4Addr;
    let src_ip = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

    if payload.len() < XDR_MIN_HEADER_LEN {
        return PacketInfo {
            src_ip,
            payload_len: payload.len(),
            xdr_discriminant: None,
            xdr_length: None,
            is_malformed: true,
            is_hello: false,
        };
    }

    // XDR record-mark: top bit is "last fragment" flag; lower 31 bits = length.
    let raw_len = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let xdr_length = raw_len & 0x7FFF_FFFF; // strip fragment bit

    let discriminant = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);

    let is_malformed =
        xdr_length == 0 || xdr_length > XDR_MAX_MSG_LEN || discriminant > XDR_DISC_MAX_KNOWN;

    let is_hello = discriminant == XDR_DISC_HELLO;

    PacketInfo {
        src_ip,
        payload_len: payload.len(),
        xdr_discriminant: Some(discriminant),
        xdr_length: Some(xdr_length),
        is_malformed,
        is_hello,
    }
}

/// Compute Shannon entropy (0.0–1.0) of a byte slice.
///
/// Returns `None` for empty slices.
pub fn byte_entropy(data: &[u8]) -> Option<f64> {
    if data.is_empty() {
        return None;
    }

    let mut counts = [0u32; 256];
    for &b in data {
        counts[b as usize] += 1;
    }

    let len = data.len() as f64;
    let entropy = counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum::<f64>();

    // Normalise to [0, 1] (max entropy for 256 symbols = 8 bits)
    Some(entropy / 8.0)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_valid_xdr(discriminant: u32, payload_len: u32) -> Vec<u8> {
        let mut v = Vec::new();
        // Fragment bit set (0x80000000) | length
        let raw_len: u32 = 0x8000_0000 | payload_len;
        v.extend_from_slice(&raw_len.to_be_bytes());
        v.extend_from_slice(&discriminant.to_be_bytes());
        v.extend(vec![0u8; payload_len as usize]);
        v
    }

    #[test]
    fn test_valid_xdr_hello() {
        let payload = make_valid_xdr(XDR_DISC_HELLO, 64);
        let info = parse_xdr_header(&payload);
        assert!(!info.is_malformed);
        assert!(info.is_hello);
        assert_eq!(info.xdr_discriminant, Some(XDR_DISC_HELLO));
        assert_eq!(info.xdr_length, Some(64));
    }

    #[test]
    fn test_malformed_too_short() {
        let info = parse_xdr_header(&[0x00, 0x01]);
        assert!(info.is_malformed);
    }

    #[test]
    fn test_malformed_zero_length() {
        let payload = make_valid_xdr(XDR_DISC_TX, 0);
        let info = parse_xdr_header(&payload);
        // length 0 → malformed
        assert!(info.is_malformed);
    }

    #[test]
    fn test_malformed_length_overflow() {
        let mut v = Vec::new();
        v.extend_from_slice(&(0xFFFF_FFFFu32).to_be_bytes()); // huge length
        v.extend_from_slice(&XDR_DISC_TX.to_be_bytes());
        let info = parse_xdr_header(&v);
        assert!(info.is_malformed);
    }

    #[test]
    fn test_malformed_unknown_discriminant() {
        let payload = make_valid_xdr(9999, 32);
        let info = parse_xdr_header(&payload);
        assert!(info.is_malformed);
    }

    #[test]
    fn test_byte_entropy_uniform() {
        let data: Vec<u8> = (0..=255u8).collect();
        let ent = byte_entropy(&data).unwrap();
        // Uniform distribution → max entropy ≈ 1.0
        assert!(ent > 0.99, "uniform entropy should be near 1.0, got {ent}");
    }

    #[test]
    fn test_byte_entropy_constant() {
        let data = vec![0x42u8; 100];
        let ent = byte_entropy(&data).unwrap();
        // Single symbol → entropy = 0
        assert!(ent < 0.01, "constant entropy should be ~0, got {ent}");
    }

    #[test]
    fn test_byte_entropy_empty() {
        assert!(byte_entropy(&[]).is_none());
    }

    #[test]
    fn test_analyzer_clean_packet() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let config = AnalyzerConfig::default();
        let analyzer = XdrAnalyzer::new(config);

        let payload = make_valid_xdr(XDR_DISC_TX, 128);
        let packet = RawPacket {
            src_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 54321),
            dst_port: 11625,
            payload: bytes::Bytes::from(payload),
            captured_at_ns: 0,
            truncated: false,
        };

        let result = analyzer.analyze(&packet);
        assert_eq!(result.threat_level, ThreatLevel::None);
        assert!(result.reasons.is_empty());
    }

    #[test]
    fn test_analyzer_malformed_packet() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let config = AnalyzerConfig::default();
        let analyzer = XdrAnalyzer::new(config);

        // Only 4 bytes — too short for XDR header
        let packet = RawPacket {
            src_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), 54321),
            dst_port: 11625,
            payload: bytes::Bytes::from(vec![0x00, 0x00, 0x00, 0x04]),
            captured_at_ns: 0,
            truncated: false,
        };

        let result = analyzer.analyze(&packet);
        assert!(result.threat_level >= ThreatLevel::Medium);
        assert!(!result.reasons.is_empty());
    }

    #[test]
    fn test_flood_detection() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let config = AnalyzerConfig {
            flood_pps_threshold: 5.0,
            window: Duration::from_secs(10),
            ..Default::default()
        };
        let analyzer = XdrAnalyzer::new(config);

        let payload = make_valid_xdr(XDR_DISC_TX, 64);
        let src = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), 9000);

        let mut last_result = None;
        // Send 60 packets — should trigger flood at pps threshold of 5
        for _ in 0..60 {
            let packet = RawPacket {
                src_addr: src,
                dst_port: 11625,
                payload: bytes::Bytes::from(payload.clone()),
                captured_at_ns: 0,
                truncated: false,
            };
            last_result = Some(analyzer.analyze(&packet));
        }

        let result = last_result.unwrap();
        assert_eq!(result.threat_level, ThreatLevel::Critical);
        assert!(result.reasons.iter().any(|r| r.contains("flood")));
    }

    #[test]
    fn test_handshake_failure_detection() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let config = AnalyzerConfig {
            handshake_fail_threshold: 3,
            ..Default::default()
        };
        let analyzer = XdrAnalyzer::new(config);

        let hello_payload = make_valid_xdr(XDR_DISC_HELLO, 64);
        let src = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40)), 12345);

        let mut last_result = None;
        // Repeatedly send Hello without Auth — simulates rapid handshake failures
        for _ in 0..6 {
            let packet = RawPacket {
                src_addr: src,
                dst_port: 11625,
                payload: bytes::Bytes::from(hello_payload.clone()),
                captured_at_ns: 0,
                truncated: false,
            };
            last_result = Some(analyzer.analyze(&packet));
        }

        let result = last_result.unwrap();
        assert!(result.threat_level >= ThreatLevel::High);
        assert!(result.reasons.iter().any(|r| r.contains("handshake")));
    }

    #[test]
    fn test_threat_level_ordering() {
        assert!(ThreatLevel::Critical > ThreatLevel::High);
        assert!(ThreatLevel::High > ThreatLevel::Medium);
        assert!(ThreatLevel::Medium > ThreatLevel::Low);
        assert!(ThreatLevel::Low > ThreatLevel::None);
    }
}
