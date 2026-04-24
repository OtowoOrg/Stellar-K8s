//! Automated Node-Drain Orchestrator for Stellar Core
//!
//! Intelligently manages node drains by gracefully migrating Stellar Core pods
//! while maintaining quorum liveness.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::core::v1::{Node, Pod};
use k8s_openapi::api::policy::v1::Eviction;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{
    api::{Api, ListParams, PostParams},
    runtime::{
        events::{EventType, Recorder, Reporter},
        watcher::{self, Config},
    },
    Client, Resource, ResourceExt,
};
use serde::Deserialize;
use tracing::{debug, error, info, instrument, warn};

use crate::controller::health::check_node_health;
use crate::crd::{NodeType, StellarNode};
use crate::error::{Error, Result};

/// NodeDrainOrchestrator manages the lifecycle of Stellar Core pods during node drains.
pub struct NodeDrainOrchestrator {
    client: Client,
    reporter: Reporter,
}

#[derive(Debug, Deserialize)]
struct StellarCoreInfo {
    info: InfoSection,
}

#[derive(Debug, Deserialize)]
struct InfoSection {
    state: String,
}

impl NodeDrainOrchestrator {
    pub fn new(client: Client, reporter: Reporter) -> Self {
        Self { client, reporter }
    }

    /// Start the node watcher loop
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let nodes: Api<Node> = Api::all(self.client.clone());
        let wc = Config::default();

        info!("Starting Node Drain Orchestrator watcher");

        let mut stream = watcher::watcher(nodes, wc).boxed();
        while let Some(event) = stream.next().await {
            match event {
                Ok(watcher::Event::Applied(node)) => {
                    if self.is_node_cordoned(&node) {
                        self.handle_cordoned_node(node).await?;
                    }
                }
                Ok(watcher::Event::Deleted(node)) => {
                    debug!("Node {} deleted, skipping", node.name_any());
                }
                Ok(watcher::Event::Restarted(nodes)) => {
                    for node in nodes {
                        if self.is_node_cordoned(&node) {
                            self.handle_cordoned_node(node).await?;
                        }
                    }
                }
                Err(e) => error!("Node watcher error: {}", e),
            }
        }
        Ok(())
    }

    /// Check if a node is cordoned (SchedulingDisabled)
    fn is_node_cordoned(&self, node: &Node) -> bool {
        node.spec
            .as_ref()
            .map_or(false, |spec| spec.unschedulable.unwrap_or(false))
    }

    /// Handle a cordoned node by migrating Stellar pods gracefully
    #[instrument(skip(self, node), fields(node = %node.name_any()))]
    async fn handle_cordoned_node(&self, node: Node) -> Result<()> {
        let node_name = node.name_any();
        info!(
            "Node {} is cordoned, checking for Stellar Core pods",
            node_name
        );

        let pods: Api<Pod> = Api::all(self.client.clone());
        let lp = ListParams::default().fields(&format!("spec.nodeName={}", node_name));
        let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

        for pod in pod_list.items {
            if self.is_stellar_pod(&pod) {
                self.manage_pod_migration(pod, &node).await?;
            }
        }

        Ok(())
    }

    /// Check if a pod is a Stellar Core pod managed by this operator
    fn is_stellar_pod(&self, pod: &Pod) -> bool {
        pod.labels()
            .get("app.kubernetes.io/managed-by")
            .map_or(false, |m| m == "stellar-operator")
    }

    /// Manage the graceful migration of a single pod
    async fn manage_pod_migration(&self, pod: Pod, node: &Node) -> Result<()> {
        let pod_name = pod.name_any();
        let namespace = pod.namespace().unwrap_or_else(|| "default".to_string());
        let recorder = Recorder::new(
            self.client.clone(),
            self.reporter.clone(),
            pod.object_ref(&()),
        );

        info!(
            "Managing migration for pod {}/{} on cordoned node",
            namespace, pod_name
        );

        // 1. Wait until the node has 'Caught up' on a peer before exiting
        if !self.is_pod_caught_up(&pod).await? {
            info!("Pod {} is not caught up yet, waiting...", pod_name);
            recorder
                .publish(kube::runtime::events::Event {
                    type_: EventType::Normal,
                    reason: "MigrationWaiting".into(),
                    note: Some(format!(
                        "Waiting for pod {} to catch up before migration",
                        pod_name
                    )),
                    action: "Migrating".into(),
                    secondary: None,
                })
                .await
                .map_err(Error::KubeError)?;

            return Ok(());
        }

        info!("Pod {} is caught up, proceeding with eviction", pod_name);

        // 2. Coordinate with PDBs (Kubernetes Eviction API handles this)
        let eviction = Eviction {
            metadata: ObjectMeta {
                name: Some(pod_name.clone()),
                namespace: Some(namespace.clone()),
                ..Default::default()
            },
            delete_options: None,
        };

        let pod_api: Api<Pod> = Api::namespaced(self.client.clone(), &namespace);
        match pod_api
            .evict(&pod_name, &PostParams::default(), &eviction)
            .await
        {
            Ok(_) => {
                info!("Successfully triggered eviction for pod {}", pod_name);
                recorder
                    .publish(kube::runtime::events::Event {
                        type_: EventType::Normal,
                        reason: "MigrationStarted".into(),
                        note: Some(format!(
                            "Gracefully migrating pod {} from cordoned node",
                            pod_name
                        )),
                        action: "Migrating".into(),
                        secondary: None,
                    })
                    .await
                    .map_err(Error::KubeError)?;
            }
            Err(e) => {
                warn!("Failed to evict pod {}: {}. Will retry.", pod_name, e);
            }
        }

        Ok(())
    }

    /// Check if a Stellar pod is caught up (Synced!)
    async fn is_pod_caught_up(&self, pod: &Pod) -> Result<bool> {
        let pod_ip = match pod.status.as_ref().and_then(|s| s.pod_ip.as_ref()) {
            Some(ip) => ip,
            None => return Ok(false),
        };

        let node_type = self.get_node_type(pod);
        let namespace = pod.namespace().unwrap_or_else(|| "default".to_string());

        match node_type {
            Some(NodeType::Validator) => {
                let url = format!("http://{}:11626/info", pod_ip);
                let http_client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(2))
                    .build()
                    .map_err(|e| Error::ConfigError(e.to_string()))?;

                match http_client.get(&url).send().await {
                    Ok(resp) => {
                        if let Ok(info) = resp.json::<StellarCoreInfo>().await {
                            let caught_up = info.info.state == "Synced!";
                            debug!(
                                "Validator pod {} state: {}, caught_up: {}",
                                pod.name_any(),
                                info.info.state,
                                caught_up
                            );
                            Ok(caught_up)
                        } else {
                            debug!("Failed to parse info from validator pod {}", pod.name_any());
                            Ok(false)
                        }
                    }
                    Err(e) => {
                        debug!("Failed to query validator pod {}: {}", pod.name_any(), e);
                        Ok(false)
                    }
                }
            }
            Some(NodeType::Horizon) | Some(NodeType::SorobanRpc) => {
                // For Horizon and Soroban, we fetch the StellarNode and use its status or health check
                let node_name = pod.labels().get("app.kubernetes.io/instance");
                if let Some(name) = node_name {
                    let stellar_nodes: Api<StellarNode> =
                        Api::namespaced(self.client.clone(), &namespace);
                    if let Ok(node) = stellar_nodes.get(name).await {
                        // Use existing health check logic
                        match check_node_health(&self.client, &node, None).await {
                            Ok(health) => {
                                debug!("Node {} health: synced={}", name, health.synced);
                                Ok(health.synced)
                            }
                            Err(_) => Ok(false),
                        }
                    } else {
                        Ok(false)
                    }
                } else {
                    Ok(true)
                }
            }
            None => Ok(true),
        }
    }

    fn get_node_type(&self, pod: &Pod) -> Option<NodeType> {
        pod.labels()
            .get("stellar.org/node-type")
            .and_then(|t| match t.as_str() {
                "validator" => Some(NodeType::Validator),
                "horizon" => Some(NodeType::Horizon),
                "soroban-rpc" => Some(NodeType::SorobanRpc),
                _ => None,
            })
    }
}
