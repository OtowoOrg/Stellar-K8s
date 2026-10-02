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
//! Shared types that cross the kernel/userspace boundary.
//!
//! These structs mirror the C structs defined in `scp_trace.c` exactly —
//! layout, field order, and padding must remain identical.  The `#[repr(C)]`
//! attribute guarantees the Rust compiler does not reorder or add implicit
//! padding beyond what C would.
//!
//! The userspace loader reads raw bytes from the perf ring buffer and
//! transmutes them into [`RawScpEvent`]; any mismatch would produce silent
//! data corruption, so this file is the single authoritative definition.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::Ipv4Addr;

// ─── Mirror of EVT_SCP_* discriminants from scp_trace.c ──────────────────────

/// Matches the `EVT_SCP_*` constants in `scp_trace.c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum EventType {
    /// Inbound SCP TCP segment received.
    ScpInbound = 1,
    /// TCP RST sent for an SCP connection (hard drop).
    TcpRst = 2,
    /// TCP retransmit event on an SCP connection.
    Retransmit = 3,
    /// TCP RST during handshake phase (cryptographic handshake failure).
    HandshakeFail = 4,
    /// Unknown discriminant (kernel/userspace ABI drift guard).
    Unknown = 255,
}

impl From<u8> for EventType {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::ScpInbound,
            2 => Self::TcpRst,
            3 => Self::Retransmit,
            4 => Self::HandshakeFail,
            _ => Self::Unknown,
        }
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScpInbound => write!(f, "scp_inbound"),
            Self::TcpRst => write!(f, "tcp_rst"),
            Self::Retransmit => write!(f, "retransmit"),
            Self::HandshakeFail => write!(f, "handshake_fail"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

// ─── Raw perf-buffer event (mirrors struct scp_event in scp_trace.c) ─────────

/// Raw event as emitted by the BPF program into the perf ring buffer.
///
/// **Must remain layout-compatible with `struct scp_event` in `scp_trace.c`.**
/// Total size: 8 + 4 + 4 + 2 + 2 + 1 + 1 + 16 + 1 + 5 = 44 bytes padded to 48.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct RawScpEvent {
    /// Kernel monotonic timestamp (nanoseconds).
    pub timestamp_ns: u64,
    /// Source IP address (network byte order).
    pub src_ip: u32,
    /// Destination IP address (network byte order).
    pub dst_ip: u32,
    /// Source port (host byte order).
    pub src_port: u16,
    /// Destination port (host byte order).
    pub dst_port: u16,
    /// Event type discriminant (`EVT_SCP_*`).
    pub event_type: u8,
    /// TCP flags byte (meaningful for RST events).
    pub tcp_flags: u8,
    /// First bytes of XDR payload (big-endian; zero-padded when not captured).
    pub xdr_sample: [u8; 16],
    /// Number of valid bytes in `xdr_sample` (0 = no payload captured).
    pub xdr_sample_len: u8,
    /// Alignment padding — reserved, always zero.
    pub _pad: [u8; 5],
}

impl RawScpEvent {
    /// Expected byte size; used by the loader to validate the perf buffer slice.
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

// ─── Decoded, ergonomic event type ───────────────────────────────────────────

/// Decoded SCP network event, ready for metrics aggregation and dashboard display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpEvent {
    /// Wall-clock time of the event (ISO-8601).
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Remote peer IP address as a human-readable string.
    pub peer_ip: String,
    /// Local node IP address.
    pub local_ip: String,
    /// Remote port (for display purposes; usually ephemeral).
    pub peer_port: u16,
    /// Local port (typically 11625).
    pub local_port: u16,
    /// Decoded event type.
    pub event_type: EventType,
    /// TCP flags byte (hex string, e.g. `"0x14"` = RST+ACK).
    pub tcp_flags_hex: String,
    /// Inferred XDR SCP message type, if payload bytes were captured.
    pub xdr_message_type: Option<XdrScpMessageType>,
    /// Whether this event represents a confirmed drop (RST or handshake fail).
    pub is_drop: bool,
}

/// Inferred XDR StellarMessage type from the first 4 bytes of payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XdrScpMessageType {
    QuorumSet,
    ScpStatement,
    Hello,
    Auth,
    Other(u32),
}

impl From<u32> for XdrScpMessageType {
    fn from(v: u32) -> Self {
        match v {
            2 => Self::QuorumSet,
            3 => Self::ScpStatement,
            14 => Self::Hello,
            _ => Self::Other(v),
        }
    }
}

impl fmt::Display for XdrScpMessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QuorumSet => write!(f, "QUORUM_SET"),
            Self::ScpStatement => write!(f, "SCP_STATEMENT"),
            Self::Hello => write!(f, "HELLO"),
            Self::Auth => write!(f, "AUTH"),
            Self::Other(v) => write!(f, "UNKNOWN({v})"),
        }
    }
}

// ─── Per-peer drop entry for the dashboard ────────────────────────────────────

/// Aggregated drop statistics for a single peer IP address.
/// Surfaced via the `/api/v1/scp/peer-drops` REST endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerDropEntry {
    /// Remote peer IP address.
    pub peer_ip: String,
    /// Total TCP RST packets sent to/from this peer on port 11625.
    pub rst_count: u64,
    /// TCP RSTs during the TLS/SCP handshake phase (cryptographic drops).
    pub handshake_fail_count: u64,
    /// TCP retransmit events on SCP connections to this peer.
    pub retransmit_count: u64,
    /// Total inbound SCP segments received from this peer.
    pub packets_rx: u64,
    /// Computed drop rate: `rst_count / max(packets_rx, 1)` as a percentage.
    pub drop_rate_pct: f64,
    /// ISO-8601 timestamp of the most recently observed event.
    pub last_seen: String,
    /// Whether this peer is currently considered degraded (drop_rate > threshold).
    pub is_degraded: bool,
}

impl PeerDropEntry {
    /// A peer is considered degraded when its drop rate exceeds 5 %.
    pub const DEGRADED_THRESHOLD_PCT: f64 = 5.0;

    /// Compute the drop rate and degraded flag from raw counters.
    pub fn compute_rates(&mut self) {
        let denom = self.packets_rx.max(1) as f64;
        self.drop_rate_pct = (self.rst_count as f64 / denom) * 100.0;
        self.is_degraded = self.drop_rate_pct > Self::DEGRADED_THRESHOLD_PCT
            || self.handshake_fail_count > 0;
    }
}

// ─── Network health summary ───────────────────────────────────────────────────

/// Top-level SCP network health snapshot for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpNetworkHealth {
    /// ISO-8601 timestamp when this snapshot was generated.
    pub generated_at: String,
    /// Total number of unique peers observed on port 11625.
    pub total_peers: usize,
    /// Number of peers currently classified as degraded.
    pub degraded_peers: usize,
    /// Total RST packets across all SCP connections since the sniffer started.
    pub total_rst_count: u64,
    /// Total handshake failures across all SCP connections.
    pub total_handshake_fails: u64,
    /// Total retransmit events across all SCP connections.
    pub total_retransmits: u64,
    /// Overall network health score 0–100 (100 = no drops, 0 = all traffic dropped).
    pub health_score: u8,
    /// List of the top offending peers sorted by drop rate descending.
    pub top_drop_peers: Vec<PeerDropEntry>,
}

impl ScpNetworkHealth {
    /// Compute the `health_score` from aggregate counters.
    ///
    /// Score = 100 − min(100, (total_rst + handshake_fails) / max(total_rx, 1) × 100)
    pub fn compute_health_score(&mut self, total_rx: u64) {
        let drops = self.total_rst_count + self.total_handshake_fails;
        let rate = (drops as f64 / total_rx.max(1) as f64) * 100.0;
        self.health_score = 100u8.saturating_sub(rate.min(100.0) as u8);
    }
}

// ─── Utility: network-byte-order u32 → Ipv4Addr ──────────────────────────────

/// Convert a network-byte-order `u32` from the BPF event into a dotted-decimal string.
pub fn nbo_u32_to_ipv4_string(nbo: u32) -> String {
    Ipv4Addr::from(u32::from_be(nbo)).to_string()
}

/// Decode the XDR message type from the first 4 bytes of the sample buffer.
/// Returns `None` if fewer than 4 bytes were captured.
pub fn decode_xdr_type(sample: &[u8; 16], len: u8) -> Option<XdrScpMessageType> {
    if len < 4 {
        return None;
    }
    let discriminant = u32::from_be_bytes([sample[0], sample[1], sample[2], sample[3]]);
    Some(XdrScpMessageType::from(discriminant))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_roundtrip() {
        for v in [1u8, 2, 3, 4] {
            let et = EventType::from(v);
            assert_ne!(et, EventType::Unknown);
        }
        assert_eq!(EventType::from(99), EventType::Unknown);
    }

    #[test]
    fn nbo_u32_conversion() {
        // 10.0.0.1 in network byte order
        let nbo: u32 = u32::to_be(0x0A000001u32);
        assert_eq!(nbo_u32_to_ipv4_string(nbo), "10.0.0.1");
    }

    #[test]
    fn xdr_type_decode_hello() {
        let mut sample = [0u8; 16];
        sample[3] = 14; // big-endian 14
        let t = decode_xdr_type(&sample, 4);
        assert_eq!(t, Some(XdrScpMessageType::Hello));
    }

    #[test]
    fn peer_drop_entry_degraded() {
        let mut entry = PeerDropEntry {
            peer_ip: "10.0.0.1".into(),
            rst_count: 10,
            handshake_fail_count: 0,
            retransmit_count: 2,
            packets_rx: 100,
            drop_rate_pct: 0.0,
            last_seen: String::new(),
            is_degraded: false,
        };
        entry.compute_rates();
        assert!((entry.drop_rate_pct - 10.0).abs() < f64::EPSILON);
        assert!(entry.is_degraded);
    }

    #[test]
    fn raw_scp_event_size() {
        // Verify layout matches C struct (44 bytes usable, padded to 48 by repr(C)).
        assert_eq!(RawScpEvent::SIZE, 48);
    }
}
