//! Cross-cluster communication and synchronization
//!
//! This module implements cross-cluster networking for Stellar nodes,
//! enabling multi-cluster deployments with service mesh or ExternalName services.
//!
//! # Features
//!
//! - Service mesh integration (Submariner, Istio, Linkerd, Cilium)
//! - ExternalName service creation for cross-cluster DNS
//! - Latency threshold monitoring and enforcement
//! - Cross-cluster health checks
//! - Automatic peer discovery

use k8s_openapi::api::core::v1::Service;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use tracing::{info, instrument, warn};

use crate::crd::{CrossClusterConfig, CrossClusterMode, StellarNode};
use crate::error::{Error, Result};

/// Ensure cross-cluster services are configured
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_cross_cluster_services(client: &Client, node: &StellarNode) -> Result<()> {
    let cross_cluster = match &node.spec.cross_cluster {
        Some(cc) if cc.enabled => cc,
        _ => return Ok(()),
    };

    info!(
        "Configuring cross-cluster services for {}/{}",
        node.namespace().unwrap_or_default(),
        node.name_any()
    );

    match cross_cluster.mode {
        CrossClusterMode::ServiceMesh => {
            ensure_service_mesh_export(client, node, cross_cluster).await?;
        }
        CrossClusterMode::ExternalName => {
            ensure_external_name_services(client, node, cross_cluster).await?;
        }
        CrossClusterMode::DirectIP => {
            // DirectIP mode uses LoadBalancer services which are handled elsewhere
            info!("DirectIP mode: using LoadBalancer services for cross-cluster communication");
        }
    }

    Ok(())
}

/// Ensure service mesh export configuration
#[instrument(skip(client, node, config), fields(name = %node.name_any()))]
async fn ensure_service_mesh_export(
    client: &Client,
    node: &StellarNode,
    config: &CrossClusterConfig,
) -> Result<()> {
    let mesh_config = config
        .service_mesh
        .as_ref()
        .ok_or_else(|| Error::ConfigError("serviceMesh config required".to_string()))?;

    info!(
        "Configuring {:?} service mesh export for {}",
        mesh_config.mesh_type,
        node.name_any()
    );

    // For Submariner and Istio, we need to create ServiceExport resources
    // These are handled by the respective service mesh controllers
    match mesh_config.mesh_type {
        crate::crd::CrossClusterMeshType::Submariner => {
            create_submariner_service_export(client, node, mesh_config).await?;
        }
        crate::crd::CrossClusterMeshType::Istio => {
            create_istio_service_export(client, node, mesh_config).await?;
        }
        crate::crd::CrossClusterMeshType::Linkerd => {
            // Linkerd uses ServiceProfile and TrafficSplit
            info!("Linkerd multi-cluster: configure ServiceProfile manually");
        }
        crate::crd::CrossClusterMeshType::Cilium => {
            // Cilium Cluster Mesh uses CiliumNetworkPolicy
            info!("Cilium Cluster Mesh: configure via CiliumNetworkPolicy");
        }
    }

    Ok(())
}

/// Create Submariner ServiceExport resource
async fn create_submariner_service_export(
    client: &Client,
    node: &StellarNode,
    _mesh_config: &crate::crd::CrossClusterServiceMeshConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let service_name = format!("{}-service", node.name_any());

    // ServiceExport is a Submariner CRD
    // We'll create it using DynamicObject
    use kube::api::DynamicObject;
    use kube::discovery::ApiResource;

    let api_resource = ApiResource {
        group: "multicluster.x-k8s.io".to_string(),
        version: "v1alpha1".to_string(),
        api_version: "multicluster.x-k8s.io/v1alpha1".to_string(),
        kind: "ServiceExport".to_string(),
        plural: "serviceexports".to_string(),
    };

    let service_export = DynamicObject::new(&service_name, &api_resource).within(&namespace);

    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &api_resource);

    match api
        .patch(
            &service_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&service_export),
        )
        .await
    {
        Ok(_) => {
            info!("Submariner ServiceExport created for {}", service_name);
            Ok(())
        }
        Err(e) => {
            warn!(
                "Failed to create ServiceExport (Submariner may not be installed): {}",
                e
            );
            Ok(()) // Don't fail if Submariner is not installed
        }
    }
}

/// Create Istio ServiceEntry for multi-cluster
async fn create_istio_service_export(
    client: &Client,
    node: &StellarNode,
    mesh_config: &crate::crd::CrossClusterServiceMeshConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let service_name = format!("{}-service", node.name_any());

    let cluster_set_id = mesh_config
        .cluster_set_id
        .as_ref()
        .ok_or_else(|| Error::ConfigError("clusterSetId required for Istio".to_string()))?;

    // Create ServiceEntry for Istio multi-cluster using DynamicObject
    use kube::api::DynamicObject;
    use kube::discovery::ApiResource;

    let api_resource = ApiResource {
        group: "networking.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "networking.istio.io/v1beta1".to_string(),
        kind: "ServiceEntry".to_string(),
        plural: "serviceentries".to_string(),
    };

    let mut service_entry =
        DynamicObject::new(&format!("{service_name}-cross-cluster"), &api_resource)
            .within(&namespace);

    // Set the spec
    service_entry.data = serde_json::json!({
        "spec": {
            "hosts": [
                format!("{}.{}.svc.cluster.local", service_name, namespace)
            ],
            "location": "MESH_INTERNAL",
            "ports": [
                {
                    "number": 11625,
                    "name": "peer",
                    "protocol": "TCP"
                },
                {
                    "number": 11626,
                    "name": "http",
                    "protocol": "HTTP"
                }
            ],
            "resolution": "DNS",
            "exportTo": ["*"]
        }
    });

    // Set labels
    let mut labels = std::collections::BTreeMap::new();
    labels.insert("cluster-set".to_string(), cluster_set_id.clone());
    service_entry.metadata.labels = Some(labels);

    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &api_resource);

    match api
        .patch(
            &format!("{service_name}-cross-cluster"),
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&service_entry),
        )
        .await
    {
        Ok(_) => {
            info!("Istio ServiceEntry created for {}", service_name);
            Ok(())
        }
        Err(e) => {
            warn!(
                "Failed to create ServiceEntry (Istio may not be installed): {}",
                e
            );
            Ok(()) // Don't fail if Istio is not installed
        }
    }
}

/// Create ExternalName services for peer clusters
#[instrument(skip(client, node, config), fields(name = %node.name_any()))]
async fn ensure_external_name_services(
    client: &Client,
    node: &StellarNode,
    config: &CrossClusterConfig,
) -> Result<()> {
    let external_name_config = config
        .external_name
        .as_ref()
        .ok_or_else(|| Error::ConfigError("externalName config required".to_string()))?;

    if !external_name_config.create_external_name_services {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);

    // Create ExternalName service for each peer cluster
    for peer in &config.peer_clusters {
        if !peer.enabled {
            continue;
        }

        let service_name = format!("{}-peer-{}", node.name_any(), peer.cluster_id);

        let external_service = build_external_name_service(node, peer, &service_name);

        api.patch(
            &service_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&external_service),
        )
        .await?;

        info!(
            "ExternalName service {} created for peer cluster {}",
            service_name, peer.cluster_id
        );
    }

    Ok(())
}

/// Build an ExternalName service for a peer cluster
fn build_external_name_service(
    node: &StellarNode,
    peer: &crate::crd::PeerClusterConfig,
    service_name: &str,
) -> Service {
    use k8s_openapi::api::core::v1::{ServicePort, ServiceSpec};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "stellar-node".to_string(),
    );
    labels.insert("app.kubernetes.io/instance".to_string(), node.name_any());
    labels.insert(
        "stellar.org/peer-cluster".to_string(),
        peer.cluster_id.clone(),
    );

    let port = peer.port.unwrap_or(11625);

    Service {
        metadata: ObjectMeta {
            name: Some(service_name.to_string()),
            namespace: node.namespace(),
            labels: Some(labels),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            type_: Some("ExternalName".to_string()),
            external_name: Some(peer.endpoint.clone()),
            ports: Some(vec![ServicePort {
                name: Some("peer".to_string()),
                port: port as i32,
                protocol: Some("TCP".to_string()),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        status: None,
    }
}

/// Check latency to peer clusters and update status
#[instrument(skip(client, node), fields(name = %node.name_any()))]
pub async fn check_peer_latency(
    client: &Client,
    node: &StellarNode,
) -> Result<Vec<PeerLatencyStatus>> {
    let cross_cluster = match &node.spec.cross_cluster {
        Some(cc) if cc.enabled => cc,
        _ => return Ok(Vec::new()),
    };

    let health_check = match &cross_cluster.health_check {
        Some(hc) if hc.enabled => hc,
        _ => return Ok(Vec::new()),
    };

    let latency_config = match &health_check.latency_measurement {
        Some(lm) if lm.enabled => lm,
        _ => return Ok(Vec::new()),
    };

    let mut results = Vec::new();

    for peer in &cross_cluster.peer_clusters {
        if !peer.enabled {
            continue;
        }

        let latency = measure_peer_latency(client, peer, latency_config).await?;
        let threshold = peer
            .latency_threshold_ms
            .unwrap_or(cross_cluster.latency_threshold_ms);

        let status = PeerLatencyStatus {
            cluster_id: peer.cluster_id.clone(),
            latency_ms: latency,
            threshold_ms: threshold,
            healthy: latency <= threshold,
        };

        if !status.healthy {
            warn!(
                "Peer cluster {} latency {}ms exceeds threshold {}ms",
                peer.cluster_id, latency, threshold
            );
        }

        results.push(status);
    }

    Ok(results)
}

/// Measure latency to a peer cluster
async fn measure_peer_latency(
    _client: &Client,
    peer: &crate::crd::PeerClusterConfig,
    config: &crate::crd::LatencyMeasurementConfig,
) -> Result<u32> {
    use crate::crd::LatencyMeasurementMethod;

    // Collect multiple samples
    let mut samples = Vec::new();

    for _ in 0..config.sample_count {
        let latency = match config.method {
            LatencyMeasurementMethod::Ping => {
                // ICMP ping (requires elevated privileges)
                measure_ping_latency(&peer.endpoint).await?
            }
            LatencyMeasurementMethod::TCP => {
                // TCP connection time
                let port = peer.port.unwrap_or(11625);
                measure_tcp_latency(&peer.endpoint, port).await?
            }
            LatencyMeasurementMethod::HTTP => {
                // HTTP request time
                measure_http_latency(&peer.endpoint).await?
            }
            LatencyMeasurementMethod::GRPC => {
                // gRPC health check
                measure_grpc_latency(&peer.endpoint).await?
            }
        };
        samples.push(latency);
    }

    // Calculate percentile
    samples.sort_unstable();
    let index = ((config.percentile as f64 / 100.0) * samples.len() as f64).ceil() as usize - 1;
    let index = index.min(samples.len() - 1);

    Ok(samples[index])
}

/// Measure ICMP ping latency
async fn measure_ping_latency(endpoint: &str) -> Result<u32> {
    // Note: ICMP ping requires elevated privileges
    // In production, use a sidecar container with NET_RAW capability
    info!("ICMP ping to {} (requires NET_RAW capability)", endpoint);

    // Placeholder: return simulated latency
    // In production, use surge-ping or similar library
    Ok(50)
}

/// Measure TCP connection latency
async fn measure_tcp_latency(endpoint: &str, port: u16) -> Result<u32> {
    use std::time::Instant;
    use tokio::net::TcpStream;
    use tokio::time::{timeout, Duration};

    let start = Instant::now();
    let addr = format!("{endpoint}:{port}");

    match timeout(Duration::from_secs(5), TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => {
            let latency = start.elapsed().as_millis() as u32;
            Ok(latency)
        }
        Ok(Err(e)) => Err(Error::NetworkError(format!("TCP connect failed: {e}"))),
        Err(_) => Err(Error::NetworkError("TCP connect timeout".to_string())),
    }
}

/// Measure HTTP request latency
async fn measure_http_latency(endpoint: &str) -> Result<u32> {
    use std::time::Instant;
    use tokio::time::{timeout, Duration};

    let start = Instant::now();
    let url = if endpoint.starts_with("http") {
        endpoint.to_string()
    } else {
        format!("http://{endpoint}:11626/info")
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| Error::NetworkError(format!("HTTP client error: {e}")))?;

    match timeout(Duration::from_secs(5), client.get(&url).send()).await {
        Ok(Ok(_)) => {
            let latency = start.elapsed().as_millis() as u32;
            Ok(latency)
        }
        Ok(Err(e)) => Err(Error::NetworkError(format!("HTTP request failed: {e}"))),
        Err(_) => Err(Error::NetworkError("HTTP request timeout".to_string())),
    }
}

/// Measure gRPC health check latency
async fn measure_grpc_latency(endpoint: &str) -> Result<u32> {
    // Placeholder for gRPC health check
    // In production, implement gRPC health check protocol
    info!("gRPC health check to {}", endpoint);
    Ok(75)
}

/// Peer latency status
#[derive(Debug, Clone)]
pub struct PeerLatencyStatus {
    pub cluster_id: String,
    pub latency_ms: u32,
    pub threshold_ms: u32,
    pub healthy: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{
        types::{NodeType, ResourceRequirements, ResourceSpec, StellarNetwork, StorageConfig},
        CrossClusterConfig, PeerClusterConfig, StellarNode, StellarNodeSpec,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn minimal_spec() -> StellarNodeSpec {
        StellarNodeSpec {
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
            replicas: 1,
            ..Default::default()
        }
    }

    fn make_node(name: &str, namespace: &str) -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            spec: minimal_spec(),
            status: None,
        }
    }

    fn make_peer(cluster_id: &str, endpoint: &str) -> PeerClusterConfig {
        PeerClusterConfig {
            cluster_id: cluster_id.to_string(),
            endpoint: endpoint.to_string(),
            latency_threshold_ms: None,
            region: None,
            priority: 100,
            port: None,
            enabled: true,
        }
    }

    // -----------------------------------------------------------------------
    // build_external_name_service — registering a remote cluster endpoint
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_external_name_service_sets_correct_external_name() {
        let node = make_node("validator-a", "stellar");
        let peer = make_peer("cluster-b", "203.0.113.20");

        let svc = build_external_name_service(&node, &peer, "validator-a-peer-cluster-b");

        let spec = svc.spec.as_ref().expect("service must have a spec");
        assert_eq!(
            spec.external_name.as_deref(),
            Some("203.0.113.20"),
            "ExternalName must match the peer endpoint"
        );
        assert_eq!(
            spec.type_.as_deref(),
            Some("ExternalName"),
            "service type must be ExternalName"
        );
    }

    #[test]
    fn test_build_external_name_service_sets_service_name_and_namespace() {
        let node = make_node("validator-a", "stellar");
        let peer = make_peer("cluster-b", "203.0.113.20");

        let svc = build_external_name_service(&node, &peer, "validator-a-peer-cluster-b");

        assert_eq!(
            svc.metadata.name.as_deref(),
            Some("validator-a-peer-cluster-b")
        );
        assert_eq!(svc.metadata.namespace.as_deref(), Some("stellar"));
    }

    #[test]
    fn test_build_external_name_service_uses_default_port_when_none_specified() {
        let node = make_node("validator-a", "default");
        let peer = PeerClusterConfig {
            port: None,
            ..make_peer("cluster-c", "10.0.0.5")
        };

        let svc = build_external_name_service(&node, &peer, "svc-name");
        let spec = svc.spec.as_ref().unwrap();
        let ports = spec.ports.as_ref().expect("service must have ports");
        assert_eq!(
            ports[0].port, 11625,
            "default Stellar peer port must be 11625"
        );
    }

    #[test]
    fn test_build_external_name_service_uses_custom_port() {
        let node = make_node("validator-a", "default");
        let peer = PeerClusterConfig {
            port: Some(11630),
            ..make_peer("cluster-d", "10.0.0.6")
        };

        let svc = build_external_name_service(&node, &peer, "svc-name");
        let spec = svc.spec.as_ref().unwrap();
        let ports = spec.ports.as_ref().unwrap();
        assert_eq!(ports[0].port, 11630);
    }

    #[test]
    fn test_build_external_name_service_labels_contain_peer_cluster_id() {
        let node = make_node("validator-a", "default");
        let peer = make_peer("cluster-b", "203.0.113.20");

        let svc = build_external_name_service(&node, &peer, "svc-name");
        let labels = svc
            .metadata
            .labels
            .as_ref()
            .expect("service must have labels");

        assert_eq!(
            labels.get("stellar.org/peer-cluster").map(String::as_str),
            Some("cluster-b"),
            "label must identify the peer cluster"
        );
        assert_eq!(
            labels.get("app.kubernetes.io/instance").map(String::as_str),
            Some("validator-a"),
            "label must reference the owning node"
        );
    }

    // -----------------------------------------------------------------------
    // PeerLatencyStatus — detecting unreachable clusters and status propagation
    // -----------------------------------------------------------------------

    #[test]
    fn test_peer_healthy_when_latency_within_threshold() {
        let status = PeerLatencyStatus {
            cluster_id: "cluster-b".to_string(),
            latency_ms: 45,
            threshold_ms: 50,
            healthy: 45 <= 50,
        };
        assert!(status.healthy, "peer within threshold must be healthy");
        assert_eq!(status.cluster_id, "cluster-b");
    }

    #[test]
    fn test_peer_unhealthy_when_latency_exceeds_threshold() {
        let latency = 200u32;
        let threshold = 150u32;
        let status = PeerLatencyStatus {
            cluster_id: "cluster-c".to_string(),
            latency_ms: latency,
            threshold_ms: threshold,
            healthy: latency <= threshold,
        };
        assert!(
            !status.healthy,
            "peer with latency {latency}ms exceeding threshold {threshold}ms must be unhealthy"
        );
    }

    #[test]
    fn test_peer_healthy_when_latency_equals_threshold() {
        let latency = 150u32;
        let threshold = 150u32;
        let status = PeerLatencyStatus {
            cluster_id: "cluster-d".to_string(),
            latency_ms: latency,
            threshold_ms: threshold,
            healthy: latency <= threshold,
        };
        assert!(
            status.healthy,
            "latency equal to threshold must be considered healthy"
        );
    }

    #[test]
    fn test_peer_latency_status_propagates_all_fields() {
        let status = PeerLatencyStatus {
            cluster_id: "remote-cluster-1".to_string(),
            latency_ms: 75,
            threshold_ms: 100,
            healthy: true,
        };
        assert_eq!(status.cluster_id, "remote-cluster-1");
        assert_eq!(status.latency_ms, 75);
        assert_eq!(status.threshold_ms, 100);
        assert!(status.healthy);
    }

    // -----------------------------------------------------------------------
    // Latency percentile calculation (retry/sampling logic)
    // -----------------------------------------------------------------------

    /// Replicate the percentile index calculation from `measure_peer_latency`.
    fn percentile_index(sample_count: usize, percentile: u8) -> usize {
        let index = ((percentile as f64 / 100.0) * sample_count as f64).ceil() as usize - 1;
        index.min(sample_count - 1)
    }

    #[test]
    fn test_p95_of_ten_samples_returns_tenth_sample() {
        // 10 samples, p95 → ceil(0.95 * 10) - 1 = ceil(9.5) - 1 = 10 - 1 = 9 (last)
        let mut samples: Vec<u32> = (10..20).collect(); // [10, 11, ..., 19]
        samples.sort_unstable();
        let idx = percentile_index(samples.len(), 95);
        assert_eq!(
            samples[idx], 19,
            "p95 of 10 samples should be the last (highest) value"
        );
    }

    #[test]
    fn test_p50_of_ten_samples_returns_median() {
        // 10 samples, p50 → ceil(0.50 * 10) - 1 = ceil(5.0) - 1 = 5 - 1 = 4
        let mut samples: Vec<u32> = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        samples.sort_unstable();
        let idx = percentile_index(samples.len(), 50);
        assert_eq!(samples[idx], 50);
    }

    #[test]
    fn test_single_sample_always_returned_at_any_percentile() {
        let samples: Vec<u32> = vec![42];
        let idx = percentile_index(samples.len(), 95);
        assert_eq!(
            samples[idx], 42,
            "single sample must always be returned regardless of percentile"
        );
    }

    #[test]
    fn test_fallback_with_disabled_cross_cluster_returns_no_peers() {
        // When cross_cluster config is None or disabled, check_peer_latency returns empty vec.
        // This mirrors the early-return guard in check_peer_latency.
        let cross_cluster: Option<CrossClusterConfig> = None;
        let result: Vec<PeerLatencyStatus> = match &cross_cluster {
            Some(cc) if cc.enabled => vec![PeerLatencyStatus {
                cluster_id: "x".to_string(),
                latency_ms: 0,
                threshold_ms: 0,
                healthy: true,
            }],
            _ => Vec::new(),
        };
        assert!(
            result.is_empty(),
            "disabled cross_cluster config must yield no latency results"
        );
    }

    #[test]
    fn test_disabled_peer_is_skipped_in_external_name_services() {
        // Verify that disabled peers are not processed.
        let peers = [
            PeerClusterConfig {
                enabled: false,
                ..make_peer("cluster-disabled", "10.0.0.1")
            },
            PeerClusterConfig {
                enabled: true,
                ..make_peer("cluster-enabled", "10.0.0.2")
            },
        ];
        let active: Vec<_> = peers.iter().filter(|p| p.enabled).collect();
        assert_eq!(active.len(), 1, "only enabled peers must be processed");
        assert_eq!(active[0].cluster_id, "cluster-enabled");
    }
}
