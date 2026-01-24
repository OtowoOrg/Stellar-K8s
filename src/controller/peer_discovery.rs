//! Dynamic peer discovery for Stellar validators
//!
//! This module watches StellarNode resources and automatically updates
//! a shared ConfigMap with discovered peer addresses (IP:port).
//!
//! Features:
//! - Real-time discovery of validator peers across the cluster
//! - Automatic ConfigMap updates with peer IP/Port combinations
//! - Rolling pod restart to apply peer configuration changes
//! - Excludes self and non-validator nodes from peer list

use std::collections::{BTreeMap, BTreeSet};

use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    client::Client,
    ResourceExt,
};
use tracing::{debug, error, info, instrument, warn};

use crate::crd::{NodeType, StellarNode};
use crate::error::{Error, Result};

/// ConfigMap name for shared peer discovery data (in same namespace)
const PEERS_CONFIGMAP_SUFFIX: &str = "peers";

/// Key in ConfigMap containing the peers list
const PEERS_CONFIG_KEY: &str = "KNOWN_PEERS";

/// Key in ConfigMap containing peer discovery metadata
const PEERS_METADATA_KEY: &str = "discovery_metadata";

/// Peer discovery result
#[derive(Debug, Clone)]
pub struct PeerDiscoveryResult {
    /// List of discovered peer addresses (IP:port)
    pub peers: Vec<String>,
    /// Number of active validators
    pub active_validator_count: usize,
    /// Whether the peer list changed from previous discovery
    pub changed: bool,
}

impl PeerDiscoveryResult {
    /// Format peers for Stellar configuration
    pub fn to_stellar_config(&self) -> String {
        self.peers.join("\n")
    }
}

/// Discover all active validator peers in the cluster (namespace)
///
/// Returns list of validator IP:port combinations, excluding the current node.
/// Only considers validators with running, ready pods.
#[instrument(skip(client), fields(namespace = %namespace))]
pub async fn discover_peers(
    client: &Client,
    namespace: &str,
    exclude_node: Option<&str>,
) -> Result<PeerDiscoveryResult> {
    let api: Api<StellarNode> = Api::namespaced(client.clone(), namespace);
    
    // List all StellarNode resources in namespace
    let nodes = api
        .list(&ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    let mut peers = BTreeSet::new();
    let mut active_validator_count = 0;

    for node in nodes.items.iter() {
        let node_name = node.name_any();

        // Skip non-validators
        if node.spec.node_type != NodeType::Validator {
            debug!("Skipping non-validator node: {}", node_name);
            continue;
        }

        // Skip excluded node (usually self)
        if let Some(exclude) = exclude_node {
            if node_name == exclude {
                debug!("Skipping excluded node: {}", node_name);
                continue;
            }
        }

        // Skip suspended nodes
        if node.spec.suspended {
            debug!("Skipping suspended validator: {}", node_name);
            continue;
        }

        active_validator_count += 1;

        // Try to get peer address from the running pod
        if let Some(peer_addr) = get_peer_address(client, node, namespace).await? {
            peers.insert(peer_addr);
            debug!("Discovered peer for {}: {}", node_name, peers.iter().last().unwrap());
        } else {
            debug!("No ready pod found for validator: {}", node_name);
        }
    }

    Ok(PeerDiscoveryResult {
        peers: peers.into_iter().collect(),
        active_validator_count,
        changed: false, // Will be set when comparing with existing config
    })
}

/// Get the peer address (IP:port) for a validator node
///
/// Queries the service to get the stable IP and uses the configured peer port
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = %namespace))]
async fn get_peer_address(
    client: &Client,
    node: &StellarNode,
    namespace: &str,
) -> Result<Option<String>> {
    // First check if the StatefulSet has running replicas
    let ss_api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
    let node_name = node.name_any();

    match ss_api.get(&node_name).await {
        Ok(ss) => {
            let replicas = ss.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
            if replicas == 0 {
                debug!("StatefulSet {} has 0 replicas", node_name);
                return Ok(None);
            }
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            debug!("StatefulSet {} not found", node_name);
            return Ok(None);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    // Try to get pod IP from running pod
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let label_selector = format!(
        "app.kubernetes.io/instance={},app.kubernetes.io/name=stellar-node",
        node_name
    );

    match pod_api
        .list(&ListParams::default().labels(&label_selector))
        .await
    {
        Ok(pods) => {
            // Find the first ready pod
            for pod in pods.items.iter() {
                if let Some(pod_ip) = pod.status.as_ref().and_then(|s| s.pod_ip.as_ref()) {
                    // Check if pod is ready
                    let is_ready = pod.status.as_ref()
                        .and_then(|s| s.conditions.as_ref())
                        .map(|conds| {
                            conds.iter().any(|c| {
                                c.type_ == "Ready" && c.status == "True"
                            })
                        })
                        .unwrap_or(false);

                    if is_ready {
                        // Default Stellar Core peer port is 11625
                        let peer_port = node.spec.validator_config
                            .as_ref()
                            .and_then(|vc| vc.peer_port)
                            .unwrap_or(11625);

                        let peer_addr = format!("{}:{}", pod_ip, peer_port);
                        return Ok(Some(peer_addr));
                    }
                }
            }
            debug!("No ready pod found for {}", node_name);
            Ok(None)
        }
        Err(e) => {
            warn!("Failed to list pods for {}: {:?}", node_name, e);
            Ok(None) // Continue with other nodes even if one fails
        }
    }
}

/// Ensure peers ConfigMap exists and is up-to-date
///
/// Creates or updates the shared peers ConfigMap with the latest discovered peers.
/// Returns true if the ConfigMap was updated (peers changed).
#[instrument(skip(client, discovery_result), fields(namespace = %namespace))]
pub async fn ensure_peers_config_map(
    client: &Client,
    namespace: &str,
    discovery_result: &PeerDiscoveryResult,
) -> Result<bool> {
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
    let config_name = format!("stellar-{}", PEERS_CONFIGMAP_SUFFIX);

    // Try to get existing ConfigMap
    let existing = api.get(&config_name).await.ok();
    let existing_peers = existing.as_ref().and_then(|cm| {
        cm.data.as_ref().and_then(|data| {
            data.get(PEERS_CONFIG_KEY).map(|s| s.to_string())
        })
    });

    let new_peers_config = discovery_result.to_stellar_config();
    
    // Check if peers actually changed
    let peers_changed = existing_peers.as_ref() != Some(&new_peers_config);

    // Only update if changed
    if !peers_changed && existing.is_some() {
        debug!("Peers ConfigMap is up-to-date, skipping update");
        return Ok(false);
    }

    // Build new ConfigMap
    let mut data = BTreeMap::new();
    data.insert(PEERS_CONFIG_KEY.to_string(), new_peers_config);
    
    // Add metadata about the discovery
    let metadata = format!(
        "discovered_at={},peer_count={},active_validators={}",
        chrono::Utc::now().to_rfc3339(),
        discovery_result.peers.len(),
        discovery_result.active_validator_count,
    );
    data.insert(PEERS_METADATA_KEY.to_string(), metadata);

    let cm = ConfigMap {
        metadata: kube::api::ObjectMeta {
            name: Some(config_name.clone()),
            namespace: Some(namespace.to_string()),
            labels: {
                let mut labels = BTreeMap::new();
                labels.insert("app.kubernetes.io/name".to_string(), "stellar-node".to_string());
                labels.insert("app.kubernetes.io/component".to_string(), "peer-discovery".to_string());
                labels.insert("app.kubernetes.io/managed-by".to_string(), "stellar-operator".to_string());
                Some(labels)
            },
            ..Default::default()
        },
        data: Some(data),
        ..Default::default()
    };

    let patch = Patch::Apply(&cm);
    api.patch(&config_name, &PatchParams::apply("stellar-operator"), &patch)
        .await?;

    let action = if existing.is_some() { "updated" } else { "created" };
    info!(
        "Peers ConfigMap {} ({}): {} peers discovered",
        action, config_name, discovery_result.peers.len()
    );

    Ok(peers_changed)
}

/// Trigger a rolling update for affected Stellar nodes
///
/// When peer configuration changes, we need to restart validator pods
/// to load the new KNOWN_PEERS configuration.
/// This is done by updating a pod restart annotation on the StatefulSet.
#[instrument(skip(client), fields(namespace = %namespace))]
pub async fn trigger_rolling_update(
    client: &Client,
    namespace: &str,
) -> Result<()> {
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
    
    // Get all validator nodes
    let node_api: Api<StellarNode> = Api::namespaced(client.clone(), namespace);
    let nodes = node_api
        .list(&ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    let mut restart_count = 0;

    for node in nodes.items.iter() {
        if node.spec.node_type != NodeType::Validator || node.spec.suspended {
            continue;
        }

        let node_name = node.name_any();

        // Create a pod restart patch by updating the template annotations
        // This triggers a rolling restart without explicit pod deletion
        let now = chrono::Utc::now().to_rfc3339();
        let patch = serde_json::json!({
            "spec": {
                "template": {
                    "metadata": {
                        "annotations": {
                            "stellar.org/restarts.io": now
                        }
                    }
                }
            }
        });

        match api
            .patch(
                &node_name,
                &PatchParams::apply("stellar-operator"),
                &Patch::Merge(patch),
            )
            .await
        {
            Ok(_) => {
                info!("Triggered rolling update for validator: {}", node_name);
                restart_count += 1;
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {
                debug!("StatefulSet {} not found, skipping restart", node_name);
            }
            Err(e) => {
                warn!(
                    "Failed to trigger rolling update for {}: {:?}",
                    node_name, e
                );
            }
        }
    }

    if restart_count > 0 {
        info!("Triggered rolling updates for {} validators", restart_count);
    }

    Ok(())
}

/// Watch StellarNode resources and update peer discovery
///
/// This runs in a separate task and periodically discovers peers
/// and updates the shared ConfigMap.
#[instrument(skip(client), fields(namespace = %namespace))]
pub async fn watch_peers(
    client: Client,
    namespace: String,
) {
    let mut last_peers: Vec<String> = Vec::new();

    loop {
        match discover_peers(&client, &namespace, None).await {
            Ok(discovery) => {
                // Check if peer list actually changed
                if discovery.peers != last_peers {
                    info!(
                        "Peer discovery detected changes: {} peers discovered",
                        discovery.peers.len()
                    );

                    // Update ConfigMap
                    match ensure_peers_config_map(&client, &namespace, &discovery).await {
                        Ok(true) => {
                            // Peers changed, trigger rolling update
                            if let Err(e) = trigger_rolling_update(&client, &namespace).await {
                                error!("Failed to trigger rolling update: {:?}", e);
                            }
                        }
                        Ok(false) => {
                            debug!("Peers ConfigMap already up-to-date");
                        }
                        Err(e) => {
                            error!("Failed to ensure peers ConfigMap: {:?}", e);
                        }
                    }

                    last_peers = discovery.peers;
                } else {
                    debug!("No peer changes detected");
                }
            }
            Err(e) => {
                error!("Peer discovery failed: {:?}", e);
            }
        }

        // Recheck every 30 seconds
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_discovery_result_stellar_config() {
        let result = PeerDiscoveryResult {
            peers: vec!["192.168.1.1:11625".to_string(), "192.168.1.2:11625".to_string()],
            active_validator_count: 2,
            changed: false,
        };

        let config = result.to_stellar_config();
        assert!(config.contains("192.168.1.1:11625"));
        assert!(config.contains("192.168.1.2:11625"));
    }
}
