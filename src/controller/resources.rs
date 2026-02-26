//! Kubernetes resource builders for StellarNode
//!
//! This module creates and manages the underlying Kubernetes resources
//! (Deployments, StatefulSets, Services, PVCs, ConfigMaps) for each StellarNode.

use crate::controller::resource_meta::merge_resource_meta;

// *** NEW: import kms_secret so we can accept SeedInjectionSpec ***
use super::kms_secret;

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
    HistoryMode, HsmProvider, IngressConfig, InitDbConfiguration, KeySource, ManagedDatabaseConfig,
    MonitoringConfiguration, NetworkPolicyConfig, NodeType, PgBouncerSpec, Pooler, PoolerCluster,
    PoolerSpec, PostgresConfiguration, RolloutStrategy, S3Credentials,
    SecretKeySelector as CnpgSecretKeySelector, StellarNode, StorageConfiguration,
    WalBackupConfiguration,
};
use crate::error::{Error, Result};

/// Get the standard labels for a StellarNode's resources
pub fn standard_labels(node: &StellarNode) -> BTreeMap<String, String> {
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
pub fn owner_reference(node: &StellarNode) -> OwnerReference {
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
        match node.spec.history_mode {
            HistoryMode::Full => "1500Gi".to_string(),
            HistoryMode::Recent => "100Gi".to_string(),
        }
    } else {
        node.spec.storage.size.clone()
    };
    requests.insert("storage".to_string(), Quantity(effective_storage_size));

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
    quorum_override: Option<crate::controller::vsl::QuorumSet>,
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
    quorum_override: Option<crate::controller::vsl::QuorumSet>,
    enable_mtls: bool,
) -> ConfigMap {
    let labels = standard_labels(node);
    let name = resource_name(node, "config");

    let mut data = BTreeMap::new();

    data.insert(
        "NETWORK_PASSPHRASE".to_string(),
        node.spec.network.passphrase().to_string(),
    );

    if enable_mtls {
        data.insert("MTLS_ENABLED".to_string(), "true".to_string());
    }

    match &node.spec.node_type {
        NodeType::Validator => {
            let mut core_cfg = String::new();
            if let Some(config) = &node.spec.validator_config {
                if let Some(qs) = quorum_override {
                    core_cfg.push_str(&qs.to_stellar_core_toml());
                } else if let Some(q) = &config.quorum_set {
                    core_cfg.push_str(q);
                }
            }

            if enable_mtls {
                core_cfg.push_str("\n# mTLS Configuration\n");
                core_cfg.push_str("HTTP_PORT_SECURE=true\n");
                core_cfg.push_str("TLS_CERT_FILE=\"/etc/stellar/tls/tls.crt\"\n");
                core_cfg.push_str("TLS_KEY_FILE=\"/etc/stellar/tls/tls.key\"\n");
            }

            match node.spec.history_mode {
                HistoryMode::Full => {
                    core_cfg.push_str("\n# Full History Mode\n");
                    core_cfg.push_str("CATCHUP_COMPLETE=true\n");
                }
                HistoryMode::Recent => {
                    core_cfg.push_str("\n# Recent History Mode\n");
                    core_cfg.push_str("CATCHUP_COMPLETE=false\n");
                    core_cfg.push_str("CATCHUP_RECENT=60480\n");
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
                    Some(annotations.clone())
                },
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &None,
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
            // Deployments (Horizon/SorobanRpc) never need seed injection → pass None
            template: build_pod_template(node, &labels, enable_mtls, None),
            ..Default::default()
        }),
        status: None,
    }
}

// ============================================================================
// StatefulSet (for Validators)
// ============================================================================

/// Ensure a StatefulSet exists for Validator nodes.
///
/// `seed_injection` describes how the validator seed should be mounted into
/// the pod — either as an env var from a Secret/ExternalSecret, or as a CSI
/// volume mount. Pass `None` when called for non-validator nodes.
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_statefulset(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
    // *** NEW PARAMETER ***
    seed_injection: Option<&kms_secret::SeedInjectionSpec>,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    // *** Pass seed_injection down to the builder ***
    let statefulset = build_statefulset(node, enable_mtls, seed_injection);

    let patch = Patch::Apply(&statefulset);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    Ok(())
}

// *** seed_injection added as parameter ***
fn build_statefulset(
    node: &StellarNode,
    enable_mtls: bool,
    seed_injection: Option<&kms_secret::SeedInjectionSpec>,
) -> StatefulSet {
    let labels = standard_labels(node);
    let name = node.name_any();

    let replicas = if node.spec.suspended { 0 } else { 1 };

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
            // *** Pass seed_injection into pod template builder ***
            template: build_pod_template(node, &labels, enable_mtls, seed_injection),
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
// LoadBalancer Service (MetalLB Integration) — stubs unchanged
// ============================================================================

#[allow(dead_code)]
#[instrument(skip(_client, _node), fields(name = %_node.name_any(), namespace = _node.namespace()))]
pub async fn ensure_load_balancer_service(_client: &Client, _node: &StellarNode) -> Result<()> {
    Ok(())
}

#[instrument(skip(_client, _node), fields(name = %_node.name_any(), namespace = _node.namespace()))]
pub async fn delete_load_balancer_service(_client: &Client, _node: &StellarNode) -> Result<()> {
    Ok(())
}

#[allow(dead_code)]
#[instrument(skip(_client, _node), fields(name = %_node.name_any(), namespace = _node.namespace()))]
pub async fn ensure_metallb_config(_client: &Client, _node: &StellarNode) -> Result<()> {
    Ok(())
}

#[instrument(skip(_client, _node), fields(name = %_node.name_any(), namespace = _node.namespace()))]
pub async fn delete_metallb_config(_client: &Client, _node: &StellarNode) -> Result<()> {
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
// CloudNativePG (CNPG) Resources — unchanged
// ============================================================================

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

    if !config.postgres_version.is_empty() {
        cluster.spec.image_name = Some(format!(
            "ghcr.io/cloudnative-pg/postgresql:{}",
            config.postgres_version
        ));
    }

    cluster
}

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

#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_cnpg_resources(client: &Client, node: &StellarNode) -> Result<()> {
    if node.spec.managed_database.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    let pooler_api: Api<Pooler> = Api::namespaced(client.clone(), &namespace);
    let pooler_name = resource_name(node, "pooler");
    let _ = pooler_api
        .delete(&pooler_name, &DeleteParams::default())
        .await;

    let cluster_api: Api<Cluster> = Api::namespaced(client.clone(), &namespace);
    let cluster_name = node.name_any();
    let _ = cluster_api
        .delete(&cluster_name, &DeleteParams::default())
        .await;

    Ok(())
}

// ============================================================================
// Ingress — unchanged
// ============================================================================

#[allow(dead_code)]
pub async fn ensure_ingress(client: &Client, node: &StellarNode) -> Result<()> {
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
            annotations.insert(
                "traefik.ingress.kubernetes.io/service.weights".to_string(),
                format!("{}:{}", node.name_any(), cfg.weight),
            );

            canary_ingress.metadata.annotations = Some(annotations);

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
            let canary_name = format!("{name}-canary");
            let _ = api.delete(&canary_name, &DeleteParams::default()).await;
        }
    }

    Ok(())
}

#[allow(dead_code)]
fn build_ingress(node: &StellarNode, config: &IngressConfig) -> Ingress {
    let labels = standard_labels(node);
    let name = resource_name(node, "ingress");

    let service_port = match node.spec.node_type {
        NodeType::Horizon | NodeType::SorobanRpc => 8000,
        NodeType::Validator => 11626,
    };

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

/// Build the pod template.
///
/// `seed_injection` is `Some` only for Validator StatefulSets; it adds the
/// env vars / volumes / mounts required to deliver the seed from KMS/ESO/CSI.
fn build_pod_template(
    node: &StellarNode,
    labels: &BTreeMap<String, String>,
    enable_mtls: bool,
    // *** NEW PARAMETER ***
    seed_injection: Option<&kms_secret::SeedInjectionSpec>,
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
        topology_spread_constraints: Some(build_topology_spread_constraints(
            &node.spec,
            &node.name_any(),
        )),
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
                    let volumes = pod_spec.volumes.get_or_insert_with(Vec::new);
                    volumes.push(Volume {
                        name: "keys".to_string(),
                        empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
                            medium: Some("Memory".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });

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
                if hsm_config.provider == HsmProvider::AWS {
                    volumes.push(Volume {
                        name: "cloudhsm-socket".to_string(),
                        empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
                            medium: Some("Memory".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });

                    let containers = &mut pod_spec.containers;
                    containers.push(Container {
                        name: "cloudhsm-client".to_string(),
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

    // ==========================================================================
    // NEW: Inject KMS/ESO/CSI seed env vars, volumes, and volume mounts
    // ==========================================================================
    if let Some(inj) = seed_injection {
        // Extend the main container (index 0) with seed env vars and volume mounts
        if let Some(container) = pod_spec.containers.first_mut() {
            if let Some(ref mut env) = container.env {
                env.extend(inj.env_vars());
            } else {
                container.env = Some(inj.env_vars());
            }
            if let Some(ref mut mounts) = container.volume_mounts {
                mounts.extend(inj.volume_mounts());
            } else {
                let vm = inj.volume_mounts();
                if !vm.is_empty() {
                    container.volume_mounts = Some(vm);
                }
            }
        }
        // Extend pod volumes with any CSI volume
        if let Some(ref mut vols) = pod_spec.volumes {
            vols.extend(inj.volumes());
        }
    }
    // ==========================================================================

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

/// Build `TopologySpreadConstraints` for a pod spec.
pub fn build_topology_spread_constraints(
    spec: &crate::crd::StellarNodeSpec,
    node_name: &str,
) -> Vec<k8s_openapi::api::core::v1::TopologySpreadConstraint> {
    use k8s_openapi::api::core::v1::TopologySpreadConstraint;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
    use std::collections::BTreeMap;

    if let Some(constraints) = &spec.topology_spread_constraints {
        if !constraints.is_empty() {
            return constraints.clone();
        }
    }

    let selector = LabelSelector {
        match_labels: Some(BTreeMap::from([
            (
                "app.kubernetes.io/name".to_string(),
                "stellar-node".to_string(),
            ),
            (
                "app.kubernetes.io/instance".to_string(),
                node_name.to_string(),
            ),
        ])),
        ..Default::default()
    };

    vec![
        TopologySpreadConstraint {
            max_skew: 1,
            topology_key: "kubernetes.io/hostname".to_string(),
            when_unsatisfiable: "DoNotSchedule".to_string(),
            label_selector: Some(selector.clone()),
            ..Default::default()
        },
        TopologySpreadConstraint {
            max_skew: 1,
            topology_key: "topology.kubernetes.io/zone".to_string(),
            when_unsatisfiable: "DoNotSchedule".to_string(),
            label_selector: Some(selector),
            ..Default::default()
        },
    ]
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
                    // Only inject the legacy env var when seed_secret_source is NOT set.
                    // When seed_secret_source IS set, the injection is handled via
                    // seed_injection in build_pod_template so we skip it here.
                    if validator_config.seed_secret_source.is_none()
                        && !validator_config.seed_secret_ref.is_empty()
                    {
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
                }
                KeySource::KMS => {
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
        let secret_name = node.name_any();
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
                env_vars.push(EnvVar {
                    name: "PKCS11_MODULE_PATH".to_string(),
                    value: Some(hsm_config.pkcs11_lib_path.clone()),
                    ..Default::default()
                });

                if let Some(ip) = &hsm_config.hsm_ip {
                    env_vars.push(EnvVar {
                        name: "HSM_IP_ADDRESS".to_string(),
                        value: Some(ip.clone()),
                        ..Default::default()
                    });
                }

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
    container.command = Some(vec!["/bin/sh".to_string()]);
    container.args = Some(vec![
        "-c".to_string(),
        "horizon db upgrade || horizon db init".to_string(),
    ]);
    container.ports = None;
    container.liveness_probe = None;
    container.readiness_probe = None;
    container.startup_probe = None;
    container.lifecycle = None;
    container
}

// ============================================================================
// HorizontalPodAutoscaler — unchanged
// ============================================================================

pub async fn ensure_hpa(client: &Client, node: &StellarNode) -> Result<()> {
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
// Alerting — unchanged
// ============================================================================

pub async fn ensure_alerting(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "alerts");

    if !node.spec.alerting {
        return delete_alerting(client, node).await;
    }

    let labels = standard_labels(node);
    let mut data = BTreeMap::new();

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
                        value: Some(Quantity("5".to_string())),
                        ..Default::default()
                    },
                }),
                ..Default::default()
            });
        }
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

pub async fn delete_hpa(client: &Client, node: &StellarNode) -> Result<()> {
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
// ServiceMonitor — unchanged
// ============================================================================

pub async fn ensure_service_monitor(_client: &Client, node: &StellarNode) -> Result<()> {
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

    Ok(())
}

pub async fn delete_service_monitor(_client: &Client, node: &StellarNode) -> Result<()> {
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

pub async fn delete_alerting(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "alerts");

    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted alerting ConfigMap {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

pub async fn delete_canary_resources(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let canary_name = format!("{name}-canary");

    if node.spec.ingress.is_some() {
        let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
        let _ = api.delete(&canary_name, &DeleteParams::default()).await;
    }

    let api_svc: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let _ = api_svc.delete(&canary_name, &DeleteParams::default()).await;

    let api_deploy: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let _ = api_deploy
        .delete(&canary_name, &DeleteParams::default())
        .await;

    Ok(())
}

// ============================================================================
// NetworkPolicy — unchanged
// ============================================================================

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
// PodDisruptionBudget — unchanged
// ============================================================================

fn build_pdb(node: &StellarNode) -> Option<PodDisruptionBudget> {
    if node.spec.replicas <= 1 {
        return None;
    }

    let labels = standard_labels(node);
    let name = node.name_any();

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

pub async fn ensure_pdb(client: &Client, node: &StellarNode) -> Result<()> {
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

pub async fn delete_pdb(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), &namespace);

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted PodDisruptionBudget {}/{}", namespace, name),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}
