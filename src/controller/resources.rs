//! Kubernetes resource builders for StellarNode
//!
//! This module creates and manages the underlying Kubernetes resources
//! (Deployments, StatefulSets, Services, PVCs, ConfigMaps) for each StellarNode.

use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, StatefulSet, StatefulSetSpec};
use k8s_openapi::api::autoscaling::v2::{
    CrossVersionObjectReference, HorizontalPodAutoscaler, HorizontalPodAutoscalerSpec,
};
use k8s_openapi::api::core::v1::{
    ConfigMap, Container, ContainerPort, EnvVar, EnvVarSource, PersistentVolumeClaim,
    PersistentVolumeClaimSpec, PodSpec, PodTemplateSpec, ResourceRequirements as K8sResources,
    SecretKeySelector, Service, ServicePort, ServiceSpec, Volume, VolumeMount,
    VolumeResourceRequirements,
};
use k8s_openapi::api::networking::v1::{
    HTTPIngressPath, HTTPIngressRuleValue, Ingress, IngressBackend, IngressRule,
    IngressServiceBackend, IngressSpec, IngressTLS, ServiceBackendPort,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta, OwnerReference};
use kube::api::{Api, DeleteParams, Patch, PatchParams, PostParams};
use kube::{Client, Resource, ResourceExt};
use tracing::{info, instrument, warn};

use crate::crd::{IngressConfig, KeySource, NodeType, StellarNode};
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
    requests.insert(
        "storage".to_string(),
        Quantity(node.spec.storage.size.clone()),
    );

    // Merge custom annotations from storage config with existing annotations
    let annotations = node.spec.storage.annotations.clone().unwrap_or_default();

    PersistentVolumeClaim {
        metadata: ObjectMeta {
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
    enable_mtls: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "config");

    let cm = build_config_map(node, enable_mtls);

    let patch = Patch::Apply(&cm);
    api.patch(&name, &PatchParams::apply("stellar-operator"), &patch)
        .await?;

    Ok(())
}

fn build_config_map(node: &StellarNode, enable_mtls: bool) -> ConfigMap {
    let labels = standard_labels(node);
    let name = resource_name(node, "config");

    let mut data = BTreeMap::new();

    // Add network-specific configuration
    data.insert(
        "NETWORK_PASSPHRASE".to_string(),
        node.spec.network.passphrase().to_string(),
    );

    // Add node-type-specific configuration
    match &node.spec.node_type {
        NodeType::Validator => {
            let mut core_cfg = String::new();
            if let Some(config) = &node.spec.validator_config {
                if let Some(quorum) = &config.quorum_set {
                    core_cfg.push_str(quorum);
                }
            }
            if enable_mtls {
                core_cfg.push_str("\n# mTLS Configuration\n");
                core_cfg.push_str("HTTP_PORT_SECURE=true\n");
                core_cfg.push_str("TLS_CERT_FILE=\"/etc/stellar/tls/tls.crt\"\n");
                core_cfg.push_str("TLS_KEY_FILE=\"/etc/stellar/tls/tls.key\"\n");
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
                if let Some(captive_config) = &config.captive_core_config {
                    data.insert("captive-core.cfg".to_string(), captive_config.clone());
                }
            }
        }
    }

    ConfigMap {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: node.namespace(),
            labels: Some(labels),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        data: Some(data),
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

fn build_deployment(node: &StellarNode, enable_mtls: bool) -> Deployment {
    let labels = standard_labels(node);
    let name = node.name_any();

    let replicas = if node.spec.suspended {
        0
    } else {
        node.spec.replicas
    };

    Deployment {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: node.namespace(),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
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

    StatefulSet {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: node.namespace(),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(StatefulSetSpec {
            replicas: Some(replicas),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            service_name: format!("{}-headless", name),
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
        metadata: ObjectMeta {
            name: Some(name),
            namespace: node.namespace(),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(labels),
            ports: Some(ports),
            ..Default::default()
        }),
        status: None,
    }
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
            ..Default::default()
        }]
    });

    Ingress {
        metadata: ObjectMeta {
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
        ..Default::default()
    };

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

    PodTemplateSpec {
        metadata: Some(ObjectMeta {
            labels: Some(labels.clone()),
            ..Default::default()
        }),
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
                        ..Default::default()
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
                    optional: None,
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
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(labels),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
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

    // Note: Custom metrics require Prometheus Adapter to be installed
    // For now, we create a basic HPA with just the min/max replicas configured
    // Users can manually add metrics via kubectl or kustomize/helm patches
    if !autoscaling.custom_metrics.is_empty() {
        info!(
            "Custom metrics configured: {:?}. These require Prometheus Adapter to be installed.",
            autoscaling.custom_metrics
        );
    }

    let hpa = HorizontalPodAutoscaler {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace),
            labels: Some(standard_labels(node)),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(HorizontalPodAutoscalerSpec {
            scale_target_ref: CrossVersionObjectReference {
                api_version: Some("apps/v1".to_string()),
                kind: "Deployment".to_string(),
                name: deployment_name,
            },
            min_replicas: Some(autoscaling.min_replicas),
            max_replicas: autoscaling.max_replicas,
            metrics: None,
            behavior: None,
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
