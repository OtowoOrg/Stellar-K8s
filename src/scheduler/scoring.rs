use anyhow::Result;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{Client, ResourceExt};
use std::collections::HashMap;

// Topology labels
const LABEL_ZONE: &str = "topology.kubernetes.io/zone";
const LABEL_REGION: &str = "topology.kubernetes.io/region";

pub async fn score_nodes<'a>(
    pod: &Pod,
    candidates: &[&'a Node],
    client: &Client,
) -> Result<Option<&'a Node>> {
    // 1. Check if carbon-aware scheduling should be used
    if should_use_carbon_aware_scheduling(pod) {
        return score_nodes_carbon_aware(pod, candidates, client).await;
    }

    // 2. Traditional topology-based scoring
    score_nodes_topology_based(pod, candidates, client).await
}

/// Check if pod should use carbon-aware scheduling
fn should_use_carbon_aware_scheduling(pod: &Pod) -> bool {
    // Check for carbon-aware scheduling annotation
    if let Some(annotations) = &pod.metadata.annotations {
        if let Some(value) = annotations.get("stellar.org/carbon-aware") {
            return value == "true" || value == "enabled";
        }
    }

    // Check for read pool pods (they are non-critical)
    if let Some(labels) = &pod.metadata.labels {
        if labels.get("stellar.org/role").map(|s| s.as_str()) == Some("read-replica") {
            return true;
        }
    }

    false
}

/// Carbon-aware scoring using mock data (for now)
async fn score_nodes_carbon_aware<'a>(
    _pod: &Pod,
    candidates: &[&'a Node],
    _client: &Client,
) -> Result<Option<&'a Node>> {
    // Mock carbon intensity data by region
    // In real implementation, this would come from the carbon intensity API
    let mock_carbon_intensity = HashMap::from([
        ("us-west-2", 150.0),      // Washington/Oregon - hydro heavy
        ("us-east-1", 400.0),      // Virginia - mixed
        ("eu-west-1", 300.0),      // Ireland - mixed
        ("eu-central-1", 450.0),   // Frankfurt - coal heavy
        ("ap-southeast-1", 600.0), // Singapore - gas heavy
    ]);

    let mut best_node = None;
    let mut best_intensity = f64::MAX;

    for node in candidates {
        let region = extract_region_from_node(node);

        if let Some(r) = region {
            if let Some(&intensity) = mock_carbon_intensity.get(&r.as_str()) {
                if intensity < best_intensity {
                    best_intensity = intensity;
                    best_node = Some(*node);
                }
            }
        } else if best_node.is_none() {
            // Fallback to first node if no region info
            best_node = Some(*node);
        }
    }

    if let Some(node) = best_node {
        let region = extract_region_from_node(node).unwrap_or_else(|| "unknown".to_string());
        tracing::info!(
            "Carbon-aware scheduling: selected node {} in region {} with intensity {} gCO2/kWh",
            node.name_any(),
            region,
            best_intensity
        );
    }

    Ok(best_node)
}

/// Traditional topology-based scoring
async fn score_nodes_topology_based<'a>(
    pod: &Pod,
    candidates: &[&'a Node],
    client: &Client,
) -> Result<Option<&'a Node>> {
    // 1. Identify "peers"
    // Heuristic: Look for other pods with the same "app" or "component" label in the same namespace
    // In a real implementation, we might check a CRD or a specific annotation on the pod defining its peer group.

    let peers = find_peers(pod, client).await?;

    if peers.is_empty() {
        // No peers to be close to, return the first capable node (or random)
        // Better: spread? For now, just pick first.
        return Ok(candidates.first().copied());
    }

    // 2. Calculate "Center of Gravity" or preferred topology
    // We want to count how many peers are in each Zone/Region.
    let mut zone_counts: HashMap<String, i32> = HashMap::new();
    let mut region_counts: HashMap<String, i32> = HashMap::new();

    for peer in &peers {
        if let Some(node_name) = &peer.spec.as_ref().and_then(|s| s.node_name.clone()) {
            // We need to resolve the peer's node to get its labels.
            // This is expensive to do one-by-one.
            // Optimization: List all nodes once (we passed them in?) -> No, 'candidates' are potential nodes, peers might be on other nodes.
            // We should fetch the node for each peer. Caching would be good here.

            // For simplicity in this POC: We assume we can get node info efficiently or just ignore for now if too expensive without cache.
            // Let's fetch the node.
            let nodes: kube::Api<Node> = kube::Api::all(client.clone());
            if let Ok(node) = nodes.get(node_name).await {
                if let Some(labels) = &node.metadata.labels {
                    if let Some(z) = labels.get(LABEL_ZONE) {
                        *zone_counts.entry(z.clone()).or_insert(0) += 1;
                    }
                    if let Some(r) = labels.get(LABEL_REGION) {
                        *region_counts.entry(r.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    // 3. Score candidates
    // Prefer nodes in zones with high peer counts.
    let mut best_score: i32 = -1;
    let mut best_node = None;

    for node in candidates {
        let mut score: i32 = 0;
        if let Some(labels) = &node.metadata.labels {
            if let Some(z) = labels.get(LABEL_ZONE) {
                score += zone_counts.get(z).copied().unwrap_or(0) * 10; // High weight for same zone
            }
            if let Some(r) = labels.get(LABEL_REGION) {
                score += region_counts.get(r).copied().unwrap_or(0) * 5; // Medium weight for same region
            }
        }

        // Tie-breaker or load balancing could go here
        if score > best_score {
            best_score = score;
            best_node = Some(*node);
        }
    }

    // If all scores are 0 (e.g. no topology labels), just pick first.
    if best_node.is_none() && !candidates.is_empty() {
        Ok(candidates.first().copied())
    } else {
        Ok(best_node)
    }
}

/// Extract region from node labels or name
fn extract_region_from_node(node: &Node) -> Option<String> {
    // Try labels first
    if let Some(labels) = &node.metadata.labels {
        let region_keys = [
            "topology.kubernetes.io/region",
            "failure-domain.beta.kubernetes.io/region",
            "region.kubernetes.io",
        ];

        for key in &region_keys {
            if let Some(region) = labels.get(*key) {
                return Some(region.clone());
            }
        }
    }

    // Try to extract from node name for cloud providers
    let node_name = node.name_any();

    // AWS regions
    let aws_regions = [
        "us-east-1",
        "us-east-2",
        "us-west-1",
        "us-west-2",
        "ca-central-1",
        "eu-west-1",
        "eu-west-2",
        "eu-central-1",
        "eu-north-1",
        "ap-southeast-1",
        "ap-southeast-2",
        "ap-northeast-1",
    ];

    for region in &aws_regions {
        if node_name.contains(region) {
            return Some(region.to_string());
        }
    }

    None
}

async fn find_peers(pod: &Pod, client: &Client) -> Result<Vec<Pod>> {
    let namespace = pod.metadata.namespace.as_deref().unwrap_or("default");
    let pods: kube::Api<Pod> = kube::Api::namespaced(client.clone(), namespace);

    // Filter by specific labels
    // Example: app=stellar-node
    let mut selector = String::new();
    if let Some(labels) = &pod.metadata.labels {
        if let Some(app) = labels.get("app") {
            selector = format!("app={app}");
        }
    }

    if selector.is_empty() {
        return Ok(vec![]);
    }

    let lp = kube::api::ListParams::default().labels(&selector);
    let list = pods.list(&lp).await?;

    // Filter out the pod itself
    let my_name = pod.metadata.name.as_deref().unwrap_or("");
    Ok(list
        .items
        .into_iter()
        .filter(|p| p.metadata.name.as_deref() != Some(my_name))
        .collect())
}
