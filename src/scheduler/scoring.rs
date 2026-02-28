use anyhow::Result;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{Client, ResourceExt};
use std::collections::HashMap;
use toml;
use tracing;

// Topology labels
const LABEL_ZONE: &str = "topology.kubernetes.io/zone";
const LABEL_REGION: &str = "topology.kubernetes.io/region";

pub async fn score_nodes<'a>(
    pod: &Pod,
    candidates: &[&'a Node],
    client: &Client,
) -> Result<Option<&'a Node>> {
    // 1. Check for Quorum Proximity scheduling (Stellar Validators)
    if is_validator_pod(pod) {
        if let Ok(Some(node)) = score_nodes_quorum_proximity(pod, candidates, client).await {
            return Ok(Some(node));
        }
    }

    // 2. Check if carbon-aware scheduling should be used
    if should_use_carbon_aware_scheduling(pod) {
        return score_nodes_carbon_aware(pod, candidates, client).await;
    }

    // 3. Traditional topology-based scoring
    score_nodes_topology_based(pod, candidates, client).await
}

/// Check if pod is a Stellar validator
fn is_validator_pod(pod: &Pod) -> bool {
    pod.metadata
        .labels
        .as_ref()
        .and_then(|l| l.get("stellar.org/node-type"))
        .map(|s| s == "Validator")
        .unwrap_or(false)
}

/// Score nodes based on Stellar quorum set proximity and redundancy.
/// Prioritizes nodes that provide the best latency/redundancy balance.
async fn score_nodes_quorum_proximity<'a>(
    pod: &Pod,
    candidates: &[&'a Node],
    client: &Client,
) -> Result<Option<&'a Node>> {
    let instance_name = pod
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get("app.kubernetes.io/instance"))
        .ok_or_else(|| anyhow::anyhow!("Pod missing instance label"))?;

    let namespace = pod.metadata.namespace.as_deref().unwrap_or("default");
    let stellar_nodes: kube::Api<crate::crd::StellarNode> =
        kube::Api::namespaced(client.clone(), namespace);

    let node_cr = match stellar_nodes.get(instance_name).await {
        Ok(n) => n,
        Err(_) => return Ok(None),
    };

    let quorum_set_toml = match node_cr
        .spec
        .validator_config
        .as_ref()
        .and_then(|c| c.quorum_set.as_ref())
    {
        Some(q) => q,
        None => return Ok(None),
    };

    // Parse peer names/keys from quorumSet TOML
    let peer_names = extract_peer_names_from_toml(quorum_set_toml);
    if peer_names.is_empty() {
        return Ok(None);
    }

    // Find where peers are currently running
    let mut peer_nodes = Vec::new();
    let all_pods: kube::Api<Pod> = kube::Api::all(client.clone());
    let all_nodes: kube::Api<Node> = kube::Api::all(client.clone());

    for peer_name in peer_names {
        // Find pods for this peer instance
        let lp = kube::api::ListParams::default()
            .labels(&format!("app.kubernetes.io/instance={}", peer_name));
        if let Ok(pods) = all_pods.list(&lp).await {
            for p in pods {
                if let Some(node_name) = p.spec.as_ref().and_then(|s| s.node_name.as_ref()) {
                    if let Ok(node) = all_nodes.get(node_name).await {
                        peer_nodes.push(node);
                    }
                }
            }
        }
    }

    if peer_nodes.is_empty() {
        // Fallback to topology-based if no peers found running
        return Ok(None);
    }

    // Score candidates
    let mut best_score = i64::MIN;
    let mut best_node = None;

    for node in candidates {
        let mut score: i64 = 0;
        let node_name = node.name_any();
        let node_zone = node
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get(LABEL_ZONE));
        let node_region = node
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get(LABEL_REGION));

        for peer_node in &peer_nodes {
            let peer_node_name = peer_node.name_any();
            let peer_zone = peer_node
                .metadata
                .labels
                .as_ref()
                .and_then(|l| l.get(LABEL_ZONE));
            let peer_region = peer_node
                .metadata
                .labels
                .as_ref()
                .and_then(|l| l.get(LABEL_REGION));

            // 1. Anti-affinity: Strongly discourage same node
            if node_name == peer_node_name {
                score -= 1000;
            }

            // 2. Redundancy: Prefer different zones
            if let (Some(nz), Some(pz)) = (node_zone, peer_zone) {
                if nz != pz {
                    score += 100;
                } else {
                    score -= 50; // Same zone penalty
                }
            }

            // 3. Latency: Prefer same region (low latency)
            if let (Some(nr), Some(pr)) = (node_region, peer_region) {
                if nr == pr {
                    score += 50;
                } else {
                    score -= 20; // Different region penalty
                }
            }
        }

        if score > best_score {
            best_score = score;
            best_node = Some(*node);
        }
    }

    tracing::info!(
        "Quorum Proximity scoring for pod {}: selected node {} with score {}",
        pod.name_any(),
        best_node.map(|n| n.name_any()).unwrap_or_default(),
        best_score
    );

    Ok(best_node)
}

/// Helper to extract peer instance names from Stellar Core quorum set TOML.
/// Handles both [VALIDATORS] map and [QUORUM_SET] VSL formats.
fn extract_peer_names_from_toml(toml_str: &str) -> Vec<String> {
    let mut names = Vec::new();

    // Try to parse as TOML Table
    if let Ok(value) = toml_str.parse::<toml::Value>() {
        // Case 1: [VALIDATORS] section
        if let Some(validators) = value.get("VALIDATORS").and_then(|v| v.as_table()) {
            for key in validators.keys() {
                names.push(key.clone());
            }
        }

        // Case 2: [QUORUM_SET] section (from VSL)
        if let Some(qs) = value.get("QUORUM_SET").and_then(|v| v.as_table()) {
            if let Some(validators) = qs.get("VALIDATORS").and_then(|v| v.as_array()) {
                for v in validators {
                    if let Some(s) = v.as_str() {
                        // VSL uses public keys, but in K8S we might use names
                        // For now, if it looks like a public key (starts with G),
                        // we'd need a mapping. If it's a simple name, use it.
                        if !s.starts_with('G') {
                            names.push(s.to_string());
                        }
                    }
                }
            }
        }
    }

    names
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
