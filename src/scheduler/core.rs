use anyhow::Result;
use k8s_openapi::api::core::v1::{Binding, Node, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{api::PostParams, Api, Client, ResourceExt};
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

use super::scoring;

pub struct Scheduler {
    client: Client,
    scheduler_name: String,
}

impl Scheduler {
    pub fn new(client: Client, scheduler_name: String) -> Self {
        Self {
            client,
            scheduler_name,
        }
    }

    pub async fn run(&self) -> Result<()> {
        info!("Starting scheduler: {}", self.scheduler_name);

        loop {
            if let Err(e) = self.schedule_one_cycle().await {
                error!("Error in scheduler cycle: {}", e);
            }
            sleep(Duration::from_secs(5)).await;
        }
    }

    async fn schedule_one_cycle(&self) -> Result<()> {
        let pods: Api<Pod> = Api::all(self.client.clone());
        let nodes: Api<Node> = Api::all(self.client.clone());

        // List all pods and filter for our scheduler and unscheduled
        let all_pods = pods.list(&kube::api::ListParams::default()).await?;

        let mut candidates = Vec::new();
        for p in all_pods {
            let spec = match &p.spec {
                Some(s) => s,
                None => continue,
            };

            if spec.scheduler_name.as_deref() == Some(&self.scheduler_name)
                && spec.node_name.is_none()
            {
                candidates.push(p);
            }
        }

        if candidates.is_empty() {
            return Ok(());
        }

        info!("Found {} unscheduled pods", candidates.len());

        let node_list = nodes.list(&kube::api::ListParams::default()).await?;
        let nodes_vec = node_list.items;

        for pod in candidates {
            self.schedule_pod(&pod, &nodes_vec).await?;
        }

        Ok(())
    }

    async fn schedule_pod(&self, pod: &Pod, nodes: &[Node]) -> Result<()> {
        let pod_name = pod.name_any();
        info!("Attempting to schedule pod: {}", pod_name);

        // 1. Filter nodes (basic checks)
        let filtered_nodes = self.filter_nodes(pod, nodes).await;
        if filtered_nodes.is_empty() {
            warn!("No suitable nodes found for pod {}", pod_name);
            return Ok(());
        }

        // 2. Score nodes
        let best_node = scoring::score_nodes(pod, &filtered_nodes, &self.client).await?;

        if let Some(node) = best_node {
            info!("Binding pod {} to node {}", pod_name, node.name_any());
            self.bind_pod(pod, node).await?;
        } else {
            warn!("No best node found for pod {}", pod_name);
        }

        Ok(())
    }

    async fn filter_nodes<'a>(&self, _pod: &Pod, nodes: &'a [Node]) -> Vec<&'a Node> {
        let mut filtered = Vec::new();

        for n in nodes {
            // 1. Check for unschedulable taint/flag
            if let Some(spec) = &n.spec {
                if spec.unschedulable == Some(true) {
                    continue;
                }
            }

            // 2. Resource check (Stub for CPU/Mem)
            // In a production scheduler, we would check if node has enough capacity

            filtered.push(n);
        }

        // 3. Quorum-aware filtering
        // If this is a validator, we ideally want to avoid nodes that already host a peer.
        // However, filtering is "hard" - if all nodes have peers, we'd fail to schedule.
        // So we keep filtering light and let scoring do the heavy lifting for "best" node.

        filtered
    }

    async fn bind_pod(&self, pod: &Pod, node: &Node) -> Result<()> {
        let namespace = pod.namespace().unwrap_or_else(|| "default".into());
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &namespace);
        let pod_name = pod.name_any();
        let node_name = node.name_any();

        let binding = Binding {
            metadata: ObjectMeta {
                name: Some(pod_name.clone()),
                namespace: Some(namespace.clone()),
                ..ObjectMeta::default()
            },
            target: k8s_openapi::api::core::v1::ObjectReference {
                api_version: Some("v1".into()),
                kind: Some("Node".into()),
                name: Some(node_name.clone()),
                ..Default::default()
            },
        };

        // Serialize the binding to JSON bytes
        let binding_bytes = serde_json::to_vec(&binding)?;

        // Create binding subresource
        let pp = PostParams::default();
        let _: Binding = pods
            .create_subresource("binding", &pod_name, &pp, binding_bytes)
            .await?;

        info!("Successfully bound {} to {}", pod_name, node_name);
        Ok(())
    }
}
