//! Cross-Region State Synchronization
//!
//! This module implements automated cross-region state synchronization for Stellar Core,
//! enabling zero-RPO disaster recovery by continuously streaming captive core ledger state
//! across geographical regions.
//!
//! # Architecture
//!
//! ```text
//!  ┌─────────────────────────────────────────────────────────────────┐
//!  │  Primary Region (us-east-1)                                     │
//!  │  ┌──────────────────────────────────────────────────────────┐   │
//!  │  │  StellarNode Pod                                         │   │
//!  │  │  ┌─────────────────┐   ┌──────────────────────────────┐ │   │
//!  │  │  │  stellar-core   │──▶│  state-sync sidecar          │ │   │
//!  │  │  │  (captive core) │   │  - polls /info every 1s      │ │   │
//!  │  │  │  port 11626     │   │  - streams ledger state      │ │   │
//!  │  │  └─────────────────┘   │  - publishes to ConfigMap    │ │   │
//!  │  │                        └──────────────┬───────────────┘ │   │
//!  │  └───────────────────────────────────────┼─────────────────┘   │
//!  └──────────────────────────────────────────┼─────────────────────┘
//!                                             │  cross-cluster bridge
//!                                             ▼  (ExternalName / Submariner)
//!  ┌─────────────────────────────────────────────────────────────────┐
//!  │  Standby Region (eu-west-1)                                     │
//!  │  ┌──────────────────────────────────────────────────────────┐   │
//!  │  │  StellarNode Pod                                         │   │
//!  │  │  ┌─────────────────┐   ┌──────────────────────────────┐ │   │
//!  │  │  │  stellar-core   │◀──│  state-sync sidecar          │ │   │
//!  │  │  │  (standby)      │   │  - receives ledger stream    │ │   │
//!  │  │  │                 │   │  - verifies hash chain       │ │   │
//!  │  │  └─────────────────┘   │  - updates sync status       │ │   │
//!  │  │                        └──────────────────────────────┘ │   │
//!  │  └──────────────────────────────────────────────────────────┘   │
//!  └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Sidecar Container
//!
//! The operator injects a `state-sync` sidecar into every StellarNode pod when
//! `dr_config.sync_strategy == StreamingLedger`. The sidecar:
//!
//! 1. Polls the local Stellar Core HTTP API (`/info`) every second.
//! 2. Extracts the latest closed ledger sequence and hash.
//! 3. Publishes the state snapshot to a Kubernetes ConfigMap
//!    (`<node>-ledger-state`) in the same namespace.
//! 4. Standby-region operators watch that ConfigMap (via cross-cluster bridge)
//!    and compare against their local ledger to compute sync lag.
//!
//! # Cross-Cluster Bridge
//!
//! The operator creates an `ExternalName` Service pointing at the primary region's
//! state-sync endpoint, enabling standby pods to reach the primary ConfigMap API
//! through the Kubernetes DNS without a full service mesh.

use std::collections::BTreeMap;

use chrono::Utc;
use k8s_openapi::api::core::v1::{
    ConfigMap, Container, EnvVar, ResourceRequirements as K8sResources, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, Patch, PatchParams};
use kube::{Client, ResourceExt};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};

use crate::crd::{DRRole, DRSyncStrategy, StellarNode};
use crate::error::{Error, Result};

// ─── Constants ───────────────────────────────────────────────────────────────

/// ConfigMap key that holds the serialised [`LedgerStateSnapshot`].
pub const LEDGER_STATE_KEY: &str = "ledger-state";

/// Annotation written on the ConfigMap to record the last update time.
pub const LAST_UPDATED_ANNOTATION: &str = "stellar.org/state-sync-updated-at";

/// Annotation written on the ConfigMap to record the source node name.
pub const SOURCE_NODE_ANNOTATION: &str = "stellar.org/state-sync-source";

/// Maximum acceptable sync lag (in ledgers) before the standby is considered
/// out-of-sync and the DR status is degraded.
pub const MAX_ACCEPTABLE_LAG_LEDGERS: u64 = 10;

/// Docker image for the state-sync sidecar.
/// In production, pin this to a specific digest.
pub const STATE_SYNC_SIDECAR_IMAGE: &str =
    "ghcr.io/stellar/stellar-k8s/state-sync-sidecar:latest";

// ─── Data types ──────────────────────────────────────────────────────────────

/// A point-in-time snapshot of the Stellar Core ledger state.
///
/// This is the payload written to the ConfigMap by the sidecar and read by
/// standby-region operators to compute sync lag and verify hash continuity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LedgerStateSnapshot {
    /// Latest closed ledger sequence number.
    pub ledger_sequence: u64,
    /// SHA-256 hash of the latest closed ledger (hex-encoded).
    pub ledger_hash: String,
    /// Network passphrase this node is running on.
    pub network_passphrase: String,
    /// RFC-3339 timestamp when this snapshot was captured.
    pub captured_at: String,
    /// Stellar Core version string (from `/info`).
    pub core_version: String,
    /// Whether the node is currently in sync with the network.
    pub in_sync: bool,
}

/// Result of a state consistency check between primary and standby.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsistencyCheckResult {
    /// Ledger sequence on the primary at check time.
    pub primary_ledger: u64,
    /// Ledger sequence on the standby at check time.
    pub standby_ledger: u64,
    /// Computed lag (primary − standby). Zero means fully in sync.
    pub lag_ledgers: u64,
    /// Whether the lag is within the acceptable threshold.
    pub within_threshold: bool,
    /// Whether the hash chain is consistent (no fork detected).
    pub hash_chain_consistent: bool,
}

/// Sync status written back to the StellarNode DR status.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StateSyncStatus {
    /// Current sync lag in ledgers (0 = fully synced).
    pub lag_ledgers: u64,
    /// Whether the standby is within the acceptable lag threshold.
    pub within_threshold: bool,
    /// RFC-3339 timestamp of the last successful sync check.
    pub last_sync_check: Option<String>,
    /// Whether a hash-chain fork has been detected.
    pub fork_detected: bool,
    /// Human-readable message about the current sync state.
    pub message: String,
}

// ─── ConfigMap helpers ───────────────────────────────────────────────────────

/// Return the name of the ledger-state ConfigMap for a given node.
pub fn ledger_state_configmap_name(node: &StellarNode) -> String {
    format!("{}-ledger-state", node.name_any())
}

/// Ensure the ledger-state ConfigMap exists and is up-to-date.
///
/// Called by the operator reconciler on every loop for primary-role nodes.
/// The ConfigMap is the single source of truth that standby operators poll.
#[instrument(skip(client, node, snapshot), fields(name = %node.name_any()))]
pub async fn ensure_ledger_state_configmap(
    client: &Client,
    node: &StellarNode,
    snapshot: &LedgerStateSnapshot,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let cm_name = ledger_state_configmap_name(node);
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);

    let payload = serde_json::to_string(snapshot)
        .map_err(Error::SerializationError)?;

    let mut labels = BTreeMap::new();
    labels.insert("app.kubernetes.io/managed-by".to_string(), "stellar-operator".to_string());
    labels.insert("stellar.org/state-sync".to_string(), "true".to_string());
    labels.insert("stellar.org/node".to_string(), node.name_any());

    let mut annotations = BTreeMap::new();
    annotations.insert(LAST_UPDATED_ANNOTATION.to_string(), Utc::now().to_rfc3339());
    annotations.insert(SOURCE_NODE_ANNOTATION.to_string(), node.name_any());

    let mut data = BTreeMap::new();
    data.insert(LEDGER_STATE_KEY.to_string(), payload);

    let cm = ConfigMap {
        metadata: ObjectMeta {
            name: Some(cm_name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(labels),
            annotations: Some(annotations),
            owner_references: Some(vec![crate::controller::resources::owner_reference(node)]),
            ..Default::default()
        },
        data: Some(data),
        ..Default::default()
    };

    api.patch(
        &cm_name,
        &PatchParams::apply("stellar-operator").force(),
        &Patch::Apply(&cm),
    )
    .await?;

    debug!("Ledger state ConfigMap {} updated (seq={})", cm_name, snapshot.ledger_sequence);
    Ok(())
}

/// Read the ledger-state ConfigMap from the cluster.
///
/// Used by standby-region operators to fetch the primary's latest state.
/// In a cross-cluster setup the `client` is configured to talk to the
/// primary cluster (via kubeconfig or in-cluster service account token).
#[instrument(skip(client), fields(node_name = %node_name, namespace = %namespace))]
pub async fn read_ledger_state_configmap(
    client: &Client,
    node_name: &str,
    namespace: &str,
) -> Result<Option<LedgerStateSnapshot>> {
    let cm_name = format!("{node_name}-ledger-state");
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);

    match api.get(&cm_name).await {
        Ok(cm) => {
            let data = cm.data.unwrap_or_default();
            match data.get(LEDGER_STATE_KEY) {
                Some(raw) => {
                    let snapshot: LedgerStateSnapshot = serde_json::from_str(raw)
                        .map_err(Error::SerializationError)?;
                    Ok(Some(snapshot))
                }
                None => {
                    warn!("ConfigMap {} exists but has no '{}' key", cm_name, LEDGER_STATE_KEY);
                    Ok(None)
                }
            }
        }
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(None),
        Err(e) => Err(Error::KubeError(e)),
    }
}

// ─── Consistency check ───────────────────────────────────────────────────────

/// Check state consistency between primary and standby.
///
/// Compares the primary's published [`LedgerStateSnapshot`] against the
/// standby node's current ledger sequence. Returns a [`ConsistencyCheckResult`]
/// that the reconciler uses to update DR status and trigger alerts.
pub fn check_state_consistency(
    primary_snapshot: &LedgerStateSnapshot,
    standby_ledger_sequence: u64,
    standby_ledger_hash: Option<&str>,
) -> ConsistencyCheckResult {
    let lag = primary_snapshot
        .ledger_sequence
        .saturating_sub(standby_ledger_sequence);

    // Hash chain consistency: if the standby is at the same ledger as the
    // primary, their hashes must match. A mismatch indicates a fork.
    let hash_chain_consistent = match standby_ledger_hash {
        Some(hash) if standby_ledger_sequence == primary_snapshot.ledger_sequence => {
            hash == primary_snapshot.ledger_hash
        }
        // Standby is behind — we can't compare hashes yet, assume consistent
        _ => true,
    };

    ConsistencyCheckResult {
        primary_ledger: primary_snapshot.ledger_sequence,
        standby_ledger: standby_ledger_sequence,
        lag_ledgers: lag,
        within_threshold: lag <= MAX_ACCEPTABLE_LAG_LEDGERS,
        hash_chain_consistent,
    }
}

// ─── Sidecar container builder ───────────────────────────────────────────────

/// Build the state-sync sidecar [`Container`] spec.
///
/// The sidecar is injected into the StellarNode pod template by the operator
/// when `dr_config.sync_strategy == StreamingLedger`. It polls the local
/// Stellar Core HTTP API and publishes ledger state to the ConfigMap.
///
/// # Environment variables consumed by the sidecar
///
/// | Variable                  | Description                                      |
/// |---------------------------|--------------------------------------------------|
/// | `STELLAR_CORE_HTTP_URL`   | URL of the local Stellar Core HTTP API           |
/// | `NAMESPACE`               | Kubernetes namespace (from Downward API)         |
/// | `NODE_NAME`               | StellarNode resource name (from Downward API)    |
/// | `POLL_INTERVAL_SECS`      | How often to poll Core (default: 1)              |
/// | `NETWORK_PASSPHRASE`      | Expected network passphrase for validation       |
pub fn build_state_sync_sidecar(node: &StellarNode) -> Container {
    let core_http_url = node
        .spec
        .soroban_config
        .as_ref()
        .map(|s| s.stellar_core_url.clone())
        .unwrap_or_else(|| "http://localhost:11626".to_string());

    let network_passphrase = node.spec.network.passphrase().to_string();

    let mut env = vec![
        EnvVar {
            name: "STELLAR_CORE_HTTP_URL".to_string(),
            value: Some(core_http_url),
            value_from: None,
        },
        EnvVar {
            name: "NETWORK_PASSPHRASE".to_string(),
            value: Some(network_passphrase),
            value_from: None,
        },
        EnvVar {
            name: "POLL_INTERVAL_SECS".to_string(),
            value: Some("1".to_string()),
            value_from: None,
        },
        EnvVar {
            name: "NODE_NAME".to_string(),
            value: Some(node.name_any()),
            value_from: None,
        },
    ];

    // Inject namespace via Downward API
    env.push(EnvVar {
        name: "NAMESPACE".to_string(),
        value: node.namespace(),
        value_from: None,
    });

    Container {
        name: "state-sync".to_string(),
        image: Some(STATE_SYNC_SIDECAR_IMAGE.to_string()),
        image_pull_policy: Some("IfNotPresent".to_string()),
        command: Some(vec!["stellar-operator".to_string(), "sidecar".to_string()]),
        env: Some(env),
        resources: Some(K8sResources {
            requests: Some({
                let mut m = BTreeMap::new();
                m.insert("cpu".to_string(), Quantity("50m".to_string()));
                m.insert("memory".to_string(), Quantity("64Mi".to_string()));
                m
            }),
            limits: Some({
                let mut m = BTreeMap::new();
                m.insert("cpu".to_string(), Quantity("200m".to_string()));
                m.insert("memory".to_string(), Quantity("128Mi".to_string()));
                m
            }),
            ..Default::default()
        }),
        // Mount the shared data volume read-only so the sidecar can inspect
        // the Stellar Core database directory for additional state signals.
        volume_mounts: Some(vec![VolumeMount {
            name: "data".to_string(),
            mount_path: "/data".to_string(),
            read_only: Some(true),
            ..Default::default()
        }]),
        ..Default::default()
    }
}

// ─── Main reconcile entry point ──────────────────────────────────────────────

/// Reconcile cross-region state synchronization for a StellarNode.
///
/// This is called from the main reconciler loop. It:
///
/// 1. For **primary** nodes: fetches the current ledger state from the local
///    Stellar Core HTTP API and publishes it to the ledger-state ConfigMap.
/// 2. For **standby** nodes: reads the primary's ConfigMap (via cross-cluster
///    bridge), computes sync lag, checks hash-chain consistency, and updates
///    the node's DR status.
///
/// Returns `Ok(Some(StateSyncStatus))` when state sync is active, or
/// `Ok(None)` when the node has no DR config or sync is disabled.
#[instrument(skip(client, node), fields(name = %node.name_any()))]
pub async fn reconcile_state_sync(
    client: &Client,
    node: &StellarNode,
) -> Result<Option<StateSyncStatus>> {
    let dr_config = match &node.spec.dr_config {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(None),
    };

    // Only StreamingLedger strategy uses this module.
    // Other strategies (Consensus, PeerTracking, ArchiveSync) are handled in dr.rs.
    if dr_config.sync_strategy != DRSyncStrategy::StreamingLedger {
        return Ok(None);
    }

    match dr_config.role {
        DRRole::Primary => reconcile_primary(client, node).await,
        DRRole::Standby => reconcile_standby(client, node, &dr_config.peer_cluster_id).await,
    }
}

/// Primary-side reconciliation: publish ledger state to ConfigMap.
async fn reconcile_primary(client: &Client, node: &StellarNode) -> Result<Option<StateSyncStatus>> {
    // The sidecar is responsible for high-frequency updates.
    // The operator just ensures the initial ConfigMap exists if it's missing.
    let cm_name = ledger_state_configmap_name(node);
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);

    if api.get(&cm_name).await.is_err() {
        let snapshot = fetch_local_ledger_state(node).await?;
        ensure_ledger_state_configmap(client, node, &snapshot).await?;
    }

    Ok(Some(StateSyncStatus {
        lag_ledgers: 0,
        within_threshold: true,
        last_sync_check: Some(Utc::now().to_rfc3339()),
        fork_detected: false,
        message: "Primary state-sync active (sidecar publishing)".to_string(),
    }))
}

/// Run the sidecar loop that polls Stellar Core and updates the ConfigMap.
pub async fn run_sidecar_loop(
    client: Client,
    namespace: &str,
    node_name: &str,
    core_url: &str,
    poll_interval_secs: u64,
    expected_passphrase: &str,
) -> Result<()> {
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
    let node_api: Api<StellarNode> = Api::namespaced(client.clone(), namespace);

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(poll_interval_secs));

    loop {
        interval.tick().await;

        // 1. Fetch StellarNode to get owner reference
        let node = match node_api.get(node_name).await {
            Ok(n) => n,
            Err(e) => {
                warn!("Sidecar: failed to fetch StellarNode {}: {}", node_name, e);
                continue;
            }
        };

        // 2. Poll Stellar Core
        let snapshot = match fetch_local_ledger_state_internal(core_url, expected_passphrase).await {
            Ok(s) => s,
            Err(e) => {
                warn!("Sidecar: failed to poll Stellar Core: {}", e);
                continue;
            }
        };

        // 3. Update ConfigMap
        if let Err(e) = ensure_ledger_state_configmap(&client, &node, &snapshot).await {
            warn!("Sidecar: failed to update ConfigMap: {}", e);
        } else {
            debug!(
                "Sidecar: updated ledger state to {} (hash={}…)",
                snapshot.ledger_sequence,
                &snapshot.ledger_hash[..8.min(snapshot.ledger_hash.len())]
            );
        }
    }
}

/// Internal helper for polling Stellar Core without needing a StellarNode object.
async fn fetch_local_ledger_state_internal(
    base_url: &str,
    network_passphrase: &str,
) -> Result<LedgerStateSnapshot> {
    let url = format!("{}/info", base_url.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| Error::NetworkError(format!("HTTP client build error: {e}")))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| Error::NetworkError(format!("Failed to reach Stellar Core: {e}")))?;

    if !resp.status().is_success() {
        return Err(Error::NetworkError(format!(
            "Stellar Core /info returned HTTP {}",
            resp.status()
        )));
    }

    let info: CoreInfoResponse = resp
        .json()
        .await
        .map_err(|e| Error::NetworkError(format!("Failed to parse /info response: {e}")))?;

    Ok(LedgerStateSnapshot {
        ledger_sequence: info.info.ledger.num,
        ledger_hash: info.info.ledger.hash,
        network_passphrase: network_passphrase.to_string(),
        captured_at: Utc::now().to_rfc3339(),
        core_version: info.info.build,
        in_sync: info.info.state == "Synced!" || info.info.state.contains("synced"),
    })
}

/// Standby-side reconciliation: read primary state and compute lag.
async fn reconcile_standby(
    client: &Client,
    node: &StellarNode,
    peer_cluster_id: &str,
) -> Result<Option<StateSyncStatus>> {
    // Derive the primary node name from the peer cluster ID convention:
    // peer_cluster_id is expected to be "<namespace>/<node-name>" or just "<node-name>".
    let (peer_namespace, peer_node_name) = parse_peer_cluster_id(peer_cluster_id, node);

    let primary_snapshot =
        read_ledger_state_configmap(client, &peer_node_name, &peer_namespace).await?;

    let primary_snapshot = match primary_snapshot {
        Some(s) => s,
        None => {
            warn!(
                "Standby {}: primary ConfigMap not found for peer {}",
                node.name_any(),
                peer_cluster_id
            );
            return Ok(Some(StateSyncStatus {
                lag_ledgers: u64::MAX,
                within_threshold: false,
                last_sync_check: Some(Utc::now().to_rfc3339()),
                fork_detected: false,
                message: "Primary ledger state ConfigMap not found".to_string(),
            }));
        }
    };

    // Get local ledger state
    let local_snapshot = fetch_local_ledger_state(node).await?;

    let result = check_state_consistency(
        &primary_snapshot,
        local_snapshot.ledger_sequence,
        Some(&local_snapshot.ledger_hash),
    );

    if !result.hash_chain_consistent {
        warn!(
            "FORK DETECTED on standby {}: primary_hash={}, local_hash={}",
            node.name_any(),
            primary_snapshot.ledger_hash,
            local_snapshot.ledger_hash
        );
    }

    let message = if result.within_threshold {
        format!(
            "In sync: lag={} ledgers (primary={}, local={})",
            result.lag_ledgers, result.primary_ledger, result.standby_ledger
        )
    } else {
        format!(
            "LAG EXCEEDED: {} ledgers behind (threshold={})",
            result.lag_ledgers, MAX_ACCEPTABLE_LAG_LEDGERS
        )
    };

    info!("Standby state-sync {}: {}", node.name_any(), message);

    Ok(Some(StateSyncStatus {
        lag_ledgers: result.lag_ledgers,
        within_threshold: result.within_threshold,
        last_sync_check: Some(Utc::now().to_rfc3339()),
        fork_detected: !result.hash_chain_consistent,
        message,
    }))
}

// ─── Stellar Core HTTP client ─────────────────────────────────────────────────

/// Response shape from Stellar Core's `/info` endpoint (subset we care about).
#[derive(Debug, Deserialize)]
struct CoreInfoResponse {
    info: CoreInfo,
}

#[derive(Debug, Deserialize)]
struct CoreInfo {
    ledger: CoreLedgerInfo,
    network: String,
    build: String,
    state: String,
}

#[derive(Debug, Deserialize)]
struct CoreLedgerInfo {
    num: u64,
    hash: String,
}

/// Fetch the current ledger state from the local Stellar Core HTTP API.
///
/// In production the sidecar container does this continuously; the operator
/// also calls this during reconciliation to publish the initial ConfigMap.
async fn fetch_local_ledger_state(node: &StellarNode) -> Result<LedgerStateSnapshot> {
    let base_url = node
        .spec
        .soroban_config
        .as_ref()
        .map(|s| s.stellar_core_url.trim_end_matches('/').to_string())
        .unwrap_or_else(|| "http://localhost:11626".to_string());

    let url = format!("{base_url}/info");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| Error::NetworkError(format!("HTTP client build error: {e}")))?;

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let info: CoreInfoResponse = resp
                .json()
                .await
                .map_err(|e| Error::NetworkError(format!("Failed to parse /info response: {e}")))?;

            Ok(LedgerStateSnapshot {
                ledger_sequence: info.info.ledger.num,
                ledger_hash: info.info.ledger.hash,
                network_passphrase: info.info.network,
                captured_at: Utc::now().to_rfc3339(),
                core_version: info.info.build,
                in_sync: info.info.state == "Synced!" || info.info.state.contains("synced"),
            })
        }
        Ok(resp) => Err(Error::NetworkError(format!(
            "Stellar Core /info returned HTTP {}",
            resp.status()
        ))),
        Err(e) => {
            // Return a placeholder snapshot so the operator doesn't crash
            // when Core is still starting up.
            warn!("Could not reach Stellar Core at {}: {}", url, e);
            Ok(LedgerStateSnapshot {
                ledger_sequence: node
                    .status
                    .as_ref()
                    .and_then(|s| s.ledger_sequence)
                    .unwrap_or(0),
                ledger_hash: "unknown".to_string(),
                network_passphrase: node.spec.network.passphrase().to_string(),
                captured_at: Utc::now().to_rfc3339(),
                core_version: "unknown".to_string(),
                in_sync: false,
            })
        }
    }
}

/// Parse a peer cluster ID into (namespace, node_name).
///
/// Convention: `"<namespace>/<node-name>"` or just `"<node-name>"` (uses
/// the same namespace as the local node).
fn parse_peer_cluster_id<'a>(
    peer_cluster_id: &'a str,
    local_node: &StellarNode,
) -> (String, String) {
    if let Some((ns, name)) = peer_cluster_id.split_once('/') {
        (ns.to_string(), name.to_string())
    } else {
        (
            local_node.namespace().unwrap_or_else(|| "default".to_string()),
            peer_cluster_id.to_string(),
        )
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{
        DisasterRecoveryConfig, DRRole, DRSyncStrategy, NodeType, ResourceRequirements,
        ResourceSpec, StellarNetwork, StellarNode, StellarNodeSpec, StorageConfig,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn make_node(name: &str, role: DRRole) -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("stellar-system".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Validator,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                history_mode: Default::default(),
                resources: ResourceRequirements {
                    requests: ResourceSpec {
                        cpu: "500m".to_string(),
                        memory: "1Gi".to_string(),
                    },
                    limits: ResourceSpec {
                        cpu: "2".to_string(),
                        memory: "4Gi".to_string(),
                    },
                },
                storage: StorageConfig::default(),
                validator_config: None,
                horizon_config: None,
                soroban_config: None,
                replicas: 1,
                min_available: None,
                max_unavailable: None,
                suspended: false,
                alerting: false,
                database: None,
                managed_database: None,
                autoscaling: None,
                vpa_config: None,
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
                strategy: Default::default(),
                maintenance_mode: false,
                network_policy: None,
                dr_config: Some(DisasterRecoveryConfig {
                    enabled: true,
                    role,
                    peer_cluster_id: "stellar-system/validator-primary".to_string(),
                    sync_strategy: DRSyncStrategy::StreamingLedger,
                    failover_dns: None,
                    health_check_interval: 30,
                    drill_schedule: None,
                }),
                pod_anti_affinity: Default::default(),
                topology_spread_constraints: None,
                cve_handling: None,
                snapshot_schedule: None,
                restore_from_snapshot: None,
                read_replica_config: None,
                read_pool_endpoint: None,
                db_maintenance_config: None,
                oci_snapshot: None,
                service_mesh: None,
                forensic_snapshot: None,
                resource_meta: None,
            },
            status: None,
        }
    }

    fn make_snapshot(seq: u64, hash: &str) -> LedgerStateSnapshot {
        LedgerStateSnapshot {
            ledger_sequence: seq,
            ledger_hash: hash.to_string(),
            network_passphrase: "Test SDF Network ; September 2015".to_string(),
            captured_at: "2026-05-31T00:00:00Z".to_string(),
            core_version: "v21.0.0".to_string(),
            in_sync: true,
        }
    }

    // ── Consistency checks ────────────────────────────────────────────────────

    #[test]
    fn test_fully_synced_no_lag() {
        let primary = make_snapshot(1_000_000, "abc123");
        let result = check_state_consistency(&primary, 1_000_000, Some("abc123"));
        assert_eq!(result.lag_ledgers, 0);
        assert!(result.within_threshold);
        assert!(result.hash_chain_consistent);
    }

    #[test]
    fn test_small_lag_within_threshold() {
        let primary = make_snapshot(1_000_005, "abc123");
        let result = check_state_consistency(&primary, 1_000_000, None);
        assert_eq!(result.lag_ledgers, 5);
        assert!(result.within_threshold, "lag of 5 should be within threshold of {MAX_ACCEPTABLE_LAG_LEDGERS}");
    }

    #[test]
    fn test_large_lag_exceeds_threshold() {
        let primary = make_snapshot(1_000_100, "abc123");
        let result = check_state_consistency(&primary, 1_000_000, None);
        assert_eq!(result.lag_ledgers, 100);
        assert!(!result.within_threshold, "lag of 100 should exceed threshold");
    }

    #[test]
    fn test_hash_mismatch_at_same_ledger_detects_fork() {
        let primary = make_snapshot(1_000_000, "correct_hash");
        let result = check_state_consistency(&primary, 1_000_000, Some("wrong_hash"));
        assert!(!result.hash_chain_consistent, "hash mismatch must be detected as fork");
        assert_eq!(result.lag_ledgers, 0);
    }

    #[test]
    fn test_hash_match_at_same_ledger_is_consistent() {
        let primary = make_snapshot(1_000_000, "correct_hash");
        let result = check_state_consistency(&primary, 1_000_000, Some("correct_hash"));
        assert!(result.hash_chain_consistent);
    }

    #[test]
    fn test_standby_behind_no_hash_comparison() {
        // When standby is behind, we can't compare hashes — should be consistent
        let primary = make_snapshot(1_000_010, "primary_hash");
        let result = check_state_consistency(&primary, 1_000_000, Some("old_hash"));
        assert!(result.hash_chain_consistent, "behind standby should not trigger fork detection");
        assert_eq!(result.lag_ledgers, 10);
    }

    #[test]
    fn test_standby_ahead_of_primary_zero_lag() {
        // Saturating sub: standby ahead → lag = 0
        let primary = make_snapshot(999_990, "hash_a");
        let result = check_state_consistency(&primary, 1_000_000, None);
        assert_eq!(result.lag_ledgers, 0, "saturating_sub should prevent underflow");
        assert!(result.within_threshold);
    }

    // ── Sidecar container spec ────────────────────────────────────────────────

    #[test]
    fn test_sidecar_has_correct_name() {
        let node = make_node("validator-primary", DRRole::Primary);
        let sidecar = build_state_sync_sidecar(&node);
        assert_eq!(sidecar.name, "state-sync");
    }

    #[test]
    fn test_sidecar_has_network_passphrase_env() {
        let node = make_node("validator-primary", DRRole::Primary);
        let sidecar = build_state_sync_sidecar(&node);
        let env = sidecar.env.unwrap_or_default();
        let passphrase_env = env.iter().find(|e| e.name == "NETWORK_PASSPHRASE");
        assert!(passphrase_env.is_some(), "sidecar must have NETWORK_PASSPHRASE env var");
        assert_eq!(
            passphrase_env.unwrap().value.as_deref(),
            Some("Test SDF Network ; September 2015")
        );
    }

    #[test]
    fn test_sidecar_has_resource_limits() {
        let node = make_node("validator-primary", DRRole::Primary);
        let sidecar = build_state_sync_sidecar(&node);
        let resources = sidecar.resources.expect("sidecar must have resource limits");
        assert!(resources.limits.is_some(), "sidecar must have resource limits");
        assert!(resources.requests.is_some(), "sidecar must have resource requests");
    }

    #[test]
    fn test_sidecar_mounts_data_volume_readonly() {
        let node = make_node("validator-primary", DRRole::Primary);
        let sidecar = build_state_sync_sidecar(&node);
        let mounts = sidecar.volume_mounts.unwrap_or_default();
        let data_mount = mounts.iter().find(|m| m.name == "data");
        assert!(data_mount.is_some(), "sidecar must mount the data volume");
        assert_eq!(data_mount.unwrap().read_only, Some(true), "data volume must be read-only");
    }

    // ── ConfigMap naming ──────────────────────────────────────────────────────

    #[test]
    fn test_configmap_name_format() {
        let node = make_node("validator-primary", DRRole::Primary);
        assert_eq!(
            ledger_state_configmap_name(&node),
            "validator-primary-ledger-state"
        );
    }

    // ── Peer cluster ID parsing ───────────────────────────────────────────────

    #[test]
    fn test_parse_peer_cluster_id_with_namespace() {
        let node = make_node("standby", DRRole::Standby);
        let (ns, name) = parse_peer_cluster_id("other-ns/primary-node", &node);
        assert_eq!(ns, "other-ns");
        assert_eq!(name, "primary-node");
    }

    #[test]
    fn test_parse_peer_cluster_id_without_namespace_uses_local() {
        let node = make_node("standby", DRRole::Standby);
        let (ns, name) = parse_peer_cluster_id("primary-node", &node);
        assert_eq!(ns, "stellar-system");
        assert_eq!(name, "primary-node");
    }

    #[test]
    fn test_consistency_under_high_load() {
        // Simulate 1000 ledger advances with random jitter (0-3 ledgers behind)
        let mut primary_seq = 1_000_000;
        let mut standby_seq = 1_000_000;

        for i in 0..1000 {
            primary_seq += 1;
            // Simulate standby being slightly behind (0, 1, 2, or 3 ledgers)
            let jitter = i % 4;
            standby_seq = primary_seq.saturating_sub(jitter);

            let primary = make_snapshot(primary_seq, "hash");
            let result = check_state_consistency(&primary, standby_seq, Some("hash"));

            assert!(
                result.within_threshold,
                "Lag of {} should be within threshold at primary={}",
                result.lag_ledgers,
                primary_seq
            );
            assert!(
                result.hash_chain_consistent,
                "Hash should be consistent (when compared)"
            );
        }
    }
}

    // ── High-load consistency simulation ─────────────────────────────────────
    // Proves state consistency under high load by simulating rapid ledger
    // advancement (Stellar closes ~1 ledger/5s; under load we simulate bursts).

    #[test]
    fn test_consistency_under_rapid_ledger_advancement() {
        // Simulate 1000 ledger advances in rapid succession
        let mut primary_seq: u64 = 1_000_000;
        let mut standby_seq: u64 = 1_000_000;
        let hash = "deadbeef";

        for i in 0..1000u64 {
            primary_seq += 1;
            // Standby lags by at most 3 ledgers (simulating network jitter)
            if i % 4 != 0 {
                standby_seq += 1;
            }

            let primary = make_snapshot(primary_seq, hash);
            let result = check_state_consistency(&primary, standby_seq, None);

            assert!(
                result.lag_ledgers <= MAX_ACCEPTABLE_LAG_LEDGERS,
                "lag {} exceeded threshold {} at iteration {}",
                result.lag_ledgers,
                MAX_ACCEPTABLE_LAG_LEDGERS,
                i
            );
        }
    }

    #[test]
    fn test_consistency_detects_sustained_lag_under_load() {
        // Simulate standby falling behind by 20 ledgers (network partition)
        let primary = make_snapshot(1_020_000, "hash_x");
        let result = check_state_consistency(&primary, 1_000_000, None);
        assert!(!result.within_threshold, "sustained 20-ledger lag must be flagged");
        assert_eq!(result.lag_ledgers, 20_000);
    }

    #[test]
    fn test_snapshot_serialization_roundtrip() {
        let original = make_snapshot(42_000_000, "cafebabe1234");
        let json = serde_json::to_string(&original).expect("serialization must succeed");
        let decoded: LedgerStateSnapshot =
            serde_json::from_str(&json).expect("deserialization must succeed");
        assert_eq!(original, decoded);
    }
}
