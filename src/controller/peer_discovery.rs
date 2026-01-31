//! Dynamic peer discovery for Stellar nodes
//!
//! Watches all StellarNode resources in the cluster and maintains a shared ConfigMap
//! with the latest peer IPs and ports. Automatically triggers configuration reload
//! when peers change.
//!
//! ## Implementation
//!
//! Uses a polling approach to discover peers:
//! - Polls all StellarNode resources every 30 seconds
//! - Filters for Validator nodes only
//! - Extracts peer information (IP, port, namespace, name)
//! - Updates shared ConfigMap when peer list changes
//! - Triggers config reload on healthy validators

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use k8s_openapi::api::core::v1::{ConfigMap, Service};
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    client::Client,
    ResourceExt,
};
use serde_json::json;
use tracing::{debug, error, info, instrument, warn};

use crate::crd::{NodeType, StellarNode};
use crate::error::{Error, Result};

/// Peer information extracted from a StellarNode
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PeerInfo {
    pub name: String,
    pub namespace: String,
    pub node_type: NodeType,
    pub ip: String,
    pub port: u16,
}

impl PeerInfo {
    /// Format peer as "ip:port" for Stellar Core configuration
    pub fn to_peer_string(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }

    /// Format peer as JSON for the shared ConfigMap
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "name": self.name,
            "namespace": self.namespace,
            "nodeType": self.node_type.to_string(),
            "ip": self.ip,
            "port": self.port,
            "peerString": self.to_peer_string(),
        })
    }
}

/// Configuration for peer discovery
#[derive(Clone, Debug)]
pub struct PeerDiscoveryConfig {
    /// Namespace where the shared peers ConfigMap is stored
    pub config_namespace: String,
    /// Name of the shared peers ConfigMap
    pub config_map_name: String,
    /// Port used by Stellar Core for peer connections
    pub peer_port: u16,
}

impl Default for PeerDiscoveryConfig {
    fn default() -> Self {
        Self {
            config_namespace: "stellar-system".to_string(),
            config_map_name: "stellar-peers".to_string(),
            peer_port: 11625,
        }
    }
}

/// Peer discovery manager
pub struct PeerDiscoveryManager {
    client: Client,
    config: PeerDiscoveryConfig,
}

impl PeerDiscoveryManager {
    pub fn new(client: Client, config: PeerDiscoveryConfig) -> Self {
        Self { client, config }
    }

    /// Start the peer discovery watcher
    /// This runs continuously and updates the shared ConfigMap when peers change
    pub async fn run(&self) -> Result<()> {
        info!(
            "Starting peer discovery for namespace: {}",
            self.config.config_namespace
        );

        let stellar_nodes: Api<StellarNode> = Api::all(self.client.clone());
        let mut last_peers: HashSet<PeerInfo> = HashSet::new();

        loop {
            // Poll for nodes
            match stellar_nodes.list(&Default::default()).await {
                Ok(nodes) => {
                    let mut current_peers = HashSet::new();

                    for node in nodes.items {
                        if let Err(e) = self.process_node_event(&node, &mut current_peers).await {
                            debug!("Error processing node {}: {}", node.name_any(), e);
                        }
                    }

                    // Check if peers changed
                    if current_peers != last_peers {
                        info!(
                            "Peer list changed: {} -> {} peers",
                            last_peers.len(),
                            current_peers.len()
                        );
                        if let Err(e) = self.update_peers_config_map(&current_peers).await {
                            error!("Failed to update peers ConfigMap: {}", e);
                        }
                        last_peers = current_peers;
                    }
                }
                Err(e) => {
                    error!("Failed to list StellarNodes: {}", e);
                }
            }

            // Sleep for 30 seconds before the next poll cycle
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    /// Process a node event and update peers if needed
    async fn process_node_event(
        &self,
        node: &StellarNode,
        current_peers: &mut HashSet<PeerInfo>,
    ) -> Result<()> {
        // Only include validators in peer discovery
        if node.spec.node_type != NodeType::Validator {
            return Ok(());
        }

        // Skip suspended nodes
        if node.spec.suspended {
            return Ok(());
        }

        // Extract peer information
        if let Some(peer) = self.extract_peer_info(node).await? {
            current_peers.insert(peer);
        }

        Ok(())
    }

    /// Extract peer information from a StellarNode
    async fn extract_peer_info(&self, node: &StellarNode) -> Result<Option<PeerInfo>> {
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let name = node.name_any();

        // Get the service to find the IP
        let services: Api<Service> = Api::namespaced(self.client.clone(), &namespace);
        let service_name = format!("{name}-service");

        match services.get(&service_name).await {
            Ok(service) => {
                // Try to get cluster IP
                if let Some(spec) = &service.spec {
                    if let Some(cluster_ip) = &spec.cluster_ip {
                        if cluster_ip != "None" {
                            return Ok(Some(PeerInfo {
                                name: name.clone(),
                                namespace: namespace.clone(),
                                node_type: node.spec.node_type.clone(),
                                ip: cluster_ip.clone(),
                                port: self.config.peer_port,
                            }));
                        }
                    }
                }

                // Try to get external IP (LoadBalancer)
                if let Some(status) = &service.status {
                    if let Some(ingress) = &status.load_balancer {
                        if let Some(ingresses) = &ingress.ingress {
                            for ing in ingresses {
                                if let Some(ip) = &ing.ip {
                                    return Ok(Some(PeerInfo {
                                        name: name.clone(),
                                        namespace: namespace.clone(),
                                        node_type: node.spec.node_type.clone(),
                                        ip: ip.clone(),
                                        port: self.config.peer_port,
                                    }));
                                }
                            }
                        }
                    }
                }

                debug!("Service {} found but no IP available yet", service_name);
                Ok(None)
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {
                debug!("Service {} not found yet", service_name);
                Ok(None)
            }
            Err(e) => {
                warn!("Error fetching service {}: {}", service_name, e);
                Ok(None)
            }
        }
    }

    /// Update the shared peers ConfigMap with current peer list
    #[instrument(skip(self, peers))]
    async fn update_peers_config_map(&self, peers: &HashSet<PeerInfo>) -> Result<()> {
        let api: Api<ConfigMap> =
            Api::namespaced(self.client.clone(), &self.config.config_namespace);

        let mut data = BTreeMap::new();

        // Add peers as JSON array
        let peers_json: Vec<serde_json::Value> = peers.iter().map(|p| p.to_json()).collect();
        data.insert(
            "peers.json".to_string(),
            serde_json::to_string_pretty(&peers_json).unwrap_or_else(|_| "[]".to_string()),
        );

        // Add peers as simple list (ip:port format)
        let peers_list: Vec<String> = peers.iter().map(|p| p.to_peer_string()).collect();
        data.insert("peers.txt".to_string(), peers_list.join("\n"));

        // Add peer count
        data.insert("peer_count".to_string(), peers.len().to_string());

        let cm = ConfigMap {
            metadata: kube::api::ObjectMeta {
                name: Some(self.config.config_map_name.clone()),
                namespace: Some(self.config.config_namespace.clone()),
                labels: Some({
                    let mut labels = BTreeMap::new();
                    labels.insert("app".to_string(), "stellar-operator".to_string());
                    labels.insert("component".to_string(), "peer-discovery".to_string());
                    labels
                }),
                ..Default::default()
            },
            data: Some(data),
            ..Default::default()
        };

        let patch = Patch::Apply(&cm);
        api.patch(
            &self.config.config_map_name,
            &PatchParams::apply("stellar-operator").force(),
            &patch,
        )
        .await?;

        info!("Updated peers ConfigMap with {} peers", peers.len());

        Ok(())
    }
}

/// Get all validator peers from the shared ConfigMap
pub async fn get_peers_from_config_map(
    client: &Client,
    config: &PeerDiscoveryConfig,
) -> Result<Vec<PeerInfo>> {
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &config.config_namespace);

    match api.get(&config.config_map_name).await {
        Ok(cm) => {
            if let Some(data) = cm.data {
                if let Some(peers_json) = data.get("peers.json") {
                    match serde_json::from_str::<Vec<serde_json::Value>>(peers_json) {
                        Ok(peers_values) => {
                            let peers: Vec<PeerInfo> = peers_values
                                .iter()
                                .filter_map(|v| {
                                    Some(PeerInfo {
                                        name: v.get("name")?.as_str()?.to_string(),
                                        namespace: v.get("namespace")?.as_str()?.to_string(),
                                        node_type: match v.get("nodeType")?.as_str()? {
                                            "Validator" => NodeType::Validator,
                                            "Horizon" => NodeType::Horizon,
                                            "SorobanRpc" => NodeType::SorobanRpc,
                                            _ => return None,
                                        },
                                        ip: v.get("ip")?.as_str()?.to_string(),
                                        port: v.get("port")?.as_u64()? as u16,
                                    })
                                })
                                .collect();
                            return Ok(peers);
                        }
                        Err(e) => {
                            warn!("Failed to parse peers.json: {}", e);
                        }
                    }
                }
            }
            Ok(Vec::new())
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            debug!("Peers ConfigMap not found yet");
            Ok(Vec::new())
        }
        Err(e) => Err(Error::KubeError(e)),
    }
}

/// Trigger configuration reload for a specific node
pub async fn trigger_peer_config_reload(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    // Get the pod to find its IP
    let pods: Api<k8s_openapi::api::core::v1::Pod> = Api::namespaced(client.clone(), &namespace);

    let label_selector = format!("app={name}");
    let params = ListParams::default().labels(&label_selector);

    match pods.list(&params).await {
        Ok(pod_list) => {
            for pod in pod_list.items {
                if let Some(status) = &pod.status {
                    if let Some(pod_ip) = &status.pod_ip {
                        debug!("Triggering config reload for pod at {}", pod_ip);
                        if let Err(e) = trigger_config_reload_http(pod_ip).await {
                            warn!("Failed to trigger config reload: {}", e);
                        }
                    }
                }
            }
        }
        Err(e) => {
            warn!("Failed to list pods for {}/{}: {}", namespace, name, e);
        }
    }

    Ok(())
}

/// Trigger config reload via HTTP command
async fn trigger_config_reload_http(pod_ip: &str) -> Result<()> {
    let url = format!("http://{pod_ip}:11626/http-command?admin=true&command=config-reload");

    debug!("Triggering config-reload via {}", url);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| Error::ConfigError(format!("Failed to build HTTP client: {e}")))?;

    let response = client.get(&url).send().await.map_err(Error::HttpError)?;

    if !response.status().is_success() {
        return Err(Error::ConfigError(format!(
            "Failed to trigger config-reload: status {}",
            response.status()
        )));
    }

    info!("Successfully triggered config-reload for pod at {}", pod_ip);
    Ok(())
}
