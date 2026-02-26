//! Read-only replica pool management.
//!
//! Manages the full lifecycle of read replica resources:
//! - `StatefulSet` — the pool of read-only stellar-core replicas
//! - `Service` (ClusterIP) — stable DNS endpoint for clients
//! - `HorizontalPodAutoscaler` (v2) — CPU/memory-based autoscaling
//! - `ConfigMap` — startup script with archive sharding logic
//!
//! All resources are created when `spec.readReplicaConfig` is set and
//! cleaned up when it is removed.

use k8s_openapi::api::apps::v1::{StatefulSet, StatefulSetSpec};
use k8s_openapi::api::autoscaling::v2::{
    CrossVersionObjectReference, HorizontalPodAutoscaler, HorizontalPodAutoscalerSpec, MetricSpec,
    MetricTarget,
};
use k8s_openapi::api::core::v1::{
    ConfigMap, Container, ContainerPort, PodSpec, PodTemplateSpec, Service, ServicePort,
    ServiceSpec, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    api::{Api, DeleteParams, Patch, PatchParams},
    Client, ResourceExt,
};
use std::collections::BTreeMap;
use tracing::{info, instrument, warn};

use crate::crd::{ReadReplicaConfig, StellarNode};
use crate::error::Result;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Port that stellar-core exposes its HTTP API on
const STELLAR_CORE_HTTP_PORT: i32 = 11626;
/// Port used for peer-to-peer communication
const STELLAR_CORE_PEER_PORT: i32 = 11625;
/// Field manager name for server-side apply
const FIELD_MANAGER: &str = "stellar-operator";
/// Default CPU utilization target (%) for HPA
const DEFAULT_CPU_TARGET: i32 = 70;
/// Default memory utilization target (%) for HPA
const DEFAULT_MEMORY_TARGET: i32 = 80;

// ---------------------------------------------------------------------------
// Name helpers
// ---------------------------------------------------------------------------

fn statefulset_name(node: &StellarNode) -> String {
    format!("{}-read", node.name_any())
}

fn service_name(node: &StellarNode) -> String {
    format!("{}-read", node.name_any())
}

fn hpa_name(node: &StellarNode) -> String {
    format!("{}-read-hpa", node.name_any())
}

fn configmap_name(node: &StellarNode) -> String {
    format!("{}-read-config", node.name_any())
}

/// Returns the DNS name clients should use to reach the read pool.
/// Format: `<name>-read.<namespace>.svc.cluster.local`
pub fn read_pool_endpoint(node: &StellarNode) -> String {
    let ns = node.namespace().unwrap_or_else(|| "default".to_string());
    format!("{}.{}.svc.cluster.local", service_name(node), ns)
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

fn read_pool_labels(node: &StellarNode) -> BTreeMap<String, String> {
    let mut labels = super::resources::standard_labels(node);
    labels.insert("stellar.org/role".to_string(), "read-replica".to_string());
    labels
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Ensure the complete read-pool stack (StatefulSet + Service + HPA + ConfigMap).
///
/// If `read_replica_config` is `None` the entire stack is cleaned up.
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_read_pool(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
) -> Result<()> {
    if node.spec.read_replica_config.is_none() {
        delete_read_pool(client, node).await?;
        return Ok(());
    }

    let config = node.spec.read_replica_config.as_ref().unwrap();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    // 1. ConfigMap (startup script)
    ensure_read_config_map(client, node).await?;

    // 2. StatefulSet
    ensure_read_statefulset(client, node, config, enable_mtls).await?;

    // 3. ClusterIP Service
    ensure_read_service(client, node).await?;

    // 4. HPA
    ensure_read_hpa(client, node, config).await?;

    info!(
        "Read-pool stack ensured for {}/{}",
        namespace,
        node.name_any()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// StatefulSet
// ---------------------------------------------------------------------------

async fn ensure_read_statefulset(
    client: &Client,
    node: &StellarNode,
    config: &ReadReplicaConfig,
    enable_mtls: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
    let name = statefulset_name(node);

    let ss = build_read_statefulset(node, config, enable_mtls);
    api.patch(
        &name,
        &PatchParams::apply(FIELD_MANAGER).force(),
        &Patch::Apply(&ss),
    )
    .await?;

    info!("Read StatefulSet ensured: {}/{}", namespace, name);
    Ok(())
}

fn build_read_statefulset(
    node: &StellarNode,
    config: &ReadReplicaConfig,
    enable_mtls: bool,
) -> StatefulSet {
    let labels = read_pool_labels(node);
    let name = statefulset_name(node);

    let replicas = if node.spec.suspended {
        0
    } else {
        config.replicas
    };

    // Add carbon-aware scheduling annotation for read replicas
    let mut annotations = std::collections::BTreeMap::new();
    annotations.insert(
        "stellar.org/carbon-aware".to_string(),
        "enabled".to_string(),
    );

    StatefulSet {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: node.namespace(),
            labels: Some(labels.clone()),
            annotations: Some(annotations),
            owner_references: Some(vec![super::resources::owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(StatefulSetSpec {
            replicas: Some(replicas),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            // Headless service name for stable pod DNS (pod-0.name.ns.svc…)
            service_name: name.clone(),
            template: build_read_pod_template(node, config, &labels, enable_mtls),
            ..Default::default()
        }),
        status: None,
    }
}

// ---------------------------------------------------------------------------
// Service (ClusterIP)
// ---------------------------------------------------------------------------

async fn ensure_read_service(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = service_name(node);

    let svc = build_read_service(node);
    api.patch(
        &name,
        &PatchParams::apply(FIELD_MANAGER).force(),
        &Patch::Apply(&svc),
    )
    .await?;

    info!(
        "Read Service ensured: {}/{} → {}",
        namespace,
        name,
        read_pool_endpoint(node)
    );
    Ok(())
}

fn build_read_service(node: &StellarNode) -> Service {
    let labels = read_pool_labels(node);
    let name = service_name(node);

    Service {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: node.namespace(),
            labels: Some(labels.clone()),
            // Annotation so operators know this is a read-pool endpoint
            annotations: Some(BTreeMap::from([(
                "stellar.org/read-pool".to_string(),
                "true".to_string(),
            )])),
            owner_references: Some(vec![super::resources::owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            // ClusterIP (default) — stable internal DNS name
            type_: Some("ClusterIP".to_string()),
            // Select only read-replica pods via the role label
            selector: Some(labels),
            ports: Some(vec![
                ServicePort {
                    name: Some("http".to_string()),
                    port: STELLAR_CORE_HTTP_PORT,
                    target_port: Some(IntOrString::Int(STELLAR_CORE_HTTP_PORT)),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
                ServicePort {
                    name: Some("peer".to_string()),
                    port: STELLAR_CORE_PEER_PORT,
                    target_port: Some(IntOrString::Int(STELLAR_CORE_PEER_PORT)),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        status: None,
    }
}

// ---------------------------------------------------------------------------
// HorizontalPodAutoscaler (v2)
// ---------------------------------------------------------------------------

async fn ensure_read_hpa(
    client: &Client,
    node: &StellarNode,
    config: &ReadReplicaConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client.clone(), &namespace);
    let name = hpa_name(node);

    let hpa = build_read_hpa(node, config);
    api.patch(
        &name,
        &PatchParams::apply(FIELD_MANAGER).force(),
        &Patch::Apply(&hpa),
    )
    .await?;

    info!("Read HPA ensured: {}/{}", namespace, name);
    Ok(())
}

fn build_read_hpa(node: &StellarNode, config: &ReadReplicaConfig) -> HorizontalPodAutoscaler {
    let name = hpa_name(node);
    let ss_name = statefulset_name(node);

    // Scale between configured replicas (min) and 3× that (max), capped at 20
    let min_replicas = config.replicas.max(1);
    let max_replicas = (config.replicas * 3).min(20);

    HorizontalPodAutoscaler {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: node.namespace(),
            labels: Some(read_pool_labels(node)),
            owner_references: Some(vec![super::resources::owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(HorizontalPodAutoscalerSpec {
            scale_target_ref: CrossVersionObjectReference {
                api_version: Some("apps/v1".to_string()),
                kind: "StatefulSet".to_string(),
                name: ss_name,
            },
            min_replicas: Some(min_replicas),
            max_replicas,
            metrics: Some(vec![
                // Scale on CPU utilization
                MetricSpec {
                    type_: "Resource".to_string(),
                    resource: Some(k8s_openapi::api::autoscaling::v2::ResourceMetricSource {
                        name: "cpu".to_string(),
                        target: MetricTarget {
                            type_: "Utilization".to_string(),
                            average_utilization: Some(DEFAULT_CPU_TARGET),
                            ..Default::default()
                        },
                    }),
                    ..Default::default()
                },
                // Scale on memory utilization
                MetricSpec {
                    type_: "Resource".to_string(),
                    resource: Some(k8s_openapi::api::autoscaling::v2::ResourceMetricSource {
                        name: "memory".to_string(),
                        target: MetricTarget {
                            type_: "Utilization".to_string(),
                            average_utilization: Some(DEFAULT_MEMORY_TARGET),
                            ..Default::default()
                        },
                    }),
                    ..Default::default()
                },
            ]),
            behavior: None,
        }),
        status: None,
    }
}

// ---------------------------------------------------------------------------
// ConfigMap (startup script)
// ---------------------------------------------------------------------------

async fn ensure_read_config_map(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = configmap_name(node);

    let cm = build_read_config_map(node);
    api.patch(
        &name,
        &PatchParams::apply(FIELD_MANAGER).force(),
        &Patch::Apply(&cm),
    )
    .await?;

    Ok(())
}

fn build_read_config_map(node: &StellarNode) -> ConfigMap {
    let name = configmap_name(node);
    let mut data = BTreeMap::new();

    let mut script = String::new();
    script.push_str("#!/bin/bash\n");
    script.push_str("set -e\n\n");
    script.push_str("ORDINAL=${HOSTNAME##*-}\n");
    script.push_str("echo \"Starting read replica $ORDINAL\"\n\n");

    if let Some(vc) = &node.spec.validator_config {
        if !vc.history_archive_urls.is_empty() {
            script.push_str("ARCHIVES=(\n");
            for url in &vc.history_archive_urls {
                script.push_str(&format!("  \"{}\"\n", url));
            }
            script.push_str(")\n");
            script.push_str("ARCHIVE_COUNT=${#ARCHIVES[@]}\n");
            script.push_str("INDEX=$((ORDINAL % ARCHIVE_COUNT))\n");
            script.push_str("SELECTED_ARCHIVE=${ARCHIVES[$INDEX]}\n");
            script.push_str("echo \"Selected archive shard: $SELECTED_ARCHIVE\"\n\n");

            script.push_str("cat > /etc/stellar/stellar-core.cfg <<EOF\n");
            script.push_str("HTTP_PORT=11626\n");
            script.push_str("PUBLIC_HTTP_PORT=true\n");
            script.push_str("RUN_STANDALONE=false\n");
            script.push_str(&format!(
                "NETWORK_PASSPHRASE=\"{}\"\n",
                node.spec.network.passphrase()
            ));
            script.push_str("[HISTORY.h1]\n");
            script.push_str("get=\"curl -sf $SELECTED_ARCHIVE/{0} -o {1}\"\n\n");

            let validator_svc = format!(
                "{}.{}.svc.cluster.local",
                node.name_any(),
                node.namespace().unwrap_or_else(|| "default".to_string())
            );
            script.push_str("[PREFERRED_PEERS]\n");
            script.push_str(&format!("\"{}\"\n", validator_svc));
            script.push_str("EOF\n");
        }
    }

    script.push_str("\nexec /usr/bin/stellar-core run --conf /etc/stellar/stellar-core.cfg\n");
    data.insert("startup.sh".to_string(), script);

    ConfigMap {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: node.namespace(),
            labels: Some(super::resources::standard_labels(node)),
            owner_references: Some(vec![super::resources::owner_reference(node)]),
            ..Default::default()
        },
        data: Some(data),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Pod template
// ---------------------------------------------------------------------------

fn build_read_pod_template(
    node: &StellarNode,
    config: &ReadReplicaConfig,
    labels: &BTreeMap<String, String>,
    _enable_mtls: bool,
) -> PodTemplateSpec {
    let image = node.spec.container_image();
    let cm_name = configmap_name(node);

    let mut requests = BTreeMap::new();
    requests.insert(
        "cpu".to_string(),
        Quantity(config.resources.requests.cpu.clone()),
    );
    requests.insert(
        "memory".to_string(),
        Quantity(config.resources.requests.memory.clone()),
    );

    let mut limits = BTreeMap::new();
    limits.insert(
        "cpu".to_string(),
        Quantity(config.resources.limits.cpu.clone()),
    );
    limits.insert(
        "memory".to_string(),
        Quantity(config.resources.limits.memory.clone()),
    );

    PodTemplateSpec {
        metadata: Some(ObjectMeta {
            labels: Some(labels.clone()),
            ..Default::default()
        }),
        spec: Some(PodSpec {
            containers: vec![Container {
                name: "stellar-core".to_string(),
                image: Some(image),
                command: Some(vec![
                    "/bin/bash".to_string(),
                    "/config/startup.sh".to_string(),
                ]),
                resources: Some(k8s_openapi::api::core::v1::ResourceRequirements {
                    requests: Some(requests),
                    limits: Some(limits),
                    ..Default::default()
                }),
                ports: Some(vec![
                    ContainerPort {
                        name: Some("http".to_string()),
                        container_port: STELLAR_CORE_HTTP_PORT,
                        ..Default::default()
                    },
                    ContainerPort {
                        name: Some("peer".to_string()),
                        container_port: STELLAR_CORE_PEER_PORT,
                        ..Default::default()
                    },
                ]),
                volume_mounts: Some(vec![VolumeMount {
                    name: "config".to_string(),
                    mount_path: "/config".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            volumes: Some(vec![Volume {
                name: "config".to_string(),
                config_map: Some(k8s_openapi::api::core::v1::ConfigMapVolumeSource {
                    name: Some(cm_name),
                    default_mode: Some(0o755),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            topology_spread_constraints: Some(super::resources::build_topology_spread_constraints(
                &node.spec,
                &node.name_any(),
            )),
            ..Default::default()
        }),
    }
}

// ---------------------------------------------------------------------------
// Cleanup — called when read_replica_config is removed from spec
// ---------------------------------------------------------------------------

/// Delete all read-pool resources: StatefulSet, Service, HPA, ConfigMap.
pub async fn delete_read_pool(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    // StatefulSet
    let ss_api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
    match ss_api
        .delete(&statefulset_name(node), &DeleteParams::default())
        .await
    {
        Ok(_) => info!("Deleted read StatefulSet: {}", statefulset_name(node)),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => warn!("Failed to delete read StatefulSet: {:?}", e),
    }

    // Service
    let svc_api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    match svc_api
        .delete(&service_name(node), &DeleteParams::default())
        .await
    {
        Ok(_) => info!("Deleted read Service: {}", service_name(node)),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => warn!("Failed to delete read Service: {:?}", e),
    }

    // HPA
    let hpa_api: Api<HorizontalPodAutoscaler> = Api::namespaced(client.clone(), &namespace);
    match hpa_api
        .delete(&hpa_name(node), &DeleteParams::default())
        .await
    {
        Ok(_) => info!("Deleted read HPA: {}", hpa_name(node)),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => warn!("Failed to delete read HPA: {:?}", e),
    }

    // ConfigMap
    let cm_api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    match cm_api
        .delete(&configmap_name(node), &DeleteParams::default())
        .await
    {
        Ok(_) => info!("Deleted read ConfigMap: {}", configmap_name(node)),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => warn!("Failed to delete read ConfigMap: {:?}", e),
    }

    Ok(())
}
