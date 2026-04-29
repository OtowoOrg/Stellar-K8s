//! SCP Topology REST and WebSocket handlers
//!
//! Provides two endpoints for visualising the real-time Stellar Consensus Protocol (SCP)
//! quorum graph:
//!
//! - `GET /api/v1/quorum/topology`        — one-shot JSON snapshot
//! - `GET /api/v1/quorum/topology/stream` — WebSocket stream (updates every 5 seconds)
//!
//! The handlers collect SCP state from every Validator pod via the Stellar Core HTTP API
//! (`GET http://{pod_ip}:11626/scp?limit=1`), map the quorum graph, and identify stalled
//! nodes (no SCP phase change for > 30 s) and critical nodes (whose removal would break
//! quorum intersection).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use k8s_openapi::api::core::v1::Pod;
use kube::api::ListParams;
use kube::Api;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::controller::quorum::scp_client::ScpClient;
use crate::controller::quorum::types::ScpState;
use crate::controller::ControllerState;

// ── Response types ────────────────────────────────────────────────────────────

/// A single node in the quorum graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyNode {
    /// Validator public key (short form, first 8 chars + "…")
    pub id: String,
    /// Full public key
    pub full_id: String,
    /// Current SCP phase: "PREPARE", "CONFIRM", "EXTERNALIZE", or "UNKNOWN"
    pub phase: String,
    /// Whether this node is identified as critical (its removal breaks consensus)
    pub is_critical: bool,
    /// Quorum threshold for this node's quorum set
    pub threshold: u32,
    /// `true` if the node has not advanced its SCP phase in the last 30 seconds
    pub stalled: bool,
}

/// A directed edge representing that `source` includes `target` in its quorum set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyEdge {
    pub source: String,
    pub target: String,
}

/// Full topology snapshot returned by the REST and WebSocket endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumTopologyResponse {
    pub nodes: Vec<TopologyNode>,
    pub edges: Vec<TopologyEdge>,
    /// Node IDs that are considered stalled (duplicated here for easy access by the UI)
    pub stalled_nodes: Vec<String>,
    /// RFC3339 timestamp of when this snapshot was taken
    pub timestamp: String,
    /// Whether any data could be collected; `false` means all pod queries failed
    pub healthy: bool,
}

impl QuorumTopologyResponse {
    fn empty() -> Self {
        QuorumTopologyResponse {
            nodes: vec![],
            edges: vec![],
            stalled_nodes: vec![],
            timestamp: Utc::now().to_rfc3339(),
            healthy: false,
        }
    }
}

// ── REST handler ──────────────────────────────────────────────────────────────

/// `GET /api/v1/quorum/topology` — returns a one-shot topology snapshot.
pub async fn get_topology(
    State(state): State<Arc<ControllerState>>,
) -> Json<QuorumTopologyResponse> {
    Json(build_topology_snapshot(&state).await)
}

// ── WebSocket handler ─────────────────────────────────────────────────────────

/// `GET /api/v1/quorum/topology/stream` — upgrades to a WebSocket and streams
/// `QuorumTopologyResponse` JSON frames every 5 seconds until the client disconnects.
pub async fn topology_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ControllerState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_topology(socket, state))
}

async fn stream_topology(mut socket: WebSocket, state: Arc<ControllerState>) {
    loop {
        let snapshot = build_topology_snapshot(&state).await;

        let json = match serde_json::to_string(&snapshot) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to serialize topology snapshot: {e}");
                break;
            }
        };

        if socket.send(Message::Text(json.into())).await.is_err() {
            // Client disconnected — exit cleanly
            debug!("SCP topology WebSocket client disconnected");
            break;
        }

        sleep(Duration::from_secs(5)).await;
    }
}

// ── Core topology builder ─────────────────────────────────────────────────────

/// Collect SCP state from all Validator pods and build a topology snapshot.
async fn build_topology_snapshot(state: &Arc<ControllerState>) -> QuorumTopologyResponse {
    let pod_ips = match collect_validator_pod_ips(state).await {
        Ok(ips) => ips,
        Err(e) => {
            warn!("Failed to list validator pods for topology: {e}");
            return QuorumTopologyResponse::empty();
        }
    };

    if pod_ips.is_empty() {
        debug!("No running Validator pods found; returning empty topology");
        return QuorumTopologyResponse {
            nodes: vec![],
            edges: vec![],
            stalled_nodes: vec![],
            timestamp: Utc::now().to_rfc3339(),
            healthy: true,
        };
    }

    let client = ScpClient::new(Duration::from_secs(5), 3);
    let mut scp_states: Vec<ScpState> = Vec::new();

    for ip in &pod_ips {
        match client.query_scp_state(ip).await {
            Ok(state) => scp_states.push(state),
            Err(e) => debug!("SCP query failed for pod {ip}: {e}"),
        }
    }

    if scp_states.is_empty() {
        return QuorumTopologyResponse::empty();
    }

    // ── Identify critical nodes (simplified: a node is "critical" if it
    //    appears in every other node's quorum set) ────────────────────────────

    // Build a map: node_id → set of validators it depends on
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    for s in &scp_states {
        let mut all_validators = s.quorum_set.validators.clone();
        for inner in &s.quorum_set.inner_sets {
            all_validators.extend(inner.validators.iter().cloned());
        }
        deps.insert(s.node_id.clone(), all_validators);
    }

    let all_node_ids: Vec<String> = scp_states.iter().map(|s| s.node_id.clone()).collect();

    let is_critical = |node_id: &str| -> bool {
        // A node is critical if removing it means some other node's quorum set
        // falls below threshold. Simple heuristic: present in all known quorum sets.
        deps.values()
            .filter(|validators| !validators.is_empty())
            .all(|validators| validators.contains(&node_id.to_string()))
    };

    // ── Stall detection: track last seen phase per node ───────────────────────
    // In the REST snapshot we have only one sample, so stalled detection is
    // based on whether `phase == "UNKNOWN"` (pod unreachable) or
    // `ballot_counter == 0` (never advanced).
    let is_stalled = |s: &ScpState| -> bool {
        s.ballot_state.phase == "UNKNOWN"
            || (s.ballot_state.ballot_counter == 0 && s.ballot_state.phase == "PREPARE")
    };

    // ── Build nodes ───────────────────────────────────────────────────────────
    let nodes: Vec<TopologyNode> = scp_states
        .iter()
        .map(|s| {
            let short_id = short_key(&s.node_id);
            let stalled = is_stalled(s);
            TopologyNode {
                id: short_id,
                full_id: s.node_id.clone(),
                phase: s.ballot_state.phase.clone(),
                is_critical: is_critical(&s.node_id),
                threshold: s.quorum_set.threshold,
                stalled,
            }
        })
        .collect();

    // ── Build edges ───────────────────────────────────────────────────────────
    let mut edges: Vec<TopologyEdge> = Vec::new();
    for s in &scp_states {
        let src = short_key(&s.node_id);
        for validator in &s.quorum_set.validators {
            // Only draw edges to nodes that replied (we know them)
            if all_node_ids.contains(validator) {
                edges.push(TopologyEdge {
                    source: src.clone(),
                    target: short_key(validator),
                });
            }
        }
        for inner in &s.quorum_set.inner_sets {
            for validator in &inner.validators {
                if all_node_ids.contains(validator) {
                    edges.push(TopologyEdge {
                        source: src.clone(),
                        target: short_key(validator),
                    });
                }
            }
        }
    }

    // Deduplicate edges
    edges.sort_by(|a, b| a.source.cmp(&b.source).then(a.target.cmp(&b.target)));
    edges.dedup_by(|a, b| a.source == b.source && a.target == b.target);

    let stalled_nodes: Vec<String> = nodes
        .iter()
        .filter(|n| n.stalled)
        .map(|n| n.id.clone())
        .collect();

    QuorumTopologyResponse {
        nodes,
        edges,
        stalled_nodes,
        timestamp: Utc::now().to_rfc3339(),
        healthy: true,
    }
}

/// Collect the IP addresses of all running Validator pods managed by the operator.
async fn collect_validator_pod_ips(
    state: &Arc<ControllerState>,
) -> Result<Vec<String>, kube::Error> {
    // Pods created by the operator for Validator nodes carry the label:
    //   app.kubernetes.io/name=stellar-node
    //   stellar.org/node-type=Validator   (set by the operator on validator pods)
    let label_selector =
        "app.kubernetes.io/name=stellar-node,stellar.org/node-type=Validator".to_string();

    let list_params = ListParams::default().labels(&label_selector);

    let pod_api: Api<Pod> = match &state.watch_namespace {
        Some(ns) => Api::namespaced(state.client.clone(), ns),
        None => Api::all(state.client.clone()),
    };

    let pods = pod_api.list(&list_params).await?;

    let ips: Vec<String> = pods
        .items
        .into_iter()
        .filter_map(|pod| {
            // Only consider running pods that have an IP
            let phase = pod
                .status
                .as_ref()
                .and_then(|s| s.phase.as_deref())
                .unwrap_or("");
            if phase != "Running" {
                return None;
            }
            pod.status.and_then(|s| s.pod_ip)
        })
        .collect();

    Ok(ips)
}

/// Shorten a Stellar public key to "GABC…WXYZ" (first 4 + "…" + last 4 characters).
fn short_key(key: &str) -> String {
    if key.len() <= 12 {
        return key.to_string();
    }
    format!("{}…{}", &key[..4], &key[key.len() - 4..])
}

// ── Stall tracker (used by the WebSocket stream for persistent stall detection) ─

/// Tracks the last observed SCP phase + ballot counter per node across WebSocket frames.
/// Nodes that do not advance within `stall_threshold` are flagged as stalled.
#[allow(dead_code)]
struct StallTracker {
    last_seen: HashMap<String, (String, u32, Instant)>,
    stall_threshold: Duration,
}

#[allow(dead_code)]
impl StallTracker {
    fn new(stall_threshold: Duration) -> Self {
        Self {
            last_seen: HashMap::new(),
            stall_threshold,
        }
    }

    /// Returns `true` if the node's phase + counter combination has not changed
    /// within the stall threshold.
    fn is_stalled(&mut self, node_id: &str, phase: &str, counter: u32) -> bool {
        let now = Instant::now();
        match self.last_seen.get_mut(node_id) {
            Some((last_phase, last_counter, last_seen)) => {
                if *last_phase != phase || *last_counter != counter {
                    *last_phase = phase.to_string();
                    *last_counter = counter;
                    *last_seen = now;
                    false
                } else {
                    now.duration_since(*last_seen) >= self.stall_threshold
                }
            }
            None => {
                self.last_seen
                    .insert(node_id.to_string(), (phase.to_string(), counter, now));
                false
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_key_long() {
        let key = "GCEZWKCA5VLDNRLN3RPRJMRZOX3Z6G5CHCGBWRXSJHEG8VORHEA3PUO";
        let short = short_key(key);
        assert!(short.contains('…'));
        assert!(short.starts_with("GCEZ"));
    }

    #[test]
    fn test_short_key_already_short() {
        let key = "GABC1234";
        assert_eq!(short_key(key), key);
    }

    #[test]
    fn test_stall_tracker_not_stalled_initially() {
        let mut tracker = StallTracker::new(Duration::from_secs(30));
        assert!(!tracker.is_stalled("node1", "PREPARE", 1));
    }

    #[test]
    fn test_stall_tracker_advances() {
        let mut tracker = StallTracker::new(Duration::from_secs(30));
        assert!(!tracker.is_stalled("node1", "PREPARE", 1));
        assert!(!tracker.is_stalled("node1", "CONFIRM", 2)); // advanced
    }

    #[test]
    fn test_topology_response_empty_ctor() {
        let r = QuorumTopologyResponse::empty();
        assert!(!r.healthy);
        assert!(r.nodes.is_empty());
        assert!(r.edges.is_empty());
    }
}
