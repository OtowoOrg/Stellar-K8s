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
//! Validator peer-connectivity diagnostics.
//!
//! When a validator cannot reach its peers there is normally no signal at all:
//! `stellar-core` logs a failed overlay connection, the pod stays `Ready`, and
//! the node silently falls out of quorum. The usual causes — a blocked peer
//! port, a wrong port, a DNS failure, a stale `KNOWN_PEERS` list — are all
//! cheap to detect and all produce the same symptom, which is what makes them
//! expensive to diagnose.
//!
//! This module turns that into a first-class signal:
//!
//! - [`PeerEndpoint`] / [`parse_known_peers`] recover the peer list from the
//!   `KNOWN_PEERS` key of a generated `stellar-core.cfg`.
//! - [`probe_peers`] performs bounded-concurrency TCP dials and returns a
//!   [`PeerConnectivityReport`] carrying the address, port, outcome and last
//!   attempt time for every peer.
//! - [`connectivity_condition`] renders that report as a `PeerConnectivity`
//!   status condition on the `StellarNode`.
//! - [`remediation_hint`] turns an unreachable peer into a concrete next step
//!   (port number, protocol, and whether to suspect DNS, a security group or a
//!   stale peer list).
//!
//! The default probe interval is [`DEFAULT_INTERVAL_SECS`], which keeps the
//! loop comfortably inside the 60 s detection budget.
//!
//! This is the read side of peer management. [`super::peer_discovery`] is the
//! write side: it publishes the cluster-wide peer list. This module consumes a
//! peer's configured `KNOWN_PEERS` and reports whether each one answers.

use std::time::{Duration, Instant};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tracing::{debug, warn};

use crate::controller::conditions::{
    CONDITION_STATUS_FALSE, CONDITION_STATUS_TRUE, CONDITION_STATUS_UNKNOWN,
    CONDITION_TYPE_PEER_CONNECTIVITY,
};
use crate::crd::{Condition, NodeType, StellarNode};

/// Default `stellar-core` peer-to-peer port.
///
/// This is the port overlay traffic uses; `11626` is the HTTP/admin port and is
/// *not* a peer port, so mistaking the two is a common self-inflicted outage.
pub const DEFAULT_PEER_PORT: u16 = 11625;

/// `stellar-core` HTTP/admin port, used to suggest port correction when a peer
/// is listed on the wrong port.
pub const CORE_HTTP_PORT: u16 = 11626;

/// Default seconds between connectivity probe rounds.
///
/// Two rounds fit inside the 60 s detection budget even when a round is
/// followed immediately by a slow dial.
pub const DEFAULT_INTERVAL_SECS: u64 = 30;

/// Default per-peer dial timeout.
pub const DEFAULT_PROBE_TIMEOUT_SECS: u64 = 3;

/// Upper bound on how many peers are dialed at the same time.
pub const MAX_CONCURRENT_PROBES: usize = 8;

/// Condition reason set when every configured peer is unreachable.
pub const REASON_ALL_PEERS_UNREACHABLE: &str = "AllPeersUnreachable";

/// Condition reason set when at least one configured peer is unreachable.
pub const REASON_PARTIAL_PEER_LOSS: &str = "PartialPeerLoss";

/// Condition reason set when every configured peer answered.
pub const REASON_ALL_PEERS_REACHABLE: &str = "AllPeersReachable";

/// Condition reason set when no peers are configured to probe.
pub const REASON_NO_PEERS_CONFIGURED: &str = "NoPeersConfigured";

/// One configured peer, split into the parts a diagnostic needs.
///
/// `Ord` exists so the probe list can be sorted and de-duplicated: the same
/// peer can be named by `KNOWN_PEERS` and by the quorum set, and a report that
/// listed it twice would read as two failures.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct PeerEndpoint {
    /// Hostname or IP literal of the peer.
    pub address: String,
    /// TCP port the peer listens on for overlay traffic.
    pub port: u16,
}

impl PeerEndpoint {
    /// Build an endpoint from its parts.
    pub fn new(address: impl Into<String>, port: u16) -> Self {
        Self {
            address: address.into(),
            port,
        }
    }

    /// Parse a `host`, `host:port` or `[v6]:port` peer specification.
    ///
    /// A bare host falls back to [`DEFAULT_PEER_PORT`] so a
    /// `KNOWN_PEERS=["peer.example.com"]` entry still probes the right port
    /// instead of the HTTP port.
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        if let Some(rest) = spec.strip_prefix('[') {
            // Bracketed IPv6 literal: "[::1]:11625"
            let (host, tail) = rest.split_once(']')?;
            let port = match tail.strip_prefix(':') {
                Some(p) => p.parse().ok()?,
                None => DEFAULT_PEER_PORT,
            };
            return Some(Self::new(host, port));
        }

        match spec.rsplit_once(':') {
            // More than one colon means a bare IPv6 literal, which has no port.
            Some(_) if spec.matches(':').count() > 1 => Some(Self::new(spec, DEFAULT_PEER_PORT)),
            Some((host, port)) => Some(Self::new(host, port.parse().ok()?)),
            None => Some(Self::new(spec, DEFAULT_PEER_PORT)),
        }
    }

    /// Render as the `host:port` form used by `stellar-core`.
    pub fn to_peer_string(&self) -> String {
        format!("{}:{}", self.address, self.port)
    }

    /// Value passed to `TcpStream::connect`.
    fn to_socket_target(&self) -> (&str, u16) {
        (self.address.as_str(), self.port)
    }
}

/// Outcome of a single TCP dial against one peer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PeerProbeResult {
    /// Peer address that was probed.
    pub address: String,
    /// Port that was probed.
    pub port: u16,
    /// Whether the TCP handshake completed.
    pub reachable: bool,
    /// Failure detail when `reachable` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Unix timestamp (seconds) of this attempt.
    pub last_attempt: i64,
    /// Round-trip time of the handshake in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

impl PeerProbeResult {
    /// Render as `host:port` for log and condition messages.
    pub fn to_peer_string(&self) -> String {
        format!("{}:{}", self.address, self.port)
    }
}

/// A full round of peer probes.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PeerConnectivityReport {
    /// One entry per configured peer, in configuration order.
    #[serde(default)]
    pub peers: Vec<PeerProbeResult>,
    /// Unix timestamp (seconds) when this round completed.
    pub last_check: i64,
    /// Seconds between rounds, carried so consumers can reason about staleness.
    pub interval_secs: u64,
}

impl PeerConnectivityReport {
    /// Number of peers that answered.
    pub fn reachable_count(&self) -> usize {
        self.peers.iter().filter(|p| p.reachable).count()
    }

    /// Number of peers that did not answer.
    pub fn unreachable_count(&self) -> usize {
        self.peers.len() - self.reachable_count()
    }

    /// Every peer that did not answer.
    pub fn unreachable(&self) -> Vec<&PeerProbeResult> {
        self.peers.iter().filter(|p| !p.reachable).collect()
    }

    /// True when peers are configured and none of them answered.
    ///
    /// This is the condition under which the sidecar reports the node as
    /// degraded: with no reachable peer the validator cannot complete SCP.
    pub fn is_fully_degraded(&self) -> bool {
        !self.peers.is_empty() && self.unreachable_count() == self.peers.len()
    }

    /// True when at least one peer answered.
    pub fn is_healthy(&self) -> bool {
        self.reachable_count() > 0
    }

    /// One-line summary suitable for logs.
    pub fn summary(&self) -> String {
        format!(
            "peers={} reachable={} unreachable={} checked_at={}",
            self.peers.len(),
            self.reachable_count(),
            self.unreachable_count(),
            self.last_check
        )
    }
}

/// Extract peers from a `KNOWN_PEERS` TOML value.
///
/// Accepts both a bare array fragment (`KNOWN_PEERS=["a:11625"]`) and a full
/// document containing that key, and tolerates malformed TOML by returning an
/// empty list rather than failing a reconcile.
pub fn parse_known_peers(known_peers_toml: &str) -> Vec<PeerEndpoint> {
    let Ok(value) = known_peers_toml.trim().parse::<toml::Value>() else {
        return Vec::new();
    };

    let entries = value
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .chain(
            value
                .get("KNOWN_PEERS")
                .and_then(toml::Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );

    let mut peers: Vec<PeerEndpoint> = entries
        .into_iter()
        .filter_map(|v| v.as_str().and_then(PeerEndpoint::parse))
        .collect();
    peers.sort();
    peers.dedup();
    peers
}

/// Collect the peers a `StellarNode` should be able to reach.
///
/// For validators this is the `KNOWN_PEERS` list, plus any `ip:port` entries
/// embedded in the quorum set. Non-validator nodes have no overlay peers.
pub fn known_peers_for_node(node: &StellarNode) -> Vec<PeerEndpoint> {
    if node.spec.node_type != NodeType::Validator {
        return Vec::new();
    }

    let Some(config) = node.spec.validator_config.as_ref() else {
        return Vec::new();
    };

    let mut peers = config
        .known_peers
        .as_deref()
        .map(parse_known_peers)
        .unwrap_or_default();

    peers.extend(peers_from_quorum_set(config.quorum_set.as_deref()));
    peers.sort();
    peers.dedup();
    peers
}

/// Pull `host:port`-shaped entries out of a user-supplied `[QUORUM_SET]`.
///
/// `VALIDATORS` shows up in three different shapes depending on how the user
/// wrote it, and all three are common:
///
/// - a plain array of keys, `VALIDATORS=["GCEZW7", "validator2:11625"]`
/// - an array of tables, `[[VALIDATORS]] ADDRESS="validator2:11625"`
/// - a table keyed by name, `VALIDATORS.validator2="validator2:11625"`
///
/// Only entries that actually look like network endpoints are accepted, so a
/// Stellar public key is never mistaken for an address. Public keys start with
/// `G` and are base32, never dotted, which is what the filter keys off.
fn peers_from_quorum_set(quorum_set: Option<&str>) -> Vec<PeerEndpoint> {
    let Some(raw) = quorum_set else {
        return Vec::new();
    };
    let Ok(value) = raw.trim().parse::<toml::Value>() else {
        return Vec::new();
    };

    let Some(validators) = value.get("VALIDATORS") else {
        return Vec::new();
    };

    let mut peers = Vec::new();

    match validators {
        toml::Value::Array(items) => {
            for item in items {
                match item {
                    toml::Value::String(entry) => {
                        if let Some(peer) = endpoint_from_entry(entry) {
                            peers.push(peer);
                        }
                    }
                    toml::Value::Table(table) => {
                        if let Some(address) = table.get("ADDRESS").and_then(toml::Value::as_str) {
                            if let Some(peer) = endpoint_from_entry(address) {
                                peers.push(peer);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        toml::Value::Table(table) => {
            for entry in table.values() {
                if let Some(address) = entry.as_str() {
                    if let Some(peer) = endpoint_from_entry(address) {
                        peers.push(peer);
                    }
                }
            }
        }
        _ => {}
    }

    peers.sort();
    peers.dedup();
    peers
}

/// Accept `entry` as a peer address, rejecting Stellar public keys.
fn endpoint_from_entry(entry: &str) -> Option<PeerEndpoint> {
    if entry.starts_with('G') || !entry.contains('.') {
        return None;
    }
    PeerEndpoint::parse(entry)
}

/// Dial a single peer and record the outcome.
///
/// A failed dial is data, not an error: the returned result always describes
/// what happened so the caller can surface it.
pub async fn probe_peer(endpoint: PeerEndpoint, timeout: Duration) -> PeerProbeResult {
    let started = Instant::now();
    let target = endpoint.to_socket_target();

    let (reachable, error) = match tokio::time::timeout(timeout, TcpStream::connect(target)).await {
        Ok(Ok(_stream)) => (true, None),
        Ok(Err(e)) => (false, Some(e.to_string())),
        Err(_) => (false, Some("dial timed out".to_string())),
    };

    PeerProbeResult {
        address: endpoint.address.clone(),
        port: endpoint.port,
        reachable,
        error,
        last_attempt: chrono::Utc::now().timestamp(),
        latency_ms: Some(started.elapsed().as_millis() as u64),
    }
}

/// Dial every peer, at most [`MAX_CONCURRENT_PROBES`] at a time.
///
/// Probes are independent, so a single blocked host cannot delay the rest: the
/// worst case is one timeout, not `N * timeout`.
pub async fn probe_peers(
    peers: &[PeerEndpoint],
    timeout: Duration,
    interval_secs: u64,
) -> PeerConnectivityReport {
    let owned: Vec<PeerEndpoint> = peers.iter().cloned().collect();
    let mut in_flight = futures::stream::iter(owned)
        .map(|peer| probe_peer(peer, timeout))
        .buffer_unordered(MAX_CONCURRENT_PROBES)
        .peekable();

    let mut results = Vec::with_capacity(peers.len());
    while let Some(result) = in_flight.next().await {
        results.push(result);
    }

    // `buffer_unordered` yields in completion order; restore configuration
    // order so the report and the condition message are stable across rounds.
    let order: std::collections::HashMap<(String, u16), usize> = peers
        .iter()
        .enumerate()
        .map(|(idx, p)| ((p.address.clone(), p.port), idx))
        .collect();
    results.sort_by_key(|r| order.get(&(r.address.clone(), r.port)).copied());

    PeerConnectivityReport {
        peers: results,
        last_check: chrono::Utc::now().timestamp(),
        interval_secs,
    }
}

/// Probe peers forever, refreshing the shared report on every round.
///
/// The first round runs immediately so the sidecar has data well inside the
/// 60 s detection budget instead of waiting out the first interval.
pub async fn peer_monitor_loop(
    peers: Vec<PeerEndpoint>,
    interval: Duration,
    timeout: Duration,
    slot: std::sync::Arc<tokio::sync::RwLock<Option<PeerConnectivityReport>>>,
) {
    let interval_secs = interval.as_secs().max(1);

    loop {
        let report = probe_peers(&peers, timeout, interval_secs).await;
        if report.is_fully_degraded() {
            warn!(
                "all {} configured peers are unreachable: {}",
                report.peers.len(),
                report.summary()
            );
        } else {
            debug!("peer connectivity round: {}", report.summary());
        }
        *slot.write().await = Some(report);

        tokio::time::sleep(interval).await;
    }
}

/// A concrete next step for one unreachable peer.
///
/// The message names the port, the protocol, and the two most likely causes so
/// an on-call engineer does not have to rediscover them: a security-group or
/// NetworkPolicy rule blocking the overlay port, or a peer entry pointing at
/// the wrong port / a stale address.
pub fn remediation_hint(result: &PeerProbeResult) -> String {
    let mut hint = format!(
        "peer {} (TCP port {}) is unreachable",
        result.to_peer_string(),
        result.port
    );

    if let Some(error) = &result.error {
        hint.push_str(&format!(": {error}"));
    }

    if result.port == CORE_HTTP_PORT {
        hint.push_str(
            ". Port 11626 is the stellar-core HTTP/admin port, not the overlay port; \
             use 11625 for peer-to-peer connectivity",
        );
    } else if result.port != DEFAULT_PEER_PORT {
        hint.push_str(&format!(
            ". Non-default peer port {}; confirm it matches the peer's PEER_PORT and that \
             the same port is open inbound and outbound in the security group, the \
             NetworkPolicy and any cloud firewall",
            result.port
        ));
    } else {
        hint.push_str(
            ". Allow TCP 11625 inbound and outbound in the security group, the NetworkPolicy \
             and any cloud firewall, and confirm the peer host resolves from this pod",
        );
    }

    if result.port == DEFAULT_PEER_PORT {
        hint.push_str("; if the entry is stale, refresh KNOWN_PEERS from the validator list");
    }

    hint
}

/// Human-readable message for a whole report, including per-peer detail.
pub fn connectivity_message(report: &PeerConnectivityReport) -> String {
    if report.peers.is_empty() {
        return "No peers configured for connectivity probing".to_string();
    }

    if report.unreachable_count() == 0 {
        return format!("All {} configured peers reachable", report.peers.len());
    }

    let detail = report
        .unreachable()
        .iter()
        .map(|p| remediation_hint(p))
        .collect::<Vec<_>>()
        .join("; ");

    format!(
        "{}/{} peers unreachable. {detail}",
        report.unreachable_count(),
        report.peers.len()
    )
}

/// Aggregate verdict for a report, in the shape `conditions::set_condition` wants.
///
/// Kept separate from [`connectivity_condition`] so the reconciler can feed the
/// existing condition helper, which owns `last_transition_time` bookkeeping and
/// therefore does not churn the status on every unchanged round.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectivityVerdict {
    /// `True`, `False` or `Unknown`.
    pub status: &'static str,
    /// Why the verdict was reached.
    pub reason: &'static str,
    /// Human-readable summary including per-peer detail.
    pub message: String,
}

/// Classify a report into a status/reason/message triple.
pub fn connectivity_verdict(report: &PeerConnectivityReport) -> ConnectivityVerdict {
    let (status, reason) = if report.peers.is_empty() {
        (CONDITION_STATUS_UNKNOWN, REASON_NO_PEERS_CONFIGURED)
    } else if report.is_fully_degraded() {
        (CONDITION_STATUS_FALSE, REASON_ALL_PEERS_UNREACHABLE)
    } else if report.unreachable_count() > 0 {
        (CONDITION_STATUS_TRUE, REASON_PARTIAL_PEER_LOSS)
    } else {
        (CONDITION_STATUS_TRUE, REASON_ALL_PEERS_REACHABLE)
    };

    ConnectivityVerdict {
        status,
        reason,
        message: connectivity_message(report),
    }
}

/// Render a report as a `PeerConnectivity` status condition.
///
/// The condition carries the aggregate verdict in `status`, the class of
/// failure in `reason`, and the per-peer detail (address, port, last attempt)
/// in `message`, so `kubectl describe` alone is enough to act on it.
pub fn connectivity_condition(report: &PeerConnectivityReport) -> Condition {
    let verdict = connectivity_verdict(report);
    Condition {
        type_: CONDITION_TYPE_PEER_CONNECTIVITY.to_string(),
        status: verdict.status.to_string(),
        last_transition_time: chrono::Utc::now().to_rfc3339(),
        reason: verdict.reason.to_string(),
        message: verdict.message,
        observed_generation: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{StellarNode, StellarNodeSpec};

    fn validator_with_known_peers(known_peers: &str) -> StellarNode {
        let mut node = StellarNode::new(
            "peer-diag",
            StellarNodeSpec {
                node_type: NodeType::Validator,
                ..Default::default()
            },
        );
        node.spec.validator_config = Some(crate::crd::ValidatorConfig {
            known_peers: Some(known_peers.to_string()),
            ..Default::default()
        });
        node
    }

    #[test]
    fn parses_host_with_explicit_port() {
        let peer = PeerEndpoint::parse("10.0.0.11:11625").expect("parses");
        assert_eq!(peer, PeerEndpoint::new("10.0.0.11", 11625));
        assert_eq!(peer.to_peer_string(), "10.0.0.11:11625");
    }

    #[test]
    fn parses_host_without_port_using_peer_default() {
        let peer = PeerEndpoint::parse("peer.example.com").expect("parses");
        assert_eq!(
            peer,
            PeerEndpoint::new("peer.example.com", DEFAULT_PEER_PORT)
        );
    }

    #[test]
    fn parses_bracketed_ipv6_with_and_without_port() {
        assert_eq!(
            PeerEndpoint::parse("[fd00::1]:11626"),
            Some(PeerEndpoint::new("fd00::1", 11626))
        );
        assert_eq!(
            PeerEndpoint::parse("[fd00::1]"),
            Some(PeerEndpoint::new("fd00::1", DEFAULT_PEER_PORT))
        );
        // A bare IPv6 literal has no port and must not be split on ':'.
        assert_eq!(
            PeerEndpoint::parse("fd00::1"),
            Some(PeerEndpoint::new("fd00::1", DEFAULT_PEER_PORT))
        );
    }

    #[test]
    fn rejects_empty_and_unparsable_entries() {
        assert_eq!(PeerEndpoint::parse(""), None);
        assert_eq!(PeerEndpoint::parse("   "), None);
        assert_eq!(PeerEndpoint::parse("host:not-a-port"), None);
    }

    #[test]
    fn parses_known_peers_from_bare_array_and_full_document() {
        let bare = r#"KNOWN_PEERS=["10.0.0.1:11625","10.0.0.2"]"#;
        let peers = parse_known_peers(bare);
        assert_eq!(
            peers,
            vec![
                PeerEndpoint::new("10.0.0.1", 11625),
                PeerEndpoint::new("10.0.0.2", DEFAULT_PEER_PORT),
            ]
        );

        let wrapped = format!("# comment\nKNOWN_PEERS=[\"10.0.0.9:11625\"]\n");
        assert_eq!(
            parse_known_peers(&wrapped),
            vec![PeerEndpoint::new("10.0.0.9", 11625)]
        );
    }

    #[test]
    fn known_peers_parsing_tolerates_garbage() {
        assert!(parse_known_peers("this is = = not toml").is_empty());
        assert!(parse_known_peers("").is_empty());
    }

    #[test]
    fn known_peers_are_deduplicated_and_sorted() {
        let raw = r#"KNOWN_PEERS=["b:11625","a:11625","b:11625"]"#;
        assert_eq!(
            parse_known_peers(raw),
            vec![PeerEndpoint::new("a", 11625), PeerEndpoint::new("b", 11625)]
        );
    }

    #[test]
    fn only_validators_have_overlay_peers() {
        let validator = validator_with_known_peers(r#"KNOWN_PEERS=["a:11625"]"#);
        assert_eq!(
            known_peers_for_node(&validator),
            vec![PeerEndpoint::new("a", 11625)]
        );

        let mut horizon = StellarNode::new(
            "h",
            StellarNodeSpec {
                node_type: NodeType::Horizon,
                ..Default::default()
            },
        );
        horizon.spec.horizon_config = Some(crate::crd::HorizonConfig {
            stellar_core_url: "http://core:8000".to_string(),
            ..Default::default()
        });
        assert!(known_peers_for_node(&horizon).is_empty());
    }

    #[test]
    fn peers_from_quorum_set_ignore_public_keys() {
        let qs = r#"
[QUORUM_SET]
THRESHOLD_PERCENT=67
VALIDATORS=["GCEZW7","GCB2F5","10.0.0.7:11625"]
"#;
        let peers = peers_from_quorum_set(Some(qs));
        assert_eq!(peers, vec![PeerEndpoint::new("10.0.0.7", 11625)]);
    }

    #[test]
    fn peers_from_quorum_set_reads_array_of_validators_tables() {
        let qs = r#"
[QUORUM_SET]
THRESHOLD_PERCENT=67
VALIDATORS=["GCEZW7"]

[[VALIDATORS]]
HOME_DOMAINS=["validator2.example.com"]
NAME="validator2"
PUBLIC_KEY="GCEZW7"
ADDRESS="validator2.example.com:11625"
"#;
        let peers = peers_from_quorum_set(Some(qs));
        assert_eq!(
            peers,
            vec![PeerEndpoint::new("validator2.example.com", 11625)]
        );
    }

    #[test]
    fn peers_from_quorum_set_reads_name_keyed_table() {
        let qs = r#"
[QUORUM_SET]
THRESHOLD_PERCENT=67
[QUORUM_SET.VALIDATORS]
validator1="10.0.0.1:11625"
validator2="GCEZW7"
"#;
        let peers = peers_from_quorum_set(Some(qs));
        assert_eq!(peers, vec![PeerEndpoint::new("10.0.0.1", 11625)]);
    }

    #[test]
    fn peers_from_quorum_set_tolerates_junk() {
        assert!(peers_from_quorum_set(None).is_empty());
        assert!(peers_from_quorum_set(Some("")).is_empty());
        assert!(peers_from_quorum_set(Some("= = not toml")).is_empty());
        assert!(peers_from_quorum_set(Some("[QUORUM_SET]\nVALIDATORS=42\n")).is_empty());
    }

    #[test]
    fn report_counters_cover_all_outcomes() {
        let report = PeerConnectivityReport {
            peers: vec![
                PeerProbeResult {
                    address: "a".into(),
                    port: 11625,
                    reachable: true,
                    error: None,
                    last_attempt: 1,
                    latency_ms: Some(3),
                },
                PeerProbeResult {
                    address: "b".into(),
                    port: 11625,
                    reachable: false,
                    error: Some("connection refused".into()),
                    last_attempt: 1,
                    latency_ms: Some(3),
                },
            ],
            last_check: 10,
            interval_secs: 30,
        };

        assert_eq!(report.reachable_count(), 1);
        assert_eq!(report.unreachable_count(), 1);
        assert!(report.is_healthy());
        assert!(!report.is_fully_degraded());
        assert_eq!(report.unreachable()[0].address, "b");
        assert!(report.summary().contains("reachable=1"));
    }

    #[test]
    fn report_with_no_peers_is_neither_healthy_nor_degraded() {
        let report = PeerConnectivityReport {
            last_check: 1,
            interval_secs: 30,
            ..Default::default()
        };
        assert!(!report.is_healthy());
        assert!(!report.is_fully_degraded());
        assert_eq!(report.unreachable_count(), 0);
    }

    #[test]
    fn all_unreachable_report_is_degraded() {
        let report = PeerConnectivityReport {
            peers: vec![PeerProbeResult {
                address: "a".into(),
                port: 11625,
                reachable: false,
                error: Some("timed out".into()),
                last_attempt: 1,
                latency_ms: Some(3000),
            }],
            last_check: 5,
            interval_secs: 30,
        };
        assert!(report.is_fully_degraded());
        assert!(!report.is_healthy());
    }

    #[test]
    fn condition_is_unknown_without_peers() {
        let report = PeerConnectivityReport {
            last_check: 1,
            interval_secs: 30,
            ..Default::default()
        };
        let condition = connectivity_condition(&report);
        assert_eq!(condition.type_, CONDITION_TYPE_PEER_CONNECTIVITY);
        assert_eq!(condition.status, "Unknown");
        assert_eq!(condition.reason, REASON_NO_PEERS_CONFIGURED);
    }

    #[test]
    fn condition_is_false_when_every_peer_is_unreachable() {
        let report = PeerConnectivityReport {
            peers: vec![PeerProbeResult {
                address: "10.0.0.4".into(),
                port: DEFAULT_PEER_PORT,
                reachable: false,
                error: Some("timed out".into()),
                last_attempt: 1_700_000_000,
                latency_ms: Some(3000),
            }],
            last_check: 1_700_000_000,
            interval_secs: 30,
        };
        let condition = connectivity_condition(&report);
        assert_eq!(condition.status, "False");
        assert_eq!(condition.reason, REASON_ALL_PEERS_UNREACHABLE);
        assert!(condition.message.contains("10.0.0.4:11625"));
        assert!(condition.message.contains("11625"));
    }

    #[test]
    fn condition_is_true_with_partial_peer_loss() {
        let report = PeerConnectivityReport {
            peers: vec![
                PeerProbeResult {
                    address: "a".into(),
                    port: 11625,
                    reachable: true,
                    error: None,
                    last_attempt: 1,
                    latency_ms: Some(1),
                },
                PeerProbeResult {
                    address: "b".into(),
                    port: 11625,
                    reachable: false,
                    error: None,
                    last_attempt: 1,
                    latency_ms: Some(1),
                },
            ],
            last_check: 1,
            interval_secs: 30,
        };
        let condition = connectivity_condition(&report);
        assert_eq!(condition.status, "True");
        assert_eq!(condition.reason, REASON_PARTIAL_PEER_LOSS);
    }

    #[test]
    fn condition_is_true_when_all_peers_reachable() {
        let report = PeerConnectivityReport {
            peers: vec![PeerProbeResult {
                address: "a".into(),
                port: 11625,
                reachable: true,
                error: None,
                last_attempt: 1,
                latency_ms: Some(1),
            }],
            last_check: 1,
            interval_secs: 30,
        };
        let condition = connectivity_condition(&report);
        assert_eq!(condition.status, "True");
        assert_eq!(condition.reason, REASON_ALL_PEERS_REACHABLE);
    }

    #[test]
    fn remediation_hint_names_port_and_security_group() {
        let blocked = PeerProbeResult {
            address: "10.0.0.5".into(),
            port: DEFAULT_PEER_PORT,
            reachable: false,
            error: Some("connection timed out".into()),
            last_attempt: 1,
            latency_ms: Some(3000),
        };
        let hint = remediation_hint(&blocked);
        assert!(hint.contains("10.0.0.5:11625"), "{hint}");
        assert!(hint.contains("TCP port 11625"), "{hint}");
        assert!(hint.contains("security group"), "{hint}");
        assert!(hint.contains("NetworkPolicy"), "{hint}");
        assert!(hint.contains("connection timed out"), "{hint}");
        assert!(hint.contains("KNOWN_PEERS"), "{hint}");
    }

    #[test]
    fn remediation_hint_flags_core_http_port_misuse() {
        let wrong_port = PeerProbeResult {
            address: "10.0.0.6".into(),
            port: CORE_HTTP_PORT,
            reachable: false,
            error: Some("connection refused".into()),
            last_attempt: 1,
            latency_ms: Some(1),
        };
        let hint = remediation_hint(&wrong_port);
        assert!(
            hint.contains("11626 is the stellar-core HTTP/admin port"),
            "{hint}"
        );
        assert!(hint.contains("11625"), "{hint}");
    }

    #[test]
    fn remediation_hint_handles_custom_port() {
        let custom = PeerProbeResult {
            address: "peer.example.com".into(),
            port: 2222,
            reachable: false,
            error: None,
            last_attempt: 1,
            latency_ms: Some(1),
        };
        let hint = remediation_hint(&custom);
        assert!(hint.contains("peer.example.com:2222"), "{hint}");
        assert!(hint.contains("PEER_PORT"), "{hint}");
    }

    #[tokio::test]
    async fn probe_against_a_listening_socket_succeeds() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let accepted = tokio::spawn(async move { listener.accept().await });

        let peers = vec![PeerEndpoint::new("127.0.0.1", port)];
        let report = probe_peers(&peers, Duration::from_secs(2), 30).await;

        assert_eq!(report.peers.len(), 1);
        assert!(report.peers[0].reachable, "report: {report:?}");
        assert!(report.is_healthy());
        assert!(!report.is_fully_degraded());
        assert!(report.peers[0].last_attempt > 0);
        accepted.abort();
    }

    #[tokio::test]
    async fn probe_of_a_closed_port_is_reported_not_returned_as_error() {
        // Bind then drop so the port is almost certainly unused.
        let port = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind loopback");
            listener.local_addr().expect("local addr").port()
        };

        let peers = vec![PeerEndpoint::new("127.0.0.1", port)];
        let report = probe_peers(&peers, Duration::from_millis(500), 30).await;

        assert_eq!(report.peers.len(), 1);
        assert!(!report.peers[0].reachable);
        assert!(report.peers[0].error.is_some());
        assert!(report.is_fully_degraded());
    }

    #[tokio::test]
    async fn probe_results_keep_configuration_order() {
        let reachable = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let reachable_port = reachable.local_addr().expect("addr").port();
        let closed_port = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            l.local_addr().expect("addr").port()
        };

        let peers = vec![
            PeerEndpoint::new("127.0.0.1", reachable_port),
            PeerEndpoint::new("127.0.0.1", closed_port),
        ];
        let report = probe_peers(&peers, Duration::from_millis(500), 30).await;

        assert_eq!(report.peers[0].port, reachable_port);
        assert_eq!(report.peers[1].port, closed_port);
        assert!(report.peers[0].reachable);
        assert!(!report.peers[1].reachable);
    }

    #[tokio::test]
    async fn empty_peer_list_produces_an_empty_report() {
        let report = probe_peers(&[], Duration::from_millis(50), 30).await;
        assert!(report.peers.is_empty());
        assert_eq!(report.interval_secs, 30);
    }
}
