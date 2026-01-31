//! Kubernetes resource builders for StellarNode
//!
//! This module creates and manages the underlying Kubernetes resources
//! (Deployments, StatefulSets, Services, PVCs, ConfigMaps) for each StellarNode.

use crate::controller::resource_meta::merge_resource_meta;

use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, StatefulSet, StatefulSetSpec};
use k8s_openapi::api::autoscaling::v2::{
    CrossVersionObjectReference, HPAScalingPolicy, HPAScalingRules, HorizontalPodAutoscaler,
    HorizontalPodAutoscalerBehavior, HorizontalPodAutoscalerSpec, MetricIdentifier, MetricSpec,
    MetricTarget, ObjectMetricSource,
};
use k8s_openapi::api::core::v1::{
    ConfigMap, Container, ContainerPort, EnvVar, EnvVarSource, PersistentVolumeClaim,
    PersistentVolumeClaimSpec, PodSpec, PodTemplateSpec, ResourceRequirements as K8sResources,
    SecretKeySelector, Service, ServicePort, ServiceSpec, Volume, VolumeMount,
    VolumeResourceRequirements,
};
use k8s_openapi::api::networking::v1::{
    HTTPIngressPath, HTTPIngressRuleValue, IPBlock, Ingress, IngressBackend, IngressRule,
    IngressServiceBackend, IngressSpec, IngressTLS, NetworkPolicy, NetworkPolicyIngressRule,
    NetworkPolicyPeer, NetworkPolicyPort, NetworkPolicySpec, ServiceBackendPort,
};
use k8s_openapi::api::policy::v1::{PodDisruptionBudget, PodDisruptionBudgetSpec};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta, OwnerReference};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, DeleteParams, Patch, PatchParams, PostParams};
use kube::{Client, Resource, ResourceExt};
use tracing::{info, instrument, warn};

use crate::crd::{
    BackupConfiguration, BarmanObjectStore, BootstrapConfiguration, Cluster, ClusterSpec,
    ExternalTrafficPolicy, HistoryMode, HsmProvider, IngressConfig, InitDbConfiguration, KeySource,
    LoadBalancerConfig, LoadBalancerMode, ManagedDatabaseConfig, MonitoringConfiguration,
    NetworkPolicyConfig, NodeType, PgBouncerSpec, Pooler, PoolerCluster, PoolerSpec,
    PostgresConfiguration, RolloutStrategy, S3Credentials,
    SecretKeySelector as CnpgSecretKeySelector, StellarNode, StorageConfiguration,
    WalBackupConfiguration,
};
use crate::error::{Error, Result};

/// Get the standard labels for a StellarNode's resources
fn standard_labels(node: &StellarNode) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_string(),
        "stellar-node".to_string(),
    );
    labels.insert("app.kubernetes.io/instance".to_string(), node.name_any());
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        node.spec.node_type.to_string().to_lowercase(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "stellar-operator".to_string(),
    );
    labels.insert(
        "stellar.org/node-type".to_string(),
        node.spec.node_type.to_string(),
    );
    labels
}

/// Create an OwnerReference for garbage collection
fn owner_reference(node: &StellarNode) -> OwnerReference {
    OwnerReference {
        api_version: StellarNode::api_version(&()).to_string(),
        kind: StellarNode::kind(&()).to_string(),
        name: node.name_any(),
        uid: node.metadata.uid.clone().unwrap_or_default(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    }
}

/// Build the resource name for a given component
fn resource_name(node: &StellarNode, suffix: &str) -> String {
    format!("{}-{}", node.name_any(), suffix)
}

// ============================================================================
// PersistentVolumeClaim
// ============================================================================

/// Ensure a PersistentVolumeClaim exists for the node
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_pvc(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "data");

    let pvc = build_pvc(node);

    match api.get(&name).await {
        Ok(_existing) => {
            // PVCs are mostly immutable, just ensure it exists
            info!("PVC {} already exists", name);
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            info!("Creating PVC {}", name);
            api.create(&PostParams::default(), &pvc).await?;
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

fn build_pvc(node: &StellarNode) -> PersistentVolumeClaim {
    let labels = standard_labels(node);
    let name = resource_name(node, "data");

    let mut requests = BTreeMap::new();
    let effective_storage_size = if node.spec.storage.size.is_empty() {
        // Default based on history_mode override
        match node.spec.history_mode {
            HistoryMode::Full => "1500Gi".to_string(), // Approximate for full history
            HistoryMode::Recent => "100Gi".to_string(), // Approximate for recent history
        }
    } else {
        node.spec.storage.size.clone()
    };
    requests.insert("storage".to_string(), Quantity(effective_storage_size));

    // Merge custom annotations from storage config with existing annotations
    let annotations = node.spec.storage.annotations.clone().unwrap_or_default();

    PersistentVolumeClaim {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: node.namespace(),
                labels: Some(labels),
                annotations: if annotations.is_empty() {
                    None
                } else {
                    Some(annotations)
                },
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None,
        ),
        spec: Some(PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOnce".to_string()]),
            storage_class_name: Some(node.spec.storage.storage_class.clone()),
            resources: Some(VolumeResourceRequirements {
                requests: Some(requests),
                ..Default::default()
            }),
            ..Default::default()
        }),
        status: None,
    }
}

/// Delete the PersistentVolumeClaim for a node
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_pvc(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "data");

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted PVC {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            warn!("PVC {} not found, already deleted", name);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

// ============================================================================
// ConfigMap
// ============================================================================

/// Ensure a ConfigMap exists with node configuration
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_config_map(
    client: &Client,
    node: &StellarNode,
    quorum_override: Option<String>,
    enable_mtls: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "config");

    let cm = build_config_map(node, quorum_override, enable_mtls);

    let patch = Patch::Apply(&cm);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    Ok(())
}

fn build_config_map(
    node: &StellarNode,
    quorum_override: Option<String>,
    enable_mtls: bool,
) -> ConfigMap {
    let labels = standard_labels(node);
    let name = resource_name(node, "config");

    let mut data = BTreeMap::new();

    // Add network-specific configuration
    data.insert(
        "NETWORK_PASSPHRASE".to_string(),
        node.spec.network.passphrase().to_string(),
    );

    // Add mTLS configuration if enabled
    if enable_mtls {
        data.insert("MTLS_ENABLED".to_string(), "true".to_string());
    }

    // Add node-type-specific configuration
    match &node.spec.node_type {
        NodeType::Validator => {
            let mut core_cfg = String::new();
            if let Some(config) = &node.spec.validator_config {
                let quorum = quorum_override.or_else(|| config.quorum_set.clone());
                if let Some(q) = quorum {
                    core_cfg.push_str(&q);
                }
            }

            if enable_mtls {
                core_cfg.push_str("\n# mTLS Configuration\n");
                core_cfg.push_str("HTTP_PORT_SECURE=true\n");
                core_cfg.push_str("TLS_CERT_FILE=\"/etc/stellar/tls/tls.crt\"\n");
                core_cfg.push_str("TLS_KEY_FILE=\"/etc/stellar/tls/tls.key\"\n");
            }

            // History Mode Configuration
            match node.spec.history_mode {
                HistoryMode::Full => {
                    core_cfg.push_str("\n# Full History Mode\n");
                    core_cfg.push_str("CATCHUP_COMPLETE=true\n");
                }
                HistoryMode::Recent => {
                    core_cfg.push_str("\n# Recent History Mode\n");
                    core_cfg.push_str("CATCHUP_COMPLETE=false\n");
                    core_cfg.push_str("CATCHUP_RECENT=60480\n"); // ~1 week
                }
            }

            if !core_cfg.is_empty() {
                data.insert("stellar-core.cfg".to_string(), core_cfg);
            }
        }
        NodeType::Horizon => {
            if let Some(config) = &node.spec.horizon_config {
                data.insert(
                    "STELLAR_CORE_URL".to_string(),
                    config.stellar_core_url.clone(),
                );
                data.insert("INGEST".to_string(), config.enable_ingest.to_string());
            }
        }
        NodeType::SorobanRpc => {
            if let Some(config) = &node.spec.soroban_config {
                data.insert(
                    "STELLAR_CORE_URL".to_string(),
                    config.stellar_core_url.clone(),
                );

                // Try to generate TOML from structured config (preferred)
                if config.captive_core_structured_config.is_some() {
                    match crate::controller::captive_core::CaptiveCoreConfigBuilder::from_node_config(node) {
                        Ok(builder) => {
                            match builder.build_toml() {
                                Ok(toml) => {
                                    data.insert("captive-core.cfg".to_string(), toml);
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to build Captive Core TOML: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to create Captive Core config builder: {}", e);
                        }
                    }
                } else {
                    // Fallback to deprecated raw TOML for backward compatibility
                    #[allow(deprecated)]
                    if let Some(captive_config) = &config.captive_core_config {
                        data.insert("captive-core.cfg".to_string(), captive_config.clone());
                    }
                }
            }
        }
    }

    let annotations = node.spec.storage.annotations.clone().unwrap_or_default();

    ConfigMap {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name.clone()),
                namespace: node.namespace(),
                labels: Some(labels.clone()),
                annotations: if annotations.is_empty() {
                    None
                } else {
                    Some(annotations.clone()) // <-- use the extracted annotations
                },
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None, // no extra ObjectMeta
        ),
        data: Some(data.clone()),
        ..Default::default()
    }
}

/// Delete the ConfigMap for a node
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_config_map(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "config");

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted ConfigMap {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            warn!("ConfigMap {} not found", name);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

// ============================================================================
// Deployment (for Horizon and Soroban RPC)
// ============================================================================

/// Ensure a Deployment exists for RPC nodes
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_deployment(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    let deployment = build_deployment(node, enable_mtls);

    let patch = Patch::Apply(&deployment);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    Ok(())
}

/// Ensure a canary Deployment exists if needed
pub async fn ensure_canary_deployment(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
) -> Result<()> {
    let canary_version = match node
        .status
        .as_ref()
        .and_then(|status| status.canary_version.as_ref())
    {
        Some(v) => v,
        None => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let name = format!("{}-canary", node.name_any());

    let mut canary_node = node.clone();
    canary_node.spec.version = canary_version.clone();

    let mut deployment = build_deployment(&canary_node, enable_mtls);
    deployment.metadata.name = Some(name.clone());

    // Update labels and selectors to include canary label
    if let Some(spec) = &mut deployment.spec {
        let mut labels = spec
            .template
            .metadata
            .as_ref()
            .and_then(|m| m.labels.clone())
            .unwrap_or_default();
        labels.insert("stellar.org/rollout-type".to_string(), "canary".to_string());
        spec.template.metadata.as_mut().unwrap().labels = Some(labels.clone());
        spec.selector.match_labels = Some(labels.clone());

        let meta = &mut deployment.metadata;
        let mut meta_labels = meta.labels.clone().unwrap_or_default();
        meta_labels.insert("stellar.org/rollout-type".to_string(), "canary".to_string());
        meta.labels = Some(meta_labels);
    }

    let patch = Patch::Apply(&deployment);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    Ok(())
}

fn build_deployment(node: &StellarNode, enable_mtls: bool) -> Deployment {
    let labels = standard_labels(node);
    let name = node.name_any();

    let replicas = if node.spec.suspended {
        0
    } else {
        node.spec.replicas
    };

    Deployment {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name.clone()),
                namespace: node.namespace(),
                labels: Some(labels.clone()),
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None,
        ),
        spec: Some(DeploymentSpec {
            replicas: Some(replicas),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            template: build_pod_template(node, &labels, enable_mtls),
            ..Default::default()
        }),
        status: None,
    }
}

// ============================================================================
// StatefulSet (for Validators)
// ============================================================================

/// Ensure a StatefulSet exists for Validator nodes
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_statefulset(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    let statefulset = build_statefulset(node, enable_mtls);

    let patch = Patch::Apply(&statefulset);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    Ok(())
}

fn build_statefulset(node: &StellarNode, enable_mtls: bool) -> StatefulSet {
    let labels = standard_labels(node);
    let name = node.name_any();

    let replicas = if node.spec.suspended { 0 } else { 1 }; // Validators always have 1 replica

    let mut annotations = BTreeMap::new();
    if node.spec.suspended {
        annotations.insert("stellar.org/suspended".to_string(), "true".to_string());
        annotations.insert(
            "stellar.org/suspended-at".to_string(),
            chrono::Utc::now().to_rfc3339(),
        );
    }

    let annotations = node.spec.storage.annotations.clone().unwrap_or_default();

    StatefulSet {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name.clone()),
                namespace: node.namespace(),
                labels: Some(labels.clone()),
                annotations: if annotations.is_empty() {
                    None
                } else {
                    Some(annotations)
                },
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None,
        ),
        spec: Some(StatefulSetSpec {
            replicas: Some(replicas),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            service_name: format!("{name}-headless"),
            template: build_pod_template(node, &labels, enable_mtls),
            ..Default::default()
        }),
        status: None,
    }
}

/// Delete the workload (Deployment or StatefulSet) for a node
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_workload(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    match node.spec.node_type {
        NodeType::Validator => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
            match api.delete(&name, &DeleteParams::default()).await {
                Ok(_) => info!("Deleted StatefulSet {}", name),
                Err(kube::Error::Api(e)) if e.code == 404 => {
                    warn!("StatefulSet {} not found", name);
                }
                Err(e) => return Err(Error::KubeError(e)),
            }
        }
        _ => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
            match api.delete(&name, &DeleteParams::default()).await {
                Ok(_) => info!("Deleted Deployment {}", name),
                Err(kube::Error::Api(e)) if e.code == 404 => {
                    warn!("Deployment {} not found", name);
                }
                Err(e) => return Err(Error::KubeError(e)),
            }
        }
    }

    Ok(())
}

// ============================================================================
// Service
// ============================================================================

/// Ensure a Service exists for the node
/// Service persists even when node is suspended to maintain peer discovery
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_service(client: &Client, node: &StellarNode, enable_mtls: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    let service = build_service(node, enable_mtls);

    let patch = Patch::Apply(&service);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    Ok(())
}

/// Ensure a canary Service exists if needed
pub async fn ensure_canary_service(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
) -> Result<()> {
    if node
        .status
        .as_ref()
        .and_then(|status| status.canary_version.as_ref())
        .is_none()
    {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = format!("{}-canary", node.name_any());

    let mut service = build_service(node, enable_mtls);
    service.metadata.name = Some(name.clone());

    if let Some(spec) = &mut service.spec {
        let mut selector = spec.selector.clone().unwrap_or_default();
        selector.insert("stellar.org/rollout-type".to_string(), "canary".to_string());
        spec.selector = Some(selector);

        let meta = &mut service.metadata;
        let mut labels = meta.labels.clone().unwrap_or_default();
        labels.insert("stellar.org/rollout-type".to_string(), "canary".to_string());
        meta.labels = Some(labels);
    }

    let patch = Patch::Apply(&service);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    Ok(())
}

fn build_service(node: &StellarNode, enable_mtls: bool) -> Service {
    let labels = standard_labels(node);
    let name = node.name_any();

    let http_port_name = if enable_mtls { "https" } else { "http" }.to_string();

    let ports = match node.spec.node_type {
        NodeType::Validator => vec![
            ServicePort {
                name: Some("peer".to_string()),
                port: 11625,
                ..Default::default()
            },
            ServicePort {
                name: Some(http_port_name),
                port: 11626,
                ..Default::default()
            },
        ],
        NodeType::Horizon => vec![ServicePort {
            name: Some(http_port_name),
            port: 8000,
            ..Default::default()
        }],
        NodeType::SorobanRpc => vec![ServicePort {
            name: Some(http_port_name),
            port: 8000,
            ..Default::default()
        }],
    };

    Service {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: node.namespace(),
                labels: Some(labels.clone()),
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None,
        ),
        spec: Some(ServiceSpec {
            selector: Some(labels),
            ports: Some(ports),
            ..Default::default()
        }),
        status: None,
    }
}

// ============================================================================
// LoadBalancer Service (MetalLB Integration)
// ============================================================================

/// Ensure a LoadBalancer Service exists for external access via MetalLB
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_load_balancer_service(client: &Client, node: &StellarNode) -> Result<()> {
    let lb_cfg = match &node.spec.load_balancer {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "lb");

    let service = build_load_balancer_service(node, lb_cfg);

    let patch = Patch::Apply(&service);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    info!("LoadBalancer Service ensured for {}/{}", namespace, name);
    Ok(())
}

fn build_load_balancer_service(node: &StellarNode, config: &LoadBalancerConfig) -> Service {
    let labels = standard_labels(node);
    let name = resource_name(node, "lb");

    // Build service ports based on node type
    let mut ports = match node.spec.node_type {
        NodeType::Validator => vec![
            ServicePort {
                name: Some("peer".to_string()),
                port: 11625,
                target_port: Some(
                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11625),
                ),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
            ServicePort {
                name: Some("http".to_string()),
                port: 11626,
                target_port: Some(
                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11626),
                ),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
        ],
        NodeType::Horizon | NodeType::SorobanRpc => vec![ServicePort {
            name: Some("http".to_string()),
            port: 8000,
            target_port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8000)),
            protocol: Some("TCP".to_string()),
            ..Default::default()
        }],
    };

    // Add health check port if enabled
    if config.health_check_enabled {
        ports.push(ServicePort {
            name: Some("health".to_string()),
            port: config.health_check_port,
            target_port: Some(
                k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(
                    config.health_check_port,
                ),
            ),
            protocol: Some("TCP".to_string()),
            ..Default::default()
        });
    }

    // Build annotations for MetalLB
    let mut annotations = config.annotations.clone().unwrap_or_default();

    // Add MetalLB address pool annotation if specified
    if let Some(pool) = &config.address_pool {
        annotations.insert("metallb.universe.tf/address-pool".to_string(), pool.clone());
    }

    // Add MetalLB load balancer sharing annotation for anycast
    annotations.insert(
        "metallb.universe.tf/allow-shared-ip".to_string(),
        format!("stellar-{}", node.name_any()),
    );

    // Add BGP-specific annotations
    if config.mode == LoadBalancerMode::BGP {
        if let Some(bgp) = &config.bgp {
            // Add BGP communities annotation
            if !bgp.communities.is_empty() {
                annotations.insert(
                    "metallb.universe.tf/bgp-peer-communities".to_string(),
                    bgp.communities.join(","),
                );
            }

            // Add large communities
            if !bgp.large_communities.is_empty() {
                annotations.insert(
                    "metallb.universe.tf/bgp-large-communities".to_string(),
                    bgp.large_communities.join(","),
                );
            }
        }
    }

    // Add global discovery annotations
    // TODO: global_discovery field not yet implemented in StellarNodeSpec
    /*
    if let Some(gd) = &node.spec.global_discovery {
        if gd.enabled {
            if let Some(region) = &gd.region {
                annotations.insert("stellar.org/region".to_string(), region.clone());
            }
            if let Some(zone) = &gd.zone {
                annotations.insert("stellar.org/zone".to_string(), zone.clone());
            }
            annotations.insert("stellar.org/priority".to_string(), gd.priority.to_string());

            // Add topology aware hints annotation
            if gd.topology_aware_hints {
                annotations.insert(
                    "service.kubernetes.io/topology-mode".to_string(),
                    "Auto".to_string(),
                );
            }

            // Add external-dns annotations if configured
            if let Some(dns) = &gd.external_dns {
                annotations.insert(
                    "external-dns.alpha.kubernetes.io/hostname".to_string(),
                    dns.hostname.clone(),
                );
                annotations.insert(
                    "external-dns.alpha.kubernetes.io/ttl".to_string(),
                    dns.ttl.to_string(),
                );
                if let Some(extra) = &dns.annotations {
                    annotations.extend(extra.clone());
                }
            }
        }
    }
    */

    let external_traffic_policy = match config.external_traffic_policy {
        ExternalTrafficPolicy::Cluster => "Cluster".to_string(),
        ExternalTrafficPolicy::Local => "Local".to_string(),
    };

    let annotations = node.spec.storage.annotations.clone().unwrap_or_default();

    Service {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: node.namespace(),
                labels: Some(labels.clone()),
                annotations: if annotations.is_empty() {
                    None
                } else {
                    Some(annotations)
                },
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None,
        ),
        spec: Some(ServiceSpec {
            type_: Some("LoadBalancer".to_string()),
            selector: Some(labels),
            ports: Some(ports),
            load_balancer_ip: config.load_balancer_ip.clone(),
            external_traffic_policy: Some(external_traffic_policy),
            ..Default::default()
        }),
        status: None,
    }
}

/// Delete the LoadBalancer Service for a node
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_load_balancer_service(client: &Client, node: &StellarNode) -> Result<()> {
    if node.spec.load_balancer.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "lb");

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted LoadBalancer Service {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            warn!("LoadBalancer Service {} not found, already deleted", name);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

// ============================================================================
// MetalLB BGP Configuration Resources
// ============================================================================

/// Ensure MetalLB BGPAdvertisement and IPAddressPool ConfigMaps are documented
/// Note: MetalLB CRDs must be created manually or via Helm; this function
/// creates the recommended ConfigMap for cluster operators to reference.
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_metallb_config(client: &Client, node: &StellarNode) -> Result<()> {
    let lb_cfg = match &node.spec.load_balancer {
        Some(cfg) if cfg.enabled && cfg.mode == LoadBalancerMode::BGP => cfg,
        _ => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "metallb-config");

    let config = build_metallb_config_map(node, lb_cfg);

    let patch = Patch::Apply(&config);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    info!(
        "MetalLB configuration ConfigMap ensured for {}/{}",
        namespace, name
    );
    Ok(())
}

fn build_metallb_config_map(node: &StellarNode, config: &LoadBalancerConfig) -> ConfigMap {
    let labels = standard_labels(node);
    let name = resource_name(node, "metallb-config");

    let bgp = config.bgp.as_ref();
    let mut data = BTreeMap::new();

    // Generate IPAddressPool YAML
    let address_pool_yaml = if let Some(ip) = &config.load_balancer_ip {
        format!(
            r#"# IPAddressPool for {}
# Apply this to metallb-system namespace
apiVersion: metallb.io/v1beta1
kind: IPAddressPool
metadata:
  name: stellar-{}-pool
  namespace: metallb-system
spec:
  addresses:
    - {}/32
  autoAssign: true
"#,
            node.name_any(),
            node.name_any(),
            ip
        )
    } else if let Some(pool) = &config.address_pool {
        format!(
            r#"# Using existing IPAddressPool: {pool}
# Ensure this pool exists in metallb-system namespace
"#
        )
    } else {
        "# No specific IP or pool configured; MetalLB will use default pool\n".to_string()
    };

    data.insert("ip-address-pool.yaml".to_string(), address_pool_yaml);

    // Generate BGPAdvertisement YAML
    if let Some(bgp_cfg) = bgp {
        let communities_yaml = if !bgp_cfg.communities.is_empty() {
            format!(
                "  communities:\n{}",
                bgp_cfg
                    .communities
                    .iter()
                    .map(|c| format!("    - {c}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        } else {
            String::new()
        };

        let node_selectors_yaml = if let Some(adv) = &bgp_cfg.advertisement {
            if let Some(selectors) = &adv.node_selectors {
                format!(
                    "  nodeSelectors:\n    - matchLabels:\n{}",
                    selectors
                        .iter()
                        .map(|(k, v)| format!("        {k}: \"{v}\""))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let aggregation = bgp_cfg
            .advertisement
            .as_ref()
            .map(|a| a.aggregation_length)
            .unwrap_or(32);

        let bgp_advertisement_yaml = format!(
            r#"# BGPAdvertisement for {}
# Apply this to metallb-system namespace
apiVersion: metallb.io/v1beta1
kind: BGPAdvertisement
metadata:
  name: stellar-{}-bgp
  namespace: metallb-system
spec:
  ipAddressPools:
    - stellar-{}-pool
  aggregationLength: {}
{}{}
"#,
            node.name_any(),
            node.name_any(),
            node.name_any(),
            aggregation,
            communities_yaml,
            node_selectors_yaml
        );

        data.insert("bgp-advertisement.yaml".to_string(), bgp_advertisement_yaml);

        // Generate BGPPeer YAML for each peer
        let mut peers_yaml = String::new();
        for (i, peer) in bgp_cfg.peers.iter().enumerate() {
            let password_ref = peer.password_secret_ref.as_ref().map(|s| {
                format!(
                    r#"  password:
    secretRef:
      name: {}
      key: {}"#,
                    s.name, s.key
                )
            });

            let bfd_profile = if bgp_cfg.bfd_enabled {
                bgp_cfg
                    .bfd_profile
                    .as_ref()
                    .map(|p| format!("  bfdProfile: {p}"))
                    .unwrap_or_default()
            } else {
                String::new()
            };

            use std::fmt::Write;
            write!(
                peers_yaml,
                r#"---
# BGPPeer {} for {}
apiVersion: metallb.io/v1beta2
kind: BGPPeer
metadata:
  name: stellar-{}-peer-{}
  namespace: metallb-system
spec:
  myASN: {}
  peerASN: {}
  peerAddress: {}
  peerPort: {}
  holdTime: {}s
  keepaliveTime: {}s
  ebgpMultiHop: {}
{}{}
"#,
                i + 1,
                node.name_any(),
                node.name_any(),
                i,
                bgp_cfg.local_asn,
                peer.asn,
                peer.address,
                peer.port,
                peer.hold_time,
                peer.keepalive_time,
                peer.ebgp_multi_hop,
                password_ref.unwrap_or_default(),
                bfd_profile
            )
            .unwrap();
        }

        data.insert("bgp-peers.yaml".to_string(), peers_yaml);
    }

    // Generate L2Advertisement YAML as fallback
    if config.mode == LoadBalancerMode::L2 || config.mode == LoadBalancerMode::BGP {
        let l2_advertisement_yaml = format!(
            r#"# L2Advertisement for {} (optional fallback)
# Apply this to metallb-system namespace
apiVersion: metallb.io/v1beta1
kind: L2Advertisement
metadata:
  name: stellar-{}-l2
  namespace: metallb-system
spec:
  ipAddressPools:
    - stellar-{}-pool
"#,
            node.name_any(),
            node.name_any(),
            node.name_any()
        );

        data.insert("l2-advertisement.yaml".to_string(), l2_advertisement_yaml);
    }

    // Add usage instructions
    let instructions = format!(
        r#"# MetalLB Configuration for Stellar Node: {}
#
# This ConfigMap contains the recommended MetalLB resources for BGP anycast.
# Apply these manifests to enable global node discovery.
#
# Prerequisites:
# 1. MetalLB must be installed in the cluster (metallb-system namespace)
# 2. BGP peers must be configured to accept connections from cluster nodes
# 3. Firewall rules must allow BGP traffic (TCP port 179)
#
# Installation:
#   kubectl apply -f ip-address-pool.yaml -n metallb-system
#   kubectl apply -f bgp-advertisement.yaml -n metallb-system
#   kubectl apply -f bgp-peers.yaml -n metallb-system
#
# Verification:
#   kubectl get ipaddresspools -n metallb-system
#   kubectl get bgpadvertisements -n metallb-system
#   kubectl get bgppeers -n metallb-system
#   kubectl logs -n metallb-system -l app=metallb,component=speaker
#
# For more information, see: https://metallb.universe.tf/configuration/
"#,
        node.name_any()
    );

    data.insert("README.md".to_string(), instructions);

    let annotations = node.spec.storage.annotations.clone().unwrap_or_default();
    ConfigMap {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name.clone()),
                namespace: node.namespace(),
                labels: Some(labels.clone()),
                annotations: if annotations.is_empty() {
                    None
                } else {
                    Some(annotations.clone()) // <-- use the extracted annotations
                },
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None, // no extra ObjectMeta
        ),
        data: Some(data.clone()),
        ..Default::default()
    }
}

/// Delete the MetalLB configuration ConfigMap
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_metallb_config(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "metallb-config");

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted MetalLB ConfigMap {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            info!("MetalLB ConfigMap {} not found, skipping delete", name);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

/// Delete the Service for a node
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_service(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted Service {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            warn!("Service {} not found", name);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

// ============================================================================
// CloudNativePG (CNPG) Resources
// ============================================================================

/// Ensure a CNPG Cluster exists for managed HA database
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_cnpg_cluster(client: &Client, node: &StellarNode) -> Result<()> {
    let managed_db = match &node.spec.managed_database {
        Some(cfg) => cfg,
        None => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Cluster> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    let cluster = build_cnpg_cluster(node, managed_db);

    let patch = Patch::Apply(&cluster);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    info!("CNPG Cluster ensured for {}/{}", namespace, name);
    Ok(())
}

fn build_cnpg_cluster(node: &StellarNode, config: &ManagedDatabaseConfig) -> Cluster {
    let mut labels = standard_labels(node);
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "cnpg".to_string(),
    );
    let name = node.name_any();

    let mut cluster = Cluster {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: node.namespace(),
            labels: Some(labels),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: ClusterSpec {
            instances: config.instances,
            image_name: None,
            postgresql: Some(PostgresConfiguration {
                parameters: {
                    let mut p = BTreeMap::new();
                    p.insert("max_connections".to_string(), "100".to_string());
                    p.insert("shared_buffers".to_string(), "256MB".to_string());
                    p
                },
            }),
            storage: StorageConfiguration {
                size: config.storage.size.clone(),
                storage_class: Some(config.storage.storage_class.clone()),
            },
            backup: config.backup.as_ref().map(|b| BackupConfiguration {
                barman_object_store: Some(BarmanObjectStore {
                    destination_path: b.destination_path.clone(),
                    endpoint_u_r_l: None,
                    s3_credentials: Some(S3Credentials {
                        access_key_id: CnpgSecretKeySelector {
                            name: b.credentials_secret_ref.clone(),
                            key: "AWS_ACCESS_KEY_ID".to_string(),
                        },
                        secret_access_key: CnpgSecretKeySelector {
                            name: b.credentials_secret_ref.clone(),
                            key: "AWS_SECRET_ACCESS_KEY".to_string(),
                        },
                    }),
                    azure_credentials: None,
                    google_credentials: None,
                    wal: Some(WalBackupConfiguration {
                        compression: Some("gzip".to_string()),
                    }),
                }),
                retention_policy: Some(b.retention_policy.clone()),
            }),
            bootstrap: Some(BootstrapConfiguration {
                initdb: Some(InitDbConfiguration {
                    database: "stellar".to_string(),
                    owner: "stellar".to_string(),
                    secret: None,
                }),
            }),
            monitoring: Some(MonitoringConfiguration {
                enable_pod_monitor: true,
            }),
        },
    };

    // Override image if needed (Postgres version)
    if !config.postgres_version.is_empty() {
        cluster.spec.image_name = Some(format!(
            "ghcr.io/cloudnative-pg/postgresql:{}",
            config.postgres_version
        ));
    }

    cluster
}

/// Ensure a CNPG Pooler exists for managed database pooling
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_cnpg_pooler(client: &Client, node: &StellarNode) -> Result<()> {
    let managed_db = match &node.spec.managed_database {
        Some(cfg) => cfg,
        None => return Ok(()),
    };

    let pgbouncer = match &managed_db.pooling {
        Some(p) if p.enabled => p,
        _ => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Pooler> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "pooler");

    let pooler = build_cnpg_pooler(node, pgbouncer);

    let patch = Patch::Apply(&pooler);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    info!("CNPG Pooler ensured for {}/{}", namespace, name);
    Ok(())
}

fn build_cnpg_pooler(node: &StellarNode, config: &crate::crd::PgBouncerConfig) -> Pooler {
    let mut labels = standard_labels(node);
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        "pooler".to_string(),
    );
    let name = resource_name(node, "pooler");

    Pooler {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: node.namespace(),
            labels: Some(labels),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: PoolerSpec {
            cluster: PoolerCluster {
                name: node.name_any(),
            },
            instances: config.replicas,
            type_: "pgbouncer".to_string(),
            pgbouncer: PgBouncerSpec {
                pool_mode: match config.pool_mode {
                    crate::crd::PgBouncerPoolMode::Session => "session".to_string(),
                    crate::crd::PgBouncerPoolMode::Transaction => "transaction".to_string(),
                    crate::crd::PgBouncerPoolMode::Statement => "statement".to_string(),
                },
                parameters: {
                    let mut p = BTreeMap::new();
                    p.insert(
                        "max_client_conn".to_string(),
                        config.max_client_conn.to_string(),
                    );
                    p.insert(
                        "default_pool_size".to_string(),
                        config.default_pool_size.to_string(),
                    );
                    p
                },
            },
            monitoring: Some(MonitoringConfiguration {
                enable_pod_monitor: true,
            }),
        },
    }
}

/// Delete the CNPG resources for a node
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_cnpg_resources(client: &Client, node: &StellarNode) -> Result<()> {
    if node.spec.managed_database.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    // Delete Pooler
    let pooler_api: Api<Pooler> = Api::namespaced(client.clone(), &namespace);
    let pooler_name = resource_name(node, "pooler");
    let _ = pooler_api
        .delete(&pooler_name, &DeleteParams::default())
        .await;

    // Delete Cluster
    let cluster_api: Api<Cluster> = Api::namespaced(client.clone(), &namespace);
    let cluster_name = node.name_any();
    let _ = cluster_api
        .delete(&cluster_name, &DeleteParams::default())
        .await;

    Ok(())
}

// ============================================================================
// Ingress
// ============================================================================

/// Ensure an Ingress exists for Horizon or Soroban RPC nodes when configured
pub async fn ensure_ingress(client: &Client, node: &StellarNode) -> Result<()> {
    // Ingress is only supported for Horizon and SorobanRpc
    let ingress_cfg = match &node.spec.ingress {
        Some(cfg)
            if matches!(
                node.spec.node_type,
                NodeType::Horizon | NodeType::SorobanRpc
            ) =>
        {
            cfg
        }
        _ => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "ingress");

    let ingress = build_ingress(node, ingress_cfg);

    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &Patch::Apply(&ingress),
    )
    .await?;

    info!("Ingress ensured for {}/{}", namespace, name);

    // If canary is active, ensure canary ingress as well
    if let RolloutStrategy::Canary(ref cfg) = node.spec.strategy {
        if node
            .status
            .as_ref()
            .and_then(|status| status.canary_version.as_ref())
            .is_some()
        {
            let canary_name = format!("{name}-canary");
            let mut canary_ingress = build_ingress(node, ingress_cfg);
            canary_ingress.metadata.name = Some(canary_name.clone());

            // Add canary annotations
            let mut annotations = canary_ingress
                .metadata
                .annotations
                .clone()
                .unwrap_or_default();
            annotations.insert(
                "nginx.ingress.kubernetes.io/canary".to_string(),
                "true".to_string(),
            );
            annotations.insert(
                "nginx.ingress.kubernetes.io/canary-weight".to_string(),
                cfg.weight.to_string(),
            );

            // Support for Traefik and Istio (Istio usually needs VirtualService, but some setups use Ingress annotations)
            annotations.insert(
                "traefik.ingress.kubernetes.io/service.weights".to_string(),
                format!("{}:{}", node.name_any(), cfg.weight),
            );

            canary_ingress.metadata.annotations = Some(annotations);

            // Update backend to point to canary service
            if let Some(spec) = &mut canary_ingress.spec {
                if let Some(rules) = &mut spec.rules {
                    for rule in rules {
                        if let Some(http) = &mut rule.http {
                            for path in &mut http.paths {
                                if let Some(backend) = &mut path.backend.service {
                                    backend.name = format!("{}-canary", node.name_any());
                                }
                            }
                        }
                    }
                }
            }

            api.patch(
                &canary_name,
                &PatchParams::apply("stellar-operator").force(),
                &Patch::Apply(&canary_ingress),
            )
            .await?;
            info!("Canary Ingress ensured for {}/{}", namespace, canary_name);
        } else {
            // Delete canary ingress if no longer active
            let canary_name = format!("{name}-canary");
            let _ = api.delete(&canary_name, &DeleteParams::default()).await;
        }
    }

    Ok(())
}

fn build_ingress(node: &StellarNode, config: &IngressConfig) -> Ingress {
    let labels = standard_labels(node);
    let name = resource_name(node, "ingress");

    let service_port = match node.spec.node_type {
        NodeType::Horizon | NodeType::SorobanRpc => 8000,
        NodeType::Validator => 11626,
    };

    // Merge user-provided annotations with cert-manager issuer hints
    let mut annotations = config.annotations.clone().unwrap_or_default();
    if let Some(issuer) = &config.cert_manager_issuer {
        annotations.insert("cert-manager.io/issuer".to_string(), issuer.clone());
    }
    if let Some(cluster_issuer) = &config.cert_manager_cluster_issuer {
        annotations.insert(
            "cert-manager.io/cluster-issuer".to_string(),
            cluster_issuer.clone(),
        );
    }

    let rules: Vec<IngressRule> = config
        .hosts
        .iter()
        .map(|host| IngressRule {
            host: Some(host.host.clone()),
            http: Some(HTTPIngressRuleValue {
                paths: host
                    .paths
                    .iter()
                    .map(|p| HTTPIngressPath {
                        path: Some(p.path.clone()),
                        path_type: p.path_type.clone().unwrap_or_else(|| "Prefix".to_string()),
                        backend: IngressBackend {
                            service: Some(IngressServiceBackend {
                                name: node.name_any(),
                                port: Some(ServiceBackendPort {
                                    number: Some(service_port),
                                    name: None,
                                }),
                            }),
                            ..Default::default()
                        },
                    })
                    .collect(),
            }),
        })
        .collect();

    let tls = config.tls_secret_name.as_ref().map(|secret| {
        vec![IngressTLS {
            hosts: Some(config.hosts.iter().map(|h| h.host.clone()).collect()),
            secret_name: Some(secret.clone()),
        }]
    });

    let annotations = node.spec.storage.annotations.clone().unwrap_or_default();

    Ingress {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: node.namespace(),
                labels: Some(labels),
                annotations: if annotations.is_empty() {
                    None
                } else {
                    Some(annotations)
                },
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &node.spec.resource_meta,
        ),
        spec: Some(IngressSpec {
            ingress_class_name: config.class_name.clone(),
            rules: Some(rules),
            tls,
            ..Default::default()
        }),
        status: None,
    }
}

/// Delete the Ingress for a node
pub async fn delete_ingress(client: &Client, node: &StellarNode) -> Result<()> {
    if node.spec.ingress.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "ingress");

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted Ingress {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            warn!("Ingress {} not found, already deleted", name);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

// ============================================================================
// Pod Template Builder
// ============================================================================

fn build_pod_template(
    node: &StellarNode,
    labels: &BTreeMap<String, String>,
    enable_mtls: bool,
) -> PodTemplateSpec {
    let mut pod_spec = PodSpec {
        containers: vec![build_container(node, enable_mtls)],
        volumes: Some(vec![
            Volume {
                name: "data".to_string(),
                persistent_volume_claim: Some(
                    k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                        claim_name: resource_name(node, "data"),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            },
            Volume {
                name: "config".to_string(),
                config_map: Some(k8s_openapi::api::core::v1::ConfigMapVolumeSource {
                    name: Some(resource_name(node, "config")),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]),
        topology_spread_constraints: node.spec.topology_spread_constraints.clone(),
        ..Default::default()
    };

    // Add Horizon database migration init container
    if let NodeType::Horizon = node.spec.node_type {
        if let Some(horizon_config) = &node.spec.horizon_config {
            if horizon_config.auto_migration {
                let init_containers = pod_spec.init_containers.get_or_insert_with(Vec::new);
                init_containers.push(build_horizon_migration_container(node));
            }
        }
    }

    // Add KMS init container if needed (Validator nodes only)
    if let NodeType::Validator = node.spec.node_type {
        if let Some(validator_config) = &node.spec.validator_config {
            if validator_config.key_source == KeySource::KMS {
                if let Some(kms_config) = &validator_config.kms_config {
                    // Add shared memory volume for keys (never touches disk)
                    let volumes = pod_spec.volumes.get_or_insert_with(Vec::new);
                    volumes.push(Volume {
                        name: "keys".to_string(),
                        empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
                            medium: Some("Memory".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });

                    // Add KMS fetcher init container
                    let init_containers = pod_spec.init_containers.get_or_insert_with(Vec::new);
                    init_containers.push(Container {
                        name: "kms-fetcher".to_string(),
                        image: Some(
                            kms_config
                                .fetcher_image
                                .clone()
                                .unwrap_or_else(|| "stellar/kms-fetcher:latest".to_string()),
                        ),
                        env: Some(vec![
                            EnvVar {
                                name: "KMS_KEY_ID".to_string(),
                                value: Some(kms_config.key_id.clone()),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KMS_PROVIDER".to_string(),
                                value: Some(kms_config.provider.clone()),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KMS_REGION".to_string(),
                                value: kms_config.region.clone(),
                                ..Default::default()
                            },
                            EnvVar {
                                name: "KEY_OUTPUT_PATH".to_string(),
                                value: Some("/keys/validator-seed".to_string()),
                                ..Default::default()
                            },
                        ]),
                        volume_mounts: Some(vec![VolumeMount {
                            name: "keys".to_string(),
                            mount_path: "/keys".to_string(),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Add mTLS certificate volume
    let volumes = pod_spec.volumes.get_or_insert_with(Vec::new);
    volumes.push(Volume {
        name: "tls".to_string(),
        secret: Some(k8s_openapi::api::core::v1::SecretVolumeSource {
            secret_name: Some(format!("{}-client-cert", node.name_any())),
            ..Default::default()
        }),
        ..Default::default()
    });

    // Add Cloud HSM sidecar and volumes
    if let NodeType::Validator = node.spec.node_type {
        if let Some(validator_config) = &node.spec.validator_config {
            if let Some(hsm_config) = &validator_config.hsm_config {
                // If using AWS CloudHSM, add sidecar and shared volume
                if hsm_config.provider == HsmProvider::AWS {
                    // Shared socket volume
                    volumes.push(Volume {
                        name: "cloudhsm-socket".to_string(),
                        empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
                            medium: Some("Memory".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });

                    // Sidecar container running CloudHSM client daemon
                    let containers = &mut pod_spec.containers;
                    containers.push(Container {
                        name: "cloudhsm-client".to_string(),
                        // Use a standard image or user-provided one.
                        // In reality, user would likely build this or we'd default to a known working one.
                        // For now, using a placeholder that needs to be valid.
                        image: Some("amazon/cloudhsm-client:latest".to_string()),
                        command: Some(vec!["/opt/cloudhsm/bin/cloudhsm_client".to_string()]),
                        args: Some(vec!["--foreground".to_string()]),
                        volume_mounts: Some(vec![VolumeMount {
                            name: "cloudhsm-socket".to_string(),
                            mount_path: "/var/run/cloudhsm".to_string(),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    });
                }
            }
        }
    }

    PodTemplateSpec {
        metadata: Some(merge_resource_meta(
            ObjectMeta {
                labels: Some(labels.clone()),
                ..Default::default()
            },
            &node.spec.resource_meta,
        )),
        spec: Some(pod_spec),
    }
}

fn build_container(node: &StellarNode, enable_mtls: bool) -> Container {
    let mut requests = BTreeMap::new();
    requests.insert(
        "cpu".to_string(),
        Quantity(node.spec.resources.requests.cpu.clone()),
    );
    requests.insert(
        "memory".to_string(),
        Quantity(node.spec.resources.requests.memory.clone()),
    );

    let mut limits = BTreeMap::new();
    limits.insert(
        "cpu".to_string(),
        Quantity(node.spec.resources.limits.cpu.clone()),
    );
    limits.insert(
        "memory".to_string(),
        Quantity(node.spec.resources.limits.memory.clone()),
    );

    let (container_port, data_mount_path, db_env_var_name) = match node.spec.node_type {
        NodeType::Validator => (11625, "/opt/stellar/data", "DATABASE"),
        NodeType::Horizon => (8000, "/data", "DATABASE_URL"),
        NodeType::SorobanRpc => (8000, "/data", "DATABASE_URL"),
    };

    // Build environment variables
    let mut env_vars = vec![EnvVar {
        name: "NETWORK_PASSPHRASE".to_string(),
        value: Some(node.spec.network.passphrase().to_string()),
        ..Default::default()
    }];

    // Source validator seed from Secret or shared RAM volume (KMS)
    if let NodeType::Validator = node.spec.node_type {
        if let Some(validator_config) = &node.spec.validator_config {
            match validator_config.key_source {
                KeySource::Secret => {
                    env_vars.push(EnvVar {
                        name: "STELLAR_CORE_SEED".to_string(),
                        value: None,
                        value_from: Some(EnvVarSource {
                            secret_key_ref: Some(SecretKeySelector {
                                name: Some(validator_config.seed_secret_ref.clone()),
                                key: "STELLAR_CORE_SEED".to_string(),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                    });
                }
                KeySource::KMS => {
                    // Seed will be read from /keys/validator-seed file provided by init container
                    env_vars.push(EnvVar {
                        name: "STELLAR_CORE_SEED_PATH".to_string(),
                        value: Some("/keys/validator-seed".to_string()),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Add database environment variable from secret if external database is configured
    if let Some(db_config) = &node.spec.database {
        env_vars.push(EnvVar {
            name: db_env_var_name.to_string(),
            value: None,
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: Some(db_config.secret_key_ref.name.clone()),
                    key: db_config.secret_key_ref.key.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        });
    }

    // Add database environment variable from CNPG secret if managed database is configured
    if let Some(_managed_db) = &node.spec.managed_database {
        let secret_name = node.name_any(); // CNPG cluster name matches StellarNode name
        env_vars.push(EnvVar {
            name: db_env_var_name.to_string(),
            value: None,
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: Some(format!("{secret_name}-app")),
                    key: "uri".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        });
    }

    // Add TLS environment variables if mTLS is enabled
    if enable_mtls {
        match node.spec.node_type {
            NodeType::Horizon | NodeType::SorobanRpc => {
                env_vars.push(EnvVar {
                    name: "TLS_CERT_FILE".to_string(),
                    value: Some("/etc/stellar/tls/tls.crt".to_string()),
                    ..Default::default()
                });
                env_vars.push(EnvVar {
                    name: "TLS_KEY_FILE".to_string(),
                    value: Some("/etc/stellar/tls/tls.key".to_string()),
                    ..Default::default()
                });
                env_vars.push(EnvVar {
                    name: "CA_CERT_FILE".to_string(),
                    value: Some("/etc/stellar/tls/ca.crt".to_string()),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }

    // Add HSM environment variables and mounts
    let mut extra_volume_mounts = Vec::new();
    if let NodeType::Validator = node.spec.node_type {
        if let Some(validator_config) = &node.spec.validator_config {
            if let Some(hsm_config) = &validator_config.hsm_config {
                // PKCS#11 Module Path
                env_vars.push(EnvVar {
                    name: "PKCS11_MODULE_PATH".to_string(),
                    value: Some(hsm_config.pkcs11_lib_path.clone()),
                    ..Default::default()
                });

                // HSM IP (for Azure/Network)
                if let Some(ip) = &hsm_config.hsm_ip {
                    env_vars.push(EnvVar {
                        name: "HSM_IP_ADDRESS".to_string(),
                        value: Some(ip.clone()),
                        ..Default::default()
                    });
                }

                // HSM Credentials (PIN/User)
                if let Some(secret_ref) = &hsm_config.hsm_credentials_secret_ref {
                    env_vars.push(EnvVar {
                        name: "HSM_PIN".to_string(),
                        value: None,
                        value_from: Some(EnvVarSource {
                            secret_key_ref: Some(SecretKeySelector {
                                name: Some(secret_ref.clone()),
                                key: "HSM_PIN".to_string(),
                                optional: Some(true),
                            }),
                            ..Default::default()
                        }),
                    });
                    env_vars.push(EnvVar {
                        name: "HSM_USER".to_string(),
                        value: None,
                        value_from: Some(EnvVarSource {
                            secret_key_ref: Some(SecretKeySelector {
                                name: Some(secret_ref.clone()),
                                key: "HSM_USER".to_string(),
                                optional: Some(true),
                            }),
                            ..Default::default()
                        }),
                    });
                }

                // Mount socket for AWS CloudHSM
                if hsm_config.provider == HsmProvider::AWS {
                    extra_volume_mounts.push(VolumeMount {
                        name: "cloudhsm-socket".to_string(),
                        mount_path: "/var/run/cloudhsm".to_string(),
                        ..Default::default()
                    });
                }
            }
        }
    }

    let mut volume_mounts = vec![
        VolumeMount {
            name: "data".to_string(),
            mount_path: data_mount_path.to_string(),
            ..Default::default()
        },
        VolumeMount {
            name: "config".to_string(),
            mount_path: "/config".to_string(),
            read_only: Some(true),
            ..Default::default()
        },
    ];

    // Mount keys volume if using KMS
    if node.spec.node_type == NodeType::Validator {
        if let Some(validator_config) = &node.spec.validator_config {
            if validator_config.key_source == KeySource::KMS {
                volume_mounts.push(VolumeMount {
                    name: "keys".to_string(),
                    mount_path: "/keys".to_string(),
                    read_only: Some(true),
                    ..Default::default()
                });
            }
        }
    }

    // Mount mTLS certificates
    volume_mounts.push(VolumeMount {
        name: "tls".to_string(),
        mount_path: "/etc/stellar/tls".to_string(),
        read_only: Some(true),
        ..Default::default()
    });

    // Add extra mounts (HSM)
    volume_mounts.extend(extra_volume_mounts);

    Container {
        name: "stellar-node".to_string(),
        image: Some(node.spec.container_image()),
        ports: Some(vec![ContainerPort {
            container_port,
            ..Default::default()
        }]),
        env: Some(env_vars),
        resources: Some(K8sResources {
            requests: Some(requests),
            limits: Some(limits),
            claims: None,
        }),
        volume_mounts: Some(volume_mounts),
        ..Default::default()
    }
}

/// Build the migration container for Horizon
fn build_horizon_migration_container(node: &StellarNode) -> Container {
    let mut container = build_container(node, false);
    container.name = "horizon-db-migration".to_string();
    // Use a shell to try upgrade then init if needed, ensuring the DB is ready
    container.command = Some(vec!["/bin/sh".to_string()]);
    container.args = Some(vec![
        "-c".to_string(),
        "horizon db upgrade || horizon db init".to_string(),
    ]);

    // Migration doesn't need ports or probes
    container.ports = None;
    container.liveness_probe = None;
    container.readiness_probe = None;
    container.startup_probe = None;
    container.lifecycle = None;

    // Use slightly less resources for migration if desired, but reusing main ones is safer
    container
}
// ============================================================================
// HorizontalPodAutoscaler
// ============================================================================

/// Ensure a HorizontalPodAutoscaler exists for RPC nodes with autoscaling enabled
pub async fn ensure_hpa(client: &Client, node: &StellarNode) -> Result<()> {
    // Only create HPA for Horizon and SorobanRpc nodes with autoscaling config
    if !matches!(
        node.spec.node_type,
        NodeType::Horizon | NodeType::SorobanRpc
    ) || node.spec.autoscaling.is_none()
    {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "hpa");

    let hpa = build_hpa(node)?;

    let patch = Patch::Apply(&hpa);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    info!("HPA ensured for {}/{}", namespace, name);
    Ok(())
}

// ============================================================================
// Alerting (ConfigMap with Prometheus Rules)
// ============================================================================

/// Ensure alerting resources exist for the node if enabled
pub async fn ensure_alerting(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "alerts");

    if !node.spec.alerting {
        return delete_alerting(client, node).await;
    }

    let labels = standard_labels(node);
    let mut data = BTreeMap::new();

    // Define standard alerting rules in Prometheus format
    let rules = format!(
        r#"groups:
- name: {instance}.rules
  rules:
  - alert: StellarNodeDown
    expr: up{{app_kubernetes_io_instance="{instance}"}} == 0
    for: 5m
    labels:
      severity: critical
    annotations:
      summary: "Stellar node {instance} is down"
      description: "The Stellar node {instance} has been down for more than 5 minutes."
  - alert: StellarNodeHighMemory
    expr: container_memory_usage_bytes{{pod=~"{instance}.*"}} / container_spec_memory_limit_bytes > 0.8
    for: 10m
    labels:
      severity: warning
    annotations:
      summary: "Stellar node {instance} high memory usage"
      description: "The Stellar node {instance} is using more than 80% of its memory limit."
  - alert: StellarNodeSyncIssue
    expr: stellar_core_sync_status{{app_kubernetes_io_instance="{instance}"}} != 1
    for: 15m
    labels:
      severity: warning
    annotations:
      summary: "Stellar node {instance} sync issue"
      description: "The Stellar node {instance} has not been in sync for more than 15 minutes."
"#,
        instance = node.name_any()
    );

    data.insert("alerts.yaml".to_string(), rules);

    let cm = ConfigMap {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name.clone()),
                namespace: Some(namespace.clone()),
                labels: Some(labels),
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &node.spec.resource_meta,
        ),
        data: Some(data),
        ..Default::default()
    };

    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let patch = Patch::Apply(&cm);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    info!(
        "Alerting ConfigMap {} ensured for {}/{}",
        name,
        namespace,
        node.name_any()
    );
    Ok(())
}

fn build_hpa(node: &StellarNode) -> Result<HorizontalPodAutoscaler> {
    let autoscaling = node
        .spec
        .autoscaling
        .as_ref()
        .ok_or_else(|| Error::ValidationError("Autoscaling config not found".to_string()))?;

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "hpa");
    let deployment_name = node.name_any();

    let mut metrics = Vec::new();

    // Add CPU utilization metric if configured
    if let Some(target_cpu) = autoscaling.target_cpu_utilization_percentage {
        metrics.push(MetricSpec {
            type_: "Resource".to_string(),
            resource: Some(k8s_openapi::api::autoscaling::v2::ResourceMetricSource {
                name: "cpu".to_string(),
                target: MetricTarget {
                    type_: "Utilization".to_string(),
                    average_utilization: Some(target_cpu),
                    ..Default::default()
                },
            }),
            ..Default::default()
        });
    }

    // Add custom metrics
    for metric_name in &autoscaling.custom_metrics {
        if metric_name == "ledger_ingestion_lag" {
            metrics.push(MetricSpec {
                type_: "Object".to_string(),
                object: Some(ObjectMetricSource {
                    described_object: CrossVersionObjectReference {
                        api_version: Some("stellar.org/v1alpha1".to_string()),
                        kind: "StellarNode".to_string(),
                        name: node.name_any(),
                    },
                    metric: MetricIdentifier {
                        name: "stellar_node_ingestion_lag".to_string(),
                        selector: None,
                    },
                    target: MetricTarget {
                        type_: "Value".to_string(),
                        value: Some(Quantity("5".to_string())), // Scale if lag > 5 ledgers
                        ..Default::default()
                    },
                }),
                ..Default::default()
            });
        }
        // Add more custom metrics mapping here (e.g., request throughput)
    }

    let behavior = autoscaling
        .behavior
        .as_ref()
        .map(|b| HorizontalPodAutoscalerBehavior {
            scale_up: b.scale_up.as_ref().map(|s| HPAScalingRules {
                stabilization_window_seconds: s.stabilization_window_seconds,
                policies: Some(
                    s.policies
                        .iter()
                        .map(|p| HPAScalingPolicy {
                            type_: p.policy_type.clone(),
                            value: p.value,
                            period_seconds: p.period_seconds,
                        })
                        .collect(),
                ),
                select_policy: Some("Max".to_string()),
            }),
            scale_down: b.scale_down.as_ref().map(|s| HPAScalingRules {
                stabilization_window_seconds: s.stabilization_window_seconds,
                policies: Some(
                    s.policies
                        .iter()
                        .map(|p| HPAScalingPolicy {
                            type_: p.policy_type.clone(),
                            value: p.value,
                            period_seconds: p.period_seconds,
                        })
                        .collect(),
                ),
                select_policy: Some("Min".to_string()),
            }),
        });

    let hpa = HorizontalPodAutoscaler {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: Some(namespace),
                labels: Some(standard_labels(node)),
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &node.spec.resource_meta,
        ),
        spec: Some(HorizontalPodAutoscalerSpec {
            scale_target_ref: CrossVersionObjectReference {
                api_version: Some("apps/v1".to_string()),
                kind: "Deployment".to_string(),
                name: deployment_name,
            },
            min_replicas: Some(autoscaling.min_replicas),
            max_replicas: autoscaling.max_replicas,
            metrics: if metrics.is_empty() {
                None
            } else {
                Some(metrics)
            },
            behavior,
        }),
        status: None,
    };

    Ok(hpa)
}

/// Delete the HPA when node is deleted
pub async fn delete_hpa(client: &Client, node: &StellarNode) -> Result<()> {
    // Only delete HPA if autoscaling was configured
    if node.spec.autoscaling.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "hpa");

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => {
            info!("HPA deleted for {}/{}", namespace, name);
        }
        Err(kube::Error::Api(api_err)) if api_err.code == 404 => {
            info!("HPA {}/{} not found (already deleted)", namespace, name);
        }
        Err(e) => {
            warn!("Failed to delete HPA {}/{}: {:?}", namespace, name, e);
        }
    }

    Ok(())
}

// ============================================================================
// ServiceMonitor (Prometheus Operator)
// ============================================================================

/// Ensure a ServiceMonitor exists for Prometheus scraping (Prometheus Operator)
///
/// ServiceMonitor is a custom resource from the Prometheus Operator.
/// Users should manually create ServiceMonitor resources or use a tool like
/// kustomize/helm to generate them. This function documents the capability.
pub async fn ensure_service_monitor(_client: &Client, node: &StellarNode) -> Result<()> {
    // Only log for Horizon and SorobanRpc nodes with autoscaling config
    if !matches!(
        node.spec.node_type,
        NodeType::Horizon | NodeType::SorobanRpc
    ) || node.spec.autoscaling.is_none()
    {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "service-monitor");

    info!(
        "ServiceMonitor configuration available for {}/{}. Users should manually create the ServiceMonitor resource.",
        namespace, name
    );

    info!(
        "ServiceMonitor should scrape metrics on port 'http' at path '/metrics' from service: {}",
        node.name_any()
    );

    Ok(())
}

/// Delete the ServiceMonitor when node is deleted
pub async fn delete_service_monitor(_client: &Client, node: &StellarNode) -> Result<()> {
    // Only delete ServiceMonitor if autoscaling was configured
    if node.spec.autoscaling.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "service-monitor");

    info!(
        "Note: ServiceMonitor {}/{} must be manually deleted if it was created",
        namespace, name
    );

    Ok(())
}

/// Delete alerting resources
pub async fn delete_alerting(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "alerts");

    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted alerting ConfigMap {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            // Already gone
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

/// Delete canary resources specifically
pub async fn delete_canary_resources(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let canary_name = format!("{name}-canary");

    // 1. Delete Canary Ingress
    if node.spec.ingress.is_some() {
        let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
        let _ = api.delete(&canary_name, &DeleteParams::default()).await;
    }

    // 2. Delete Canary Service
    let api_svc: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let _ = api_svc.delete(&canary_name, &DeleteParams::default()).await;

    // 3. Delete Canary Deployment
    let api_deploy: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let _ = api_deploy
        .delete(&canary_name, &DeleteParams::default())
        .await;

    Ok(())
}

// ============================================================================
// NetworkPolicy
// ============================================================================

/// Ensure a NetworkPolicy exists for the node when configured
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_network_policy(client: &Client, node: &StellarNode) -> Result<()> {
    let policy_cfg = match &node.spec.network_policy {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "netpol");

    let network_policy = build_network_policy(node, policy_cfg);

    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &Patch::Apply(&network_policy),
    )
    .await?;

    info!("NetworkPolicy ensured for {}/{}", namespace, name);
    Ok(())
}

fn build_network_policy(node: &StellarNode, config: &NetworkPolicyConfig) -> NetworkPolicy {
    let labels = standard_labels(node);
    let name = resource_name(node, "netpol");

    let mut ingress_rules: Vec<NetworkPolicyIngressRule> = Vec::new();

    // Determine ports based on node type
    let app_ports = match node.spec.node_type {
        NodeType::Validator => vec![
            NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11625)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
            NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11626)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
        ],
        NodeType::Horizon | NodeType::SorobanRpc => vec![NetworkPolicyPort {
            port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8000)),
            protocol: Some("TCP".to_string()),
            ..Default::default()
        }],
    };

    // Allow from specified namespaces
    if !config.allow_namespaces.is_empty() {
        let peers: Vec<NetworkPolicyPeer> = config
            .allow_namespaces
            .iter()
            .map(|ns| NetworkPolicyPeer {
                namespace_selector: Some(LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "kubernetes.io/metadata.name".to_string(),
                        ns.clone(),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();

        ingress_rules.push(NetworkPolicyIngressRule {
            from: Some(peers),
            ports: Some(app_ports.clone()),
        });
    }

    // Allow from specified pod selectors
    if let Some(pod_labels) = &config.allow_pod_selector {
        ingress_rules.push(NetworkPolicyIngressRule {
            from: Some(vec![NetworkPolicyPeer {
                pod_selector: Some(LabelSelector {
                    match_labels: Some(pod_labels.clone()),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ports: Some(app_ports.clone()),
        });
    }

    // Allow from specified CIDRs
    if !config.allow_cidrs.is_empty() {
        let peers: Vec<NetworkPolicyPeer> = config
            .allow_cidrs
            .iter()
            .map(|cidr| NetworkPolicyPeer {
                ip_block: Some(IPBlock {
                    cidr: cidr.clone(),
                    except: None,
                }),
                ..Default::default()
            })
            .collect();

        ingress_rules.push(NetworkPolicyIngressRule {
            from: Some(peers),
            ports: Some(app_ports.clone()),
        });
    }

    // Allow metrics scraping from monitoring namespace
    if config.allow_metrics_scrape {
        ingress_rules.push(NetworkPolicyIngressRule {
            from: Some(vec![NetworkPolicyPeer {
                namespace_selector: Some(LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "kubernetes.io/metadata.name".to_string(),
                        config.metrics_namespace.clone(),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ports: Some(vec![NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(9090)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            }]),
        });
    }

    // For Validators, allow peer-to-peer from other validators in the same namespace
    if node.spec.node_type == NodeType::Validator {
        ingress_rules.push(NetworkPolicyIngressRule {
            from: Some(vec![NetworkPolicyPeer {
                pod_selector: Some(LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "app.kubernetes.io/name".to_string(),
                        "stellar-node".to_string(),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ports: Some(vec![NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11625)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            }]),
        });
    }

    NetworkPolicy {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: node.namespace(),
                labels: Some(labels),
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &node.spec.resource_meta,
        ),
        spec: Some(NetworkPolicySpec {
            pod_selector: LabelSelector {
                match_labels: Some(BTreeMap::from([
                    ("app.kubernetes.io/instance".to_string(), node.name_any()),
                    (
                        "app.kubernetes.io/name".to_string(),
                        "stellar-node".to_string(),
                    ),
                ])),
                ..Default::default()
            },
            policy_types: Some(vec!["Ingress".to_string()]),
            ingress: if ingress_rules.is_empty() {
                None
            } else {
                Some(ingress_rules)
            },
            egress: None,
        }),
    }
}

/// Delete the NetworkPolicy for a node
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_network_policy(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "netpol");

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("NetworkPolicy {} deleted", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            info!("NetworkPolicy {} not found, skipping delete", name);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

// ============================================================================
// PodDisruptionBudget (PDB)
// ============================================================================

/// Internal builder for the PodDisruptionBudget resource
fn build_pdb(node: &StellarNode) -> Option<PodDisruptionBudget> {
    // PDBs are only applicable if there are multiple replicas to protect
    if node.spec.replicas <= 1 {
        return None;
    }

    let labels = standard_labels(node);
    let name = node.name_any();

    // Determine disruption constraints: default to maxUnavailable: 1 if not specified
    let (min_available, max_unavailable) =
        if node.spec.min_available.is_none() && node.spec.max_unavailable.is_none() {
            (None, Some(IntOrString::Int(1)))
        } else {
            (
                node.spec.min_available.clone(),
                node.spec.max_unavailable.clone(),
            )
        };

    Some(PodDisruptionBudget {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: node.namespace(),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(PodDisruptionBudgetSpec {
            selector: Some(LabelSelector {
                match_labels: Some(labels),
                ..Default::default()
            }),
            min_available,
            max_unavailable,
            ..Default::default()
        }),
        status: None,
    })
}

/// Ensure a PodDisruptionBudget exists for multi-replica nodes
pub async fn ensure_pdb(client: &Client, node: &StellarNode) -> Result<()> {
    // If replicas <= 1, we ensure the PDB is deleted (cleanup)
    if node.spec.replicas <= 1 {
        return delete_pdb(client, node).await;
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), &namespace);

    if let Some(pdb) = build_pdb(node) {
        let name = pdb.metadata.name.clone().unwrap();

        info!("Reconciling PodDisruptionBudget {}/{}", namespace, name);
        let params = PatchParams::apply("stellar-operator").force();
        api.patch(&name, &params, &Patch::Apply(&pdb))
            .await
            .map_err(Error::KubeError)?;
    }

    Ok(())
}

/// Delete the PodDisruptionBudget
pub async fn delete_pdb(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), &namespace);

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted PodDisruptionBudget {}/{}", namespace, name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            // Resource doesn't exist, ignore
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}
