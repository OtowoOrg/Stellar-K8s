use k8s_openapi::api::apps::v1::{StatefulSet, StatefulSetSpec};
use k8s_openapi::api::core::v1::{
    ConfigMap, Container, ContainerPort, EnvVar, KeyToPath, PodSpec, PodTemplateSpec,
    Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::{
    api::{Api, Patch, PatchParams},
    Client, Resource, ResourceExt,
};
use std::collections::BTreeMap;
use tracing::{info, instrument};

use crate::crd::{StellarNode, ReadReplicaConfig, NodeType};
use crate::error::Result;

/// Ensure the read-only replica pool exists and is configured correctly
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
    
    // Ensure the ConfigMap with startup script exists
    ensure_read_config_map(client, node).await?;

    let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
    let name = format!("{}-read", node.name_any());

    let statefulset = build_read_statefulset(node, config, enable_mtls);

    let patch = Patch::Apply(&statefulset);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    info!("Read-only replica pool (StatefulSet) ensured for {}/{}", namespace, name);

    Ok(())
}

async fn delete_read_pool(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    
    // Delete StatefulSet
    let ss_api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
    let ss_name = format!("{}-read", node.name_any());
    if ss_api.get(&ss_name).await.is_ok() {
        ss_api.delete(&ss_name, &kube::api::DeleteParams::default()).await?;
        info!("Deleted read-only replica pool: {}", ss_name);
    }

    // Delete ConfigMap
    let cm_api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let cm_name = format!("{}-read-config", node.name_any());
    if cm_api.get(&cm_name).await.is_ok() {
        cm_api.delete(&cm_name, &kube::api::DeleteParams::default()).await?;
    }

    Ok(())
}

async fn ensure_read_config_map(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = format!("{}-read-config", node.name_any());

    let cm = build_read_config_map(node);
    let patch = Patch::Apply(&cm);
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator").force(),
        &patch,
    )
    .await?;

    Ok(())
}

fn build_read_config_map(node: &StellarNode) -> ConfigMap {
    let name = format!("{}-read-config", node.name_any());
    let mut data = BTreeMap::new();

    // Generate the startup script that configures stellar-core based on sharding
    let mut script = String::new();
    script.push_str("#!/bin/bash\n");
    script.push_str("set -e\n\n");
    
    // Extract ordinal from hostname (e.g., node-read-0 -> 0)
    script.push_str("ORDINAL=${HOSTNAME##*-}\n");
    script.push_str("echo \"Starting read replica $ORDINAL\"\n\n");

    // Archives logic
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
            
            // Generate config content
            script.push_str("cat > /etc/stellar/stellar-core.cfg <<EOF\n");
            script.push_str("HTTP_PORT=11626\n");
            script.push_str("PUBLIC_HTTP_PORT=true\n");
            script.push_str("RUN_STANDALONE=false\n");
            script.push_str(&format!("NETWORK_PASSPHRASE=\"{}\"\n", node.spec.network.passphrase()));
            
            // Add history config
            script.push_str("[HISTORY.h1]\n");
            script.push_str("get=\"curl -sf $SELECTED_ARCHIVE/{0} -o {1}\"\n\n");
            
            // Point to the main validator as a preferred peer
            // We assume the main validator service is reachable at <node-name>.<namespace>.svc.cluster.local
            let validator_svc = format!("{}.{}.svc.cluster.local", node.name_any(), node.namespace().unwrap_or("default".to_string()));
            script.push_str("[PREFERRED_PEERS]\n");
            script.push_str(&format!("\"{}\"\n", validator_svc)); // This needs the peer port 11625
            
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

fn build_read_statefulset(
    node: &StellarNode,
    config: &ReadReplicaConfig,
    enable_mtls: bool,
) -> StatefulSet {
    let mut labels = super::resources::standard_labels(node);
    labels.insert("stellar.org/role".to_string(), "read-replica".to_string());
    
    let name = format!("{}-read", node.name_any());
    
    let replicas = if node.spec.suspended {
        0
    } else {
        config.replicas
    };

    StatefulSet {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: node.namespace(),
            labels: Some(labels.clone()),
            owner_references: Some(vec![super::resources::owner_reference(node)]),
            ..Default::default()
        },
        spec: Some(StatefulSetSpec {
            replicas: Some(replicas),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            service_name: name.clone(), // Headless service for stable networking
            template: build_read_pod_template(node, config, &labels, enable_mtls),
            ..Default::default()
        }),
        status: None,
    }
}

fn build_read_pod_template(
    node: &StellarNode,
    config: &ReadReplicaConfig,
    labels: &BTreeMap<String, String>,
    _enable_mtls: bool,
) -> PodTemplateSpec {
    let image = node.spec.container_image();
    let container_name = "stellar-core";
    let cm_name = format!("{}-read-config", node.name_any());

    // Manual conversion of resources since Into is not implemented
    let mut requests = BTreeMap::new();
    requests.insert("cpu".to_string(), k8s_openapi::apimachinery::pkg::api::resource::Quantity(config.resources.requests.cpu.clone()));
    requests.insert("memory".to_string(), k8s_openapi::apimachinery::pkg::api::resource::Quantity(config.resources.requests.memory.clone()));
    
    let mut limits = BTreeMap::new();
    limits.insert("cpu".to_string(), k8s_openapi::apimachinery::pkg::api::resource::Quantity(config.resources.limits.cpu.clone()));
    limits.insert("memory".to_string(), k8s_openapi::apimachinery::pkg::api::resource::Quantity(config.resources.limits.memory.clone()));

    let resources = k8s_openapi::api::core::v1::ResourceRequirements {
        requests: Some(requests),
        limits: Some(limits),
        ..Default::default()
    };

    PodTemplateSpec {
        metadata: Some(ObjectMeta {
            labels: Some(labels.clone()),
            ..Default::default()
        }),
        spec: Some(PodSpec {
            containers: vec![Container {
                name: container_name.to_string(),
                image: Some(image),
                command: Some(vec!["/bin/bash".to_string(), "/config/startup.sh".to_string()]),
                resources: Some(resources),
                ports: Some(vec![
                    ContainerPort {
                        name: Some("http".to_string()),
                        container_port: 11626,
                        ..Default::default()
                    },
                    ContainerPort {
                        name: Some("peer".to_string()),
                        container_port: 11625,
                        ..Default::default()
                    },
                ]),
                volume_mounts: Some(vec![
                    VolumeMount {
                        name: "config".to_string(),
                        mount_path: "/config".to_string(),
                        ..Default::default()
                    }
                ]),
                ..Default::default()
            }],
            volumes: Some(vec![
                Volume {
                    name: "config".to_string(),
                    config_map: Some(k8s_openapi::api::core::v1::ConfigMapVolumeSource {
                        name: Some(cm_name),
                        default_mode: Some(0o755),
                         ..Default::default()
                    }),
                    ..Default::default()
                }
            ]),
            ..Default::default()
        }),
    }
}
