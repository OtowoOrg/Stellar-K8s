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
//! Axum HTTP handlers for the SCP eBPF sniffer dashboard endpoints.
//!
//! # Endpoints
//!
//! | Method | Path                              | Description                          |
//! |--------|-----------------------------------|--------------------------------------|
//! | GET    | `/api/v1/scp/network-health`      | Full SCP network health snapshot     |
//! | GET    | `/api/v1/scp/peer-drops`          | Per-peer drop statistics (all peers) |
//! | GET    | `/api/v1/scp/peer-drops/:peer_ip` | Drop stats for a specific peer IP    |
//!
//! # State injection
//!
//! Handlers receive an axum `Extension<Arc<RwLock<ScpNetworkHealthState>>>` that
//! is populated by [`crate::security::ebpf_sniffer::ScpSnifferMonitor`].  The
//! extension is registered in `src/rest_api/server.rs` alongside the other
//! operator extensions.
//!
//! If the sniffer has not yet completed a poll cycle, or is unavailable, the
//! handlers return a valid JSON body with `sniffer_available: false` so the
//! dashboard degrades gracefully rather than returning HTTP 503.

use std::sync::{Arc, RwLock};

use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{instrument, warn};

use crate::rest_api::dto::ErrorResponse;
use crate::security::ebpf_sniffer::{PeerNetworkStats, ScpNetworkHealthState};

// ─── Response DTOs ────────────────────────────────────────────────────────────

/// Response body for `GET /api/v1/scp/network-health`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScpNetworkHealthResponse {
    /// Whether the sniffer exporter was reachable on the last poll.
    pub sniffer_available: bool,
    /// ISO-8601 timestamp of the last successful scrape.
    pub last_scraped: String,
    /// Overall health score 0–100.
    pub health_score: u8,
    /// Total unique SCP peers observed since sniffer started.
    pub total_peers: usize,
    /// Number of peers currently classified as degraded.
    pub degraded_peers: usize,
    /// Top offending peers, sorted by drop rate descending (max 10).
    pub top_drop_peers: Vec<PeerDropSummary>,
    /// Human-readable health assessment.
    pub assessment: String,
}

/// Per-peer drop summary included in the network health response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerDropSummary {
    /// Remote peer IP address.
    pub peer_ip: String,
    /// Total TCP RST packets on SCP connections.
    pub rst_total: u64,
    /// TCP RSTs during the handshake phase (cryptographic failures).
    pub handshake_fail_total: u64,
    /// TCP retransmit events.
    pub retransmit_total: u64,
    /// Total inbound SCP segments received.
    pub packets_rx: u64,
    /// Computed drop rate as a percentage.
    pub drop_rate_pct: f64,
    /// Whether this peer is currently degraded.
    pub is_degraded: bool,
    /// Fault classification: `"cloud_network"` or `"stellar_protocol"`.
    pub fault_class: String,
    /// ISO-8601 timestamp of the most recent event from this peer.
    pub last_updated: String,
}

impl From<&PeerNetworkStats> for PeerDropSummary {
    fn from(p: &PeerNetworkStats) -> Self {
        let fault_class = if p.handshake_fail_total > 0 {
            "stellar_protocol".to_owned()
        } else {
            "cloud_network".to_owned()
        };
        Self {
            peer_ip: p.peer_ip.clone(),
            rst_total: p.rst_total,
            handshake_fail_total: p.handshake_fail_total,
            retransmit_total: p.retransmit_total,
            packets_rx: p.packets_rx,
            drop_rate_pct: p.drop_rate_pct,
            is_degraded: p.is_degraded,
            fault_class,
            last_updated: p.last_updated.clone(),
        }
    }
}

/// Response body for `GET /api/v1/scp/peer-drops`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScpPeerDropsResponse {
    /// Whether the sniffer exporter was reachable on the last poll.
    pub sniffer_available: bool,
    /// ISO-8601 timestamp of the last successful scrape.
    pub last_scraped: String,
    /// All per-peer statistics, sorted by drop rate descending.
    pub peers: Vec<PeerDropSummary>,
    /// Total number of unique peers.
    pub total: usize,
    /// Number of degraded peers.
    pub degraded: usize,
}

// ─── Handler: GET /api/v1/scp/network-health ──────────────────────────────────

/// Return the overall SCP network health snapshot.
///
/// Reads directly from the in-memory [`ScpNetworkHealthState`] store, avoiding
/// any additional HTTP hop to the sniffer exporter.  The background monitor
/// refreshes this store every 10 seconds.
#[instrument(skip(state))]
pub async fn scp_network_health(
    Extension(state): Extension<Arc<RwLock<ScpNetworkHealthState>>>,
) -> Result<Json<ScpNetworkHealthResponse>, (StatusCode, Json<ErrorResponse>)> {
    let snap = match state.read() {
        Ok(s) => s.clone(),
        Err(e) => {
            warn!("Failed to read ScpNetworkHealthState: {e}");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "state_lock_poisoned",
                    "SCP sniffer state lock is poisoned",
                )),
            ));
        }
    };

    // Build sorted top-drop list (descending by drop rate, max 10).
    let mut top: Vec<PeerDropSummary> = snap
        .peers
        .values()
        .filter(|p| p.is_degraded || p.rst_total > 0)
        .map(PeerDropSummary::from)
        .collect();
    top.sort_by(|a, b| {
        b.drop_rate_pct
            .partial_cmp(&a.drop_rate_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    top.truncate(10);

    let assessment = build_assessment(snap.health_score, snap.degraded_peers, snap.total_peers);

    Ok(Json(ScpNetworkHealthResponse {
        sniffer_available: snap.sniffer_available,
        last_scraped: snap.last_scraped,
        health_score: snap.health_score,
        total_peers: snap.total_peers,
        degraded_peers: snap.degraded_peers,
        top_drop_peers: top,
        assessment,
    }))
}

// ─── Handler: GET /api/v1/scp/peer-drops ──────────────────────────────────────

/// Return drop statistics for all observed SCP peers.
///
/// The response is sorted by drop rate descending so operators immediately see
/// the most problematic peers at the top.
#[instrument(skip(state))]
pub async fn scp_peer_drops(
    Extension(state): Extension<Arc<RwLock<ScpNetworkHealthState>>>,
) -> Result<Json<ScpPeerDropsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let snap = match state.read() {
        Ok(s) => s.clone(),
        Err(e) => {
            warn!("Failed to read ScpNetworkHealthState: {e}");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "state_lock_poisoned",
                    "SCP sniffer state lock is poisoned",
                )),
            ));
        }
    };

    let total = snap.peers.len();
    let degraded = snap.peers.values().filter(|p| p.is_degraded).count();

    let mut peers: Vec<PeerDropSummary> = snap.peers.values().map(PeerDropSummary::from).collect();
    peers.sort_by(|a, b| {
        b.drop_rate_pct
            .partial_cmp(&a.drop_rate_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(Json(ScpPeerDropsResponse {
        sniffer_available: snap.sniffer_available,
        last_scraped: snap.last_scraped,
        peers,
        total,
        degraded,
    }))
}

// ─── Handler: GET /api/v1/scp/peer-drops/:peer_ip ─────────────────────────────

/// Return drop statistics for a single peer identified by IP address.
///
/// The peer IP should be URL-encoded if it contains colons (IPv6), though the
/// sniffer currently only supports IPv4 peers.
#[instrument(skip(state), fields(peer_ip = %peer_ip))]
pub async fn scp_peer_drop_detail(
    Extension(state): Extension<Arc<RwLock<ScpNetworkHealthState>>>,
    Path(peer_ip): Path<String>,
) -> Result<Json<PeerDropSummary>, (StatusCode, Json<ErrorResponse>)> {
    let snap = match state.read() {
        Ok(s) => s.clone(),
        Err(e) => {
            warn!("Failed to read ScpNetworkHealthState: {e}");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "state_lock_poisoned",
                    "SCP sniffer state lock is poisoned",
                )),
            ));
        }
    };

    match snap.peers.get(&peer_ip) {
        Some(peer) => Ok(Json(PeerDropSummary::from(peer))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "peer_not_found",
                &format!(
                    "No SCP drop data found for peer {peer_ip}. \
                     The peer may not have been observed yet or the sniffer is unavailable."
                ),
            )),
        )),
    }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Build a human-readable health assessment string for the network health response.
fn build_assessment(health_score: u8, degraded_peers: usize, total_peers: usize) -> String {
    if total_peers == 0 {
        return "SCP sniffer has not observed any peers yet. \
                Ensure the sniffer DaemonSet is running and the \
                stellar-core nodes are using port 11625."
            .to_owned();
    }
    if degraded_peers == 0 {
        return format!(
            "All {total_peers} SCP peer connection(s) are healthy. \
             No TCP RST packets or handshake failures detected."
        );
    }
    let word = if health_score >= 80 {
        "minor"
    } else if health_score >= 50 {
        "moderate"
    } else {
        "severe"
    };
    format!(
        "{word} SCP network degradation detected: {degraded_peers} of {total_peers} peer(s) \
         are experiencing drops (health score: {health_score}/100). \
         Check the top_drop_peers list — peers with fault_class='cloud_network' indicate \
         firewall/ACL issues; 'stellar_protocol' indicates TLS or SCP version mismatches.",
        word = capitalize(word),
    )
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_drop_summary_fault_class_cloud() {
        let stats = PeerNetworkStats {
            peer_ip: "10.0.0.1".into(),
            rst_total: 5,
            handshake_fail_total: 0,
            retransmit_total: 0,
            packets_rx: 50,
            drop_rate_pct: 10.0,
            is_degraded: true,
            last_updated: "2026-01-01T00:00:00Z".into(),
        };
        let summary = PeerDropSummary::from(&stats);
        assert_eq!(summary.fault_class, "cloud_network");
    }

    #[test]
    fn peer_drop_summary_fault_class_stellar_protocol() {
        let stats = PeerNetworkStats {
            peer_ip: "10.0.0.2".into(),
            rst_total: 1,
            handshake_fail_total: 2,
            retransmit_total: 0,
            packets_rx: 10,
            drop_rate_pct: 10.0,
            is_degraded: true,
            last_updated: "2026-01-01T00:00:00Z".into(),
        };
        let summary = PeerDropSummary::from(&stats);
        assert_eq!(summary.fault_class, "stellar_protocol");
    }

    #[test]
    fn assessment_no_peers() {
        let s = build_assessment(100, 0, 0);
        assert!(s.contains("not observed any peers"));
    }

    #[test]
    fn assessment_healthy() {
        let s = build_assessment(100, 0, 5);
        assert!(s.contains("healthy"));
        assert!(s.contains("5 SCP peer"));
    }

    #[test]
    fn assessment_degraded_severe() {
        let s = build_assessment(20, 4, 5);
        assert!(s.contains("Severe") || s.contains("severe"));
        assert!(s.contains("4 of 5"));
    }

    #[test]
    fn assessment_degraded_minor() {
        let s = build_assessment(90, 1, 10);
        assert!(s.contains("Minor") || s.contains("minor"));
    }
}
