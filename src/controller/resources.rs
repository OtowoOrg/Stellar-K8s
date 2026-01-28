use std::collections::BTreeMap;

use k8s_openapi::api::autoscaling::v2::{
    HorizontalPodAutoscaler, HorizontalPodAutoscalerSpec, MetricSpec,
    MetricTarget, ObjectMetricSource,
};
use k8s_openapi::api::core::v1::{
    ConfigMap, Container, ContainerPort, EnvVar, PersistentVolumeClaim, PersistentVolumeClaimSpec, PodTemplateSpec,
    SecretVolumeSource, Service, ServicePort, ServiceSpec, Volume, VolumeMount,
    VolumeResourceRequirements,
};
use k8s_openapi::api::networking::v1::{
    Ingress, NetworkPolicy, NetworkPolicyIngressRule, NetworkPolicySpec,
};
use k8s_openapi::api::policy::v1::{PodDisruptionBudget, PodDisruptionBudgetSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta, OwnerReference};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    api::{Api, DeleteParams, Patch, PatchParams, PostParams},
    client::Client,
    CustomResource, Resource, ResourceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::crd::{
    NodeType, StellarNode,
};
use crate::error::{Error, Result};
use k8s_openapi::api::core::v1::ResourceRequirements as K8sResources;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

// ============================================================================
// Helpers
// ============================================================================

fn resource_name(node: &StellarNode, suffix: &str) -> String {
    format!("{}-{}", node.name_any(), suffix)
}

fn standard_labels(node: &StellarNode) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    labels.insert("app.kubernetes.io/name".to_string(), "stellar-node".to_string());
    labels.insert(
        "app.kubernetes.io/instance".to_string(),
        node.name_any(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "stellar-operator".to_string(),
    );
    labels.insert("stellar.org/network".to_string(), format!("{:?}", node.spec.network));
    labels.insert("stellar.org/type".to_string(), format!("{:?}", node.spec.node_type));
    labels
}

fn owner_reference(node: &StellarNode) -> OwnerReference {
    OwnerReference {
        api_version: "stellar.org/v1alpha1".to_string(),
        kind: "StellarNode".to_string(),
        name: node.name_any(),
        uid: node.uid().unwrap_or_default(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    }
}

// ============================================================================
// Core Resources
// ============================================================================

pub async fn ensure_pvc(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "data");
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);

    if api.get(&name).await.is_ok() {
        return Ok(());
    }

    // Determine size based on HistoryMode
    let size = if !node.spec.storage.size.is_empty() {
        node.spec.storage.size.clone()
    } else {
        match node.spec.history_mode {
            crate::crd::HistoryMode::Full => "1Ti".to_string(),
            crate::crd::HistoryMode::Recent => "20Gi".to_string(),
        }
    };

    info!("Creating PVC {}/{} with size {}", namespace, name, size);

    let mut requests = BTreeMap::new();
    requests.insert("storage".to_string(), Quantity(size));

    let pvc = PersistentVolumeClaim {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(standard_labels(node)),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOnce".to_string()]),
            resources: Some(VolumeResourceRequirements {
                requests: Some(requests),
                ..Default::default()
            }),
            storage_class_name: Some(node.spec.storage.storage_class.clone()),
            ..Default::default()
        }),
        ..Default::default()
    };

    api.create(&PostParams::default(), &pvc)
        .await
        .map_err(Error::KubeError)?;

    Ok(())
}

pub async fn delete_pvc(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "data");
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted PVC {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => return Err(Error::KubeError(e)),
    }
    Ok(())
}

pub async fn ensure_config_map(
    client: &Client,
    node: &StellarNode,
    _quorum_override: Option<String>,
    _enable_mtls: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "config");
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);

    let mut data = BTreeMap::new();
    let config_content = format!(
        "# Generated by Stellar Operator\n# Node: {}\n# Network: {:?}\nHTTP_PORT=11626\nPEER_PORT=11625\n", 
        node.name_any(), 
        node.spec.network
    );
    data.insert("stellar-core.cfg".to_string(), config_content);

    let cm = ConfigMap {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(standard_labels(node)),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        data: Some(data),
        ..Default::default()
    };

    let patch = Patch::Apply(&cm);
    api.patch(&name, &PatchParams::apply("stellar-operator").force(), &patch)
        .await
        .map_err(Error::KubeError)?;

    info!("ConfigMap ensured for {}/{}", namespace, name);
    Ok(())
}

pub async fn delete_config_map(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "config");
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => info!("Deleted ConfigMap {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => return Err(Error::KubeError(e)),
    }
    Ok(())
}

// ============================================================================
// Workload
// ============================================================================

use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, StatefulSet, StatefulSetSpec};

pub async fn ensure_statefulset(client: &Client, node: &StellarNode, enable_mtls: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);

    let labels = standard_labels(node);
    let replicas = if node.spec.suspended { 0 } else { node.spec.replicas };
    let pod_template = build_pod_template(node, &labels, enable_mtls);

    let sts = StatefulSet {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(StatefulSetSpec {
            replicas: Some(replicas),
            selector: LabelSelector {
                match_labels: Some(labels),
                ..Default::default()
            },
            service_name: resource_name(node, "headless"),
            template: pod_template,
            ..Default::default()
        }),
        ..Default::default()
    };

    let patch = Patch::Apply(&sts);
    api.patch(&name, &PatchParams::apply("stellar-operator").force(), &patch)
        .await.map_err(Error::KubeError)?;
    Ok(())
}

pub async fn ensure_deployment(client: &Client, node: &StellarNode, enable_mtls: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);

    let labels = standard_labels(node);
    let replicas = if node.spec.suspended { 0 } else { node.spec.replicas };
    let pod_template = build_pod_template(node, &labels, enable_mtls);

    let deploy = Deployment {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(replicas),
            selector: LabelSelector {
                match_labels: Some(labels),
                ..Default::default()
            },
            template: pod_template,
            ..Default::default()
        }),
        ..Default::default()
    };

    let patch = Patch::Apply(&deploy);
    api.patch(&name, &PatchParams::apply("stellar-operator").force(), &patch)
        .await.map_err(Error::KubeError)?;
    Ok(())
}

pub async fn ensure_canary_deployment(client: &Client, node: &StellarNode, enable_mtls: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = format!("{}-canary", node.name_any());
    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);

    let mut labels = standard_labels(node);
    labels.insert("stellar.org/deployment".to_string(), "canary".to_string());
    let pod_template = build_pod_template(node, &labels, enable_mtls);

    let deploy = Deployment {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(1),
            selector: LabelSelector {
                match_labels: Some(labels),
                ..Default::default()
            },
            template: pod_template,
            ..Default::default()
        }),
        ..Default::default()
    };

    let patch = Patch::Apply(&deploy);
    api.patch(&name, &PatchParams::apply("stellar-operator").force(), &patch)
        .await.map_err(Error::KubeError)?;
    Ok(())
}

pub async fn delete_workload(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    match node.spec.node_type {
        NodeType::Validator => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
            let _ = api.delete(&name, &DeleteParams::default()).await;
        },
        _ => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
            let _ = api.delete(&name, &DeleteParams::default()).await;
        }
    }
    Ok(())
}

pub async fn delete_canary_resources(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = format!("{}-canary", node.name_any());

    let deploy_api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let _ = deploy_api.delete(&name, &DeleteParams::default()).await;

    let svc_api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let _ = svc_api.delete(&name, &DeleteParams::default()).await;

    Ok(())
}

fn build_pod_template(node: &StellarNode, labels: &BTreeMap<String, String>, enable_mtls: bool) -> PodTemplateSpec {
    let mut pod_spec = k8s_openapi::api::core::v1::PodSpec {
        containers: vec![build_container(node, enable_mtls)],
        volumes: Some(vec![
            Volume {
                name: "data".to_string(),
                persistent_volume_claim: Some(k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                    claim_name: resource_name(node, "data"),
                    ..Default::default()
                }),
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

    if let NodeType::Horizon = node.spec.node_type {
        if let Some(horizon_config) = &node.spec.horizon_config {
            if horizon_config.auto_migration {
                let init_containers = pod_spec.init_containers.get_or_insert_with(Vec::new);
                init_containers.push(build_horizon_migration_container(node));
            }
        }
    }

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

fn build_container(node: &StellarNode, _enable_mtls: bool) -> Container {
    let mut requests = BTreeMap::new();
    requests.insert("cpu".to_string(), Quantity(node.spec.resources.requests.cpu.clone()));
    requests.insert("memory".to_string(), Quantity(node.spec.resources.requests.memory.clone()));

    let mut limits = BTreeMap::new();
    limits.insert("cpu".to_string(), Quantity(node.spec.resources.limits.cpu.clone()));
    limits.insert("memory".to_string(), Quantity(node.spec.resources.limits.memory.clone()));

    let (container_port, data_mount_path, _) = match node.spec.node_type {
        NodeType::Validator => (11625, "/opt/stellar/data", "DATABASE"),
        NodeType::Horizon => (8000, "/data", "DATABASE_URL"),
        NodeType::SorobanRpc => (8000, "/data", "DATABASE_URL"),
    };

    let mut env_vars = vec![EnvVar {
        name: "NETWORK_PASSPHRASE".to_string(),
        value: Some(node.spec.network.passphrase().to_string()),
        ..Default::default()
    }];

    use crate::crd::HistoryMode;
    let (complete, recent) = match node.spec.history_mode {
        HistoryMode::Full => ("true", "0"),
        HistoryMode::Recent => ("false", "1024"),
    };

    env_vars.push(EnvVar { name: "CATCHUP_COMPLETE".to_string(), value: Some(complete.to_string()), ..Default::default() });
    env_vars.push(EnvVar { name: "CATCHUP_RECENT".to_string(), value: Some(recent.to_string()), ..Default::default() });

    let mut volume_mounts = vec![
        VolumeMount { name: "data".to_string(), mount_path: data_mount_path.to_string(), ..Default::default() },
        VolumeMount { name: "config".to_string(), mount_path: "/config".to_string(), read_only: Some(true), ..Default::default() },
    ];

    volume_mounts.push(VolumeMount {
        name: "tls".to_string(),
        mount_path: "/etc/stellar/tls".to_string(),
        read_only: Some(true),
        ..Default::default()
    });

    Container {
        name: "stellar-node".to_string(),
        image: Some(node.spec.container_image()),
        ports: Some(vec![ContainerPort { container_port, ..Default::default() }]),
        env: Some(env_vars),
        resources: Some(K8sResources { requests: Some(requests), limits: Some(limits), claims: None }),
        volume_mounts: Some(volume_mounts),
        ..Default::default()
    }
}

fn build_horizon_migration_container(node: &StellarNode) -> Container {
    let mut container = build_container(node, false);
    container.name = "horizon-db-migration".to_string();
    container.command = Some(vec!["/bin/sh".to_string()]);
    container.args = Some(vec!["-c".to_string(), "horizon db upgrade || horizon db init".to_string()]);
    container.ports = None;
    container.liveness_probe = None;
    container.readiness_probe = None;
    container.startup_probe = None;
    container.lifecycle = None;
    container
}

// ============================================================================
// Service
// ============================================================================

pub async fn ensure_service(client: &Client, node: &StellarNode, _enable_mtls: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "service");
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);

    let labels = standard_labels(node);
    let mut ports = vec![];
    if node.spec.node_type == NodeType::Validator {
        ports.push(ServicePort { name: Some("peer".to_string()), port: 11625, target_port: Some(IntOrString::Int(11625)), ..Default::default() });
        ports.push(ServicePort { name: Some("http".to_string()), port: 11626, target_port: Some(IntOrString::Int(11626)), ..Default::default() });
    } else {
         ports.push(ServicePort { name: Some("http".to_string()), port: 8000, target_port: Some(IntOrString::Int(8000)), ..Default::default() });
    }

    let svc = Service {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(labels),
            ports: Some(ports),
            type_: Some("ClusterIP".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let patch = Patch::Apply(&svc);
    api.patch(&name, &PatchParams::apply("stellar-operator").force(), &patch).await.map_err(Error::KubeError)?;
    Ok(())
}

pub async fn delete_service(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "service");
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let _ = api.delete(&name, &DeleteParams::default()).await;
    Ok(())
}

pub async fn ensure_canary_service(_client: &Client, _node: &StellarNode, _enable_mtls: bool) -> Result<()> { Ok(()) }
pub async fn ensure_load_balancer_service(_client: &Client, _node: &StellarNode) -> Result<()> { Ok(()) }
pub async fn delete_load_balancer_service(_client: &Client, _node: &StellarNode) -> Result<()> { Ok(()) }
pub async fn ensure_metallb_config(_client: &Client, _node: &StellarNode) -> Result<()> { Ok(()) }
pub async fn delete_metallb_config(_client: &Client, _node: &StellarNode) -> Result<()> { Ok(()) }

// ============================================================================
// Ingress
// ============================================================================

pub async fn ensure_ingress(client: &Client, node: &StellarNode) -> Result<()> {
    if node.spec.ingress.is_none() {
        return delete_ingress(client, node).await;
    }
    Ok(())
}

pub async fn delete_ingress(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "ingress");
    let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
    let _ = api.delete(&name, &DeleteParams::default()).await;
    Ok(())
}

// ============================================================================
// ServiceMonitor
// ============================================================================

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct MonitorLabelSelector {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_labels: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_expressions: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMonitorEndpoint {
    pub port: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_config: Option<ServiceMonitorTlsConfig>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMonitorTlsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insecure_skip_verify: Option<bool>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMonitorNamespaceSelector {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_names: Option<Vec<String>>,
}

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "monitoring.coreos.com",
    version = "v1",
    kind = "ServiceMonitor",
    namespaced
)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMonitorSpec {
    pub selector: MonitorLabelSelector,
    pub endpoints: Vec<ServiceMonitorEndpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace_selector: Option<ServiceMonitorNamespaceSelector>,
}

fn build_service_monitor(node: &StellarNode, enable_mtls: bool) -> ServiceMonitor {
    let name = resource_name(node, "monitor");
    let labels = standard_labels(node);
    let port_name = if enable_mtls { "https" } else { "http" };
    let scheme = if enable_mtls { Some("https".to_string()) } else { None };
    let tls_config = if enable_mtls { Some(ServiceMonitorTlsConfig { insecure_skip_verify: Some(true) }) } else { None };

    ServiceMonitor::new(
        &name,
        ServiceMonitorSpec {
            selector: MonitorLabelSelector {
                match_labels: Some(labels.clone()),
                match_expressions: None,
            },
            endpoints: vec![ServiceMonitorEndpoint {
                port: port_name.to_string(),
                path: Some("/metrics".to_string()),
                interval: Some("30s".to_string()),
                scheme,
                tls_config,
            }],
            namespace_selector: None,
        },
    )
}

pub async fn ensure_service_monitor(client: &Client, node: &StellarNode, enable_mtls: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ServiceMonitor> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "monitor");
    let sm = build_service_monitor(node, enable_mtls);

    if let Err(e) = api.patch(&name, &PatchParams::apply("stellar-operator").force(), &Patch::Apply(&sm)).await {
         warn!("Failed to ensure ServiceMonitor: {}", e);
    }
    Ok(())
}

pub async fn delete_service_monitor(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ServiceMonitor> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "monitor");
    let _ = api.delete(&name, &DeleteParams::default()).await;
    Ok(())
}

// ============================================================================
// HPA
// ============================================================================

pub async fn ensure_hpa(_client: &Client, _node: &StellarNode) -> Result<()> { Ok(()) }
pub async fn delete_hpa(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "hpa");
    let _ = api.delete(&name, &DeleteParams::default()).await;
    Ok(())
}

// ============================================================================
// PDB
// ============================================================================

pub async fn ensure_pdb(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "pdb");

    if node.spec.min_available.is_none() && node.spec.max_unavailable.is_none() {
        if api.get(&name).await.is_ok() {
             let _ = api.delete(&name, &DeleteParams::default()).await;
        }
        return Ok(());
    }

    let labels = standard_labels(node);
    let pdb = PodDisruptionBudget {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(PodDisruptionBudgetSpec {
            min_available: node.spec.min_available.clone(),
            max_unavailable: node.spec.max_unavailable.clone(),
            selector: Some(LabelSelector {
                match_labels: Some(labels),
                ..Default::default()
            }),
            unhealthy_pod_eviction_policy: None,
        }),
        status: None,
    };

    let patch = Patch::Apply(&pdb);
    api.patch(&name, &PatchParams::apply("stellar-operator").force(), &patch)
        .await.map_err(Error::KubeError)?;
    Ok(())
}

// ============================================================================
// Network Policy
// ============================================================================

fn build_network_policy(node: &StellarNode) -> NetworkPolicy {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "netpol");
    let labels = standard_labels(node);
    let ports = vec![]; 

    NetworkPolicy {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace.clone()),
            labels: Some(labels.clone()),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(NetworkPolicySpec {
            pod_selector: LabelSelector {
                match_labels: Some(labels),
                ..Default::default()
            },
            policy_types: Some(vec!["Ingress".to_string()]),
            ingress: Some(vec![NetworkPolicyIngressRule {
                ports: Some(ports),
                from: None,
            }]),
            egress: None,
        }),
    }
}

pub async fn ensure_network_policy(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "netpol");

    if node.spec.network_policy.is_none() {
        return delete_network_policy(client, node).await;
    }

    let policy = build_network_policy(node);
    let patch = Patch::Apply(&policy);
    api.patch(&name, &PatchParams::apply("stellar-operator").force(), &patch)
        .await.map_err(Error::KubeError)?;
    Ok(())
}

pub async fn delete_network_policy(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "netpol");
    let _ = api.delete(&name, &DeleteParams::default()).await;
    Ok(())
}

// ============================================================================
// Alerting
// ============================================================================

pub async fn ensure_alerting(client: &Client, node: &StellarNode) -> Result<()> {
    if !node.spec.alerting {
        return delete_alerting(client, node).await;
    }
    Ok(())
}

pub async fn delete_alerting(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "alerts");
    let _ = api.delete(&name, &DeleteParams::default()).await;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{NetworkPolicyConfig, StellarNodeSpec, StorageConfig};

    fn mock_node() -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("test-ns".to_string()),
                uid: Some("test-uid".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Validator,
                storage: StorageConfig {
                    size: "10Gi".to_string(),
                    storage_class: "standard".to_string(),
                    retention_policy: crate::crd::RetentionPolicy::Retain,
                    annotations: None,
                },
                ..Default::default()
            },
            status: None,
        }
    }

    #[test]
    fn test_build_service_monitor() {
        let node = mock_node();
        let sm = build_service_monitor(&node, false);
        assert_eq!(sm.metadata.name.unwrap(), "test-node-monitor");
    }

    #[test]
    fn test_build_network_policy() {
        let mut node = mock_node();
        node.spec.network_policy = Some(NetworkPolicyConfig { 
            enabled: true,
            allow_cidrs: vec![],
            allow_namespaces: vec![],
            allow_metrics_scrape: false,
            allow_pod_selector: None,
            // FIX: This field is a String, so we must provide an empty string, not None
            metrics_namespace: "".to_string(), 
        });

        let netpol = build_network_policy(&node);
        assert_eq!(netpol.metadata.name.unwrap(), "test-node-netpol");
    }
}