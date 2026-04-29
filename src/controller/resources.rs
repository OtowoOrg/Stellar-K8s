//! Kubernetes resource builders for StellarNode
//!
//! This module creates and manages the underlying Kubernetes resources
//! (Deployments, StatefulSets, Services, PVCs, ConfigMaps) for each StellarNode.

use crate::controller::resource_meta::merge_resource_meta;

// *** NEW: import kms_secret so we can accept SeedInjectionSpec ***
use super::kms_secret;
use super::label_propagation::LabelPropagator;

use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, StatefulSet, StatefulSetSpec};
use k8s_openapi::api::autoscaling::v2::{
    CrossVersionObjectReference, HPAScalingPolicy, HPAScalingRules, HorizontalPodAutoscaler,
    HorizontalPodAutoscalerBehavior, HorizontalPodAutoscalerSpec, MetricIdentifier, MetricSpec,
    MetricTarget, ObjectMetricSource,
};
use k8s_openapi::api::core::v1::{
    Affinity, Capabilities, ConfigMap, Container, ContainerPort, EnvVar, EnvVarSource,
    PersistentVolumeClaim, PersistentVolumeClaimSpec, PodAffinityTerm, PodAntiAffinity,
    PodSecurityContext, PodSpec, PodTemplateSpec, ResourceRequirements as K8sResources,
    SeccompProfile, SecretKeySelector, SecurityContext, Service, ServicePort, ServiceSpec,
    TypedLocalObjectReference, Volume, VolumeMount, VolumeResourceRequirements,
    WeightedPodAffinityTerm,
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

use crate::crd::types::{PodAntiAffinityStrength, ReplicationRole, RolloutStrategyType};
use crate::crd::{
    BackupConfiguration, BarmanObjectStore, BootstrapConfiguration, Cluster, ClusterSpec,
    ExternalCluster, HistoryMode, HsmProvider, IngressConfig, InitDbConfiguration, KeySource,
    ManagedDatabaseConfig, MonitoringConfiguration, NetworkPolicyConfig, NodeType, PgBouncerSpec,
    Pooler, PoolerCluster, PoolerSpec, PostgresConfiguration, RecoveryConfiguration,
    ReplicaConfiguration, S3Credentials, SecretKeySelector as CnpgSecretKeySelector, StellarNode,
    StellarNodeSpec, StorageConfiguration, WalBackupConfiguration,
};
use crate::error::{Error, Result};
use crate::scheduler::scoring::extract_peer_names_from_toml;

/// Get the standard labels for a StellarNode's resources
pub(crate) fn standard_labels(node: &StellarNode) -> BTreeMap<String, String> {
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
    labels.insert(
        "stellar-network".to_string(),
        node.spec
            .network
            .scheduling_label_value(&node.spec.custom_network_passphrase),
    );
    labels
}

/// Create an OwnerReference for garbage collection
pub(crate) fn owner_reference(node: &StellarNode) -> OwnerReference {
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
pub(crate) fn resource_name(node: &StellarNode, suffix: &str) -> String {
    format!("{}-{}", node.name_any(), suffix)
}

/// Apply a [`ProbeOverride`] on top of an optional base [`k8s_openapi::api::core::v1::Probe`].
/// Apply a [`ProbeOverride`] on top of an optional base [`k8s_openapi::api::core::v1::Probe`].
///
/// If `override_cfg` is `None`, the base probe is returned unchanged.
/// If `base` is `None` and `override_cfg` is `Some`, a minimal probe shell is created and the
/// overrides are applied so the operator can still honour user-supplied thresholds even when no
/// default probe is configured.
pub(crate) fn apply_probe_override_pub(
    base: Option<k8s_openapi::api::core::v1::Probe>,
    override_cfg: Option<&crate::crd::types::ProbeOverride>,
) -> Option<k8s_openapi::api::core::v1::Probe> {
    apply_probe_override(base, override_cfg)
}

fn apply_probe_override(
    base: Option<k8s_openapi::api::core::v1::Probe>,
    override_cfg: Option<&crate::crd::types::ProbeOverride>,
) -> Option<k8s_openapi::api::core::v1::Probe> {
    let cfg = override_cfg?;
    let mut probe = base.unwrap_or_default();
    if let Some(v) = cfg.initial_delay_seconds {
        probe.initial_delay_seconds = Some(v);
    }
    if let Some(v) = cfg.period_seconds {
        probe.period_seconds = Some(v);
    }
    if let Some(v) = cfg.timeout_seconds {
        probe.timeout_seconds = Some(v);
    }
    if let Some(v) = cfg.success_threshold {
        probe.success_threshold = Some(v);
    }
    if let Some(v) = cfg.failure_threshold {
        probe.failure_threshold = Some(v);
    }
    Some(probe)
}

/// Create PostParams with dry-run support
fn post_params(dry_run: bool) -> PostParams {
    if dry_run {
        PostParams {
            dry_run: true,
            ..Default::default()
        }
    } else {
        PostParams::default()
    }
}

/// Create PatchParams with dry-run support
fn patch_params(dry_run: bool) -> PatchParams {
    let mut params = PatchParams::apply("stellar-operator").force();
    if dry_run {
        params.dry_run = true;
    }
    params
}

/// Create DeleteParams with dry-run support
fn delete_params(dry_run: bool) -> DeleteParams {
    if dry_run {
        DeleteParams {
            dry_run: true,
            ..Default::default()
        }
    } else {
        DeleteParams::default()
    }
}

// ============================================================================
// PersistentVolumeClaim
// ============================================================================

/// Ensure a PersistentVolumeClaim exists for the node
#[instrument(skip(client, node, propagated_labels), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_pvc(
    client: &Client,
    node: &StellarNode,
    propagated_labels: &BTreeMap<String, String>,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "data");

    // Dynamic resolution of storage class for local mode.
    let mut has_local_path = false;
    let mut has_local_storage = false;
    if node.spec.storage.mode == crate::crd::types::StorageMode::Local
        && node.spec.storage.storage_class.is_empty()
    {
        let sc_api: Api<k8s_openapi::api::storage::v1::StorageClass> = Api::all(client.clone());
        has_local_path = sc_api.get("local-path").await.is_ok();
        has_local_storage = sc_api.get("local-storage").await.is_ok();
    }
    let resolved_storage_class = resolve_pvc_storage_class(node, has_local_path, has_local_storage);
    if node.spec.storage.mode == crate::crd::types::StorageMode::Local
        && resolved_storage_class.is_empty()
    {
        warn!(
            "Local StorageMode requested but no storageClass provided and local-path/local-storage auto-detection failed."
        );
    }

    // Fetch existing resource labels for stale-label removal
    let existing_labels = match api.get(&name).await {
        Ok(existing) => existing.metadata.labels.clone().unwrap_or_default(),
        Err(kube::Error::Api(e)) if e.code == 404 => BTreeMap::new(),
        Err(e) => return Err(Error::KubeError(e)),
    };

    let mut pvc = build_pvc(node, resolved_storage_class);

    // Apply label propagation: merge propagated labels, then remove stale ones
    let base_labels = pvc.metadata.labels.clone().unwrap_or_default();
    let merged = LabelPropagator::merge_onto(&base_labels, propagated_labels);
    let final_labels =
        LabelPropagator::remove_stale_labels(&merged, propagated_labels, &existing_labels);
    pvc.metadata.labels = Some(final_labels);

    match api.get(&name).await {
        Ok(existing) => {
            if pvc_needs_update(&existing, &pvc) {
                info!("Updating PVC {}", name);
                api.patch(&name, &patch_params(dry_run), &Patch::Apply(&pvc))
                    .await?;
            } else {
                info!("PVC {} already exists and is up-to-date", name);
            }
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            info!("Creating PVC {}", name);
            api.create(&post_params(dry_run), &pvc).await?;
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

fn resolve_pvc_storage_class(
    node: &StellarNode,
    has_local_path: bool,
    has_local_storage: bool,
) -> String {
    let resolved_storage_class = node.spec.storage.storage_class.clone();
    if node.spec.storage.mode != crate::crd::types::StorageMode::Local
        || !resolved_storage_class.is_empty()
    {
        return resolved_storage_class;
    }

    if has_local_path {
        "local-path".to_string()
    } else if has_local_storage {
        "local-storage".to_string()
    } else {
        String::new()
    }
}

fn pvc_needs_update(existing: &PersistentVolumeClaim, desired: &PersistentVolumeClaim) -> bool {
    existing.spec != desired.spec
        || existing.metadata.labels != desired.metadata.labels
        || existing.metadata.annotations != desired.metadata.annotations
}

fn build_pvc(node: &StellarNode, storage_class_name: String) -> PersistentVolumeClaim {
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

    // When restoring from a VolumeSnapshot, set dataSource so the PVC is populated from the snapshot.
    // Priority: spec.storage.snapshotRef.volumeSnapshotName > spec.restoreFromSnapshot.volumeSnapshotName
    let data_source = node
        .spec
        .storage
        .snapshot_ref
        .as_ref()
        .and_then(|r| r.volume_snapshot_name.as_deref())
        .or_else(|| {
            node.spec
                .restore_from_snapshot
                .as_ref()
                .map(|r| r.volume_snapshot_name.as_str())
        })
        .map(|snap_name| TypedLocalObjectReference {
            api_group: Some("snapshot.storage.k8s.io".to_string()),
            kind: "VolumeSnapshot".to_string(),
            name: snap_name.to_string(),
        });

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
            storage_class_name: if storage_class_name.is_empty() {
                None
            } else {
                Some(storage_class_name)
            },
            data_source,
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
pub async fn delete_pvc(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "data");

    match api.delete(&name, &delete_params(dry_run)).await {
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
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "config");

    let cm = build_config_map(node, quorum_override, enable_mtls);

    let patch = Patch::Apply(&cm);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    Ok(())
}

pub(crate) fn build_config_map(
    node: &StellarNode,
    quorum_override: Option<crate::controller::vsl::QuorumSet>,
    enable_mtls: bool,
) -> ConfigMap {
    let labels = standard_labels(node);
    let name = resource_name(node, "config");

    let mut data = BTreeMap::new();

    data.insert(
        "NETWORK_PASSPHRASE".to_string(),
        node.spec.network_passphrase().to_string(),
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

    if let Some(ebpf_cfg) = &node.spec.ebpf_config {
        if ebpf_cfg.enabled {
            let mut exporter_yaml = String::from("programs:\n");

            if ebpf_cfg.monitor_write_latency {
                exporter_yaml.push_str(
                    r#"  - name: write_latency
    metrics:
      counters:
        - name: ebpf_write_latency_seconds_sum
          help: Total write latency in seconds
          labels:
            - name: process
              size: 16
              decoding: string
    tracepoints:
      sys_enter_write:
        code: |
          // BPF code to track write latency
          // This is a simplified placeholder for the actual BPF C code
          bpf_trace_printk("write enter\n");
"#,
                );
            }

            if ebpf_cfg.monitor_tcp_retransmits {
                exporter_yaml.push_str(
                    r#"  - name: tcp_retransmits
    metrics:
      counters:
        - name: ebpf_tcp_retransmits_total
          help: Total TCP retransmits
          labels:
            - name: process
              size: 16
              decoding: string
    tracepoints:
      tcp_retransmit_skb:
        code: |
          // BPF code to track TCP retransmits
          bpf_trace_printk("tcp retransmit\n");
"#,
                );
            }

            if ebpf_cfg.monitor_write_latency || ebpf_cfg.monitor_tcp_retransmits {
                data.insert("ebpf-exporter.yaml".to_string(), exporter_yaml);
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
pub async fn delete_config_map(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "config");

    match api.delete(&name, &delete_params(dry_run)).await {
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
#[instrument(skip(client, node, propagated_labels), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_deployment(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
    propagated_labels: &BTreeMap<String, String>,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    // Fetch existing resource labels for stale-label removal
    let existing_labels = match api.get(&name).await {
        Ok(existing) => existing.metadata.labels.clone().unwrap_or_default(),
        Err(kube::Error::Api(e)) if e.code == 404 => BTreeMap::new(),
        Err(e) => return Err(Error::KubeError(e)),
    };

    let mut deployment = build_deployment(node, enable_mtls);

    // Apply label propagation: merge propagated labels, then remove stale ones
    let base_labels = deployment.metadata.labels.clone().unwrap_or_default();
    let merged = LabelPropagator::merge_onto(&base_labels, propagated_labels);
    let final_labels =
        LabelPropagator::remove_stale_labels(&merged, propagated_labels, &existing_labels);
    deployment.metadata.labels = Some(final_labels);

    let patch = Patch::Apply(&deployment);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    Ok(())
}

/// Ensure a canary Deployment exists if needed
pub async fn ensure_canary_deployment(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
    dry_run: bool,
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
        let mut labels = standard_labels(&canary_node);
        labels.insert("stellar.org/rollout-type".to_string(), "canary".to_string());
        spec.template.metadata.as_mut().unwrap().labels = Some(labels.clone());
        spec.selector.match_labels = Some(labels.clone());

        let meta = &mut deployment.metadata;
        meta.labels = Some(labels);
    }

    let patch = Patch::Apply(&deployment);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    Ok(())
}

fn build_deployment(node: &StellarNode, enable_mtls: bool) -> Deployment {
    let mut labels = standard_labels(node);
    let name = node.name_any();

    if node.spec.node_type == NodeType::Horizon
        && node.spec.strategy.strategy_type == RolloutStrategyType::BlueGreen
    {
        labels.insert("deployment-color".to_string(), "blue".to_string());
    }

    let mut replicas = if node.spec.suspended {
        0
    } else {
        node.spec.replicas
    };

    // If node is Passive in a replication setup, scale to 0 to prevent DB write conflicts
    // while the managed database is in read-only replica mode.
    if let Some(repl_cfg) = &node.spec.replication_config {
        if repl_cfg.enabled && repl_cfg.role == ReplicationRole::Passive {
            replicas = 0;
        }
    }

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
#[instrument(skip(client, node, propagated_labels), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_statefulset(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
    seed_injection: Option<&kms_secret::SeedInjectionSpec>,
    propagated_labels: &BTreeMap<String, String>,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    // Fetch existing resource labels for stale-label removal
    let existing_labels = match api.get(&name).await {
        Ok(existing) => existing.metadata.labels.clone().unwrap_or_default(),
        Err(kube::Error::Api(e)) if e.code == 404 => BTreeMap::new(),
        Err(e) => return Err(Error::KubeError(e)),
    };

    // *** Pass seed_injection down to the builder ***
    let mut statefulset = build_statefulset(node, enable_mtls, seed_injection);

    // Apply label propagation: merge propagated labels, then remove stale ones
    let base_labels = statefulset.metadata.labels.clone().unwrap_or_default();
    let merged = LabelPropagator::merge_onto(&base_labels, propagated_labels);
    let final_labels =
        LabelPropagator::remove_stale_labels(&merged, propagated_labels, &existing_labels);
    statefulset.metadata.labels = Some(final_labels);

    let patch = Patch::Apply(&statefulset);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

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

    let mut replicas = if node.spec.suspended { 0 } else { 1 };

    // If node is Passive in a replication setup, scale to 0 to prevent DB write conflicts
    // while the managed database is in read-only replica mode.
    if let Some(repl_cfg) = &node.spec.replication_config {
        if repl_cfg.enabled && repl_cfg.role == ReplicationRole::Passive {
            replicas = 0;
        }
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
            // *** Pass seed_injection into pod template builder ***
            template: build_pod_template(node, &labels, enable_mtls, seed_injection),
            ..Default::default()
        }),
        status: None,
    }
}

/// Delete the workload (Deployment or StatefulSet) for a node
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_workload(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    match node.spec.node_type {
        NodeType::Validator => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
            match api.delete(&name, &delete_params(dry_run)).await {
                Ok(_) => info!("Deleted StatefulSet {}", name),
                Err(kube::Error::Api(e)) if e.code == 404 => {
                    warn!("StatefulSet {} not found", name);
                }
                Err(e) => return Err(Error::KubeError(e)),
            }
        }
        _ => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
            match api.delete(&name, &delete_params(dry_run)).await {
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
#[instrument(skip(client, node, propagated_labels), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_service(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
    propagated_labels: &BTreeMap<String, String>,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    // Fetch existing resource labels for stale-label removal
    let existing_labels = match api.get(&name).await {
        Ok(existing) => existing.metadata.labels.clone().unwrap_or_default(),
        Err(kube::Error::Api(e)) if e.code == 404 => BTreeMap::new(),
        Err(e) => return Err(Error::KubeError(e)),
    };

    let mut service = build_service(node, enable_mtls);

    // Apply label propagation: merge propagated labels, then remove stale ones
    let base_labels = service.metadata.labels.clone().unwrap_or_default();
    let merged = LabelPropagator::merge_onto(&base_labels, propagated_labels);
    let final_labels =
        LabelPropagator::remove_stale_labels(&merged, propagated_labels, &existing_labels);
    service.metadata.labels = Some(final_labels);

    let patch = Patch::Apply(&service);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    Ok(())
}

/// Ensure a canary Service exists if needed
pub async fn ensure_canary_service(
    client: &Client,
    node: &StellarNode,
    enable_mtls: bool,
    dry_run: bool,
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
        let mut labels = standard_labels(node);
        labels.insert("stellar.org/rollout-type".to_string(), "canary".to_string());
        spec.selector = Some(labels.clone());

        let meta = &mut service.metadata;
        meta.labels = Some(labels);
    }

    let patch = Patch::Apply(&service);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    Ok(())
}

fn build_service(node: &StellarNode, enable_mtls: bool) -> Service {
    let labels = standard_labels(node);
    let name = node.name_any();

    let mut annotations = BTreeMap::new();

    // Collect ExternalDNS config from ValidatorConfig or LoadBalancerConfig
    let mut dns_configs = Vec::new();
    if let Some(vc) = &node.spec.validator_config {
        if let Some(dns) = &vc.external_dns {
            dns_configs.push(dns);
        }
    }
    if let Some(lb) = &node.spec.load_balancer {
        if let Some(dns) = &lb.external_dns {
            dns_configs.push(dns);
        }
    }

    if !dns_configs.is_empty() {
        // Use the first one found, prioritize ValidatorConfig
        let dns_config = dns_configs[0];
        let mut hostnames = vec![dns_config.hostname.clone()];

        // Automatically generate _stellar-peering._tcp SRV record for validators
        if node.spec.node_type == NodeType::Validator {
            hostnames.push(format!("_stellar-peering._tcp.{}", dns_config.hostname));
        }

        annotations.insert(
            "external-dns.alpha.kubernetes.io/hostname".to_string(),
            hostnames.join(", "),
        );
        annotations.insert(
            "external-dns.alpha.kubernetes.io/ttl".to_string(),
            dns_config.ttl.to_string(),
        );
        if let Some(provider) = &dns_config.provider {
            annotations.insert(
                "external-dns.alpha.kubernetes.io/provider".to_string(),
                provider.clone(),
            );
        }
        if let Some(extra_annotations) = &dns_config.annotations {
            for (k, v) in extra_annotations {
                annotations.insert(k.clone(), v.clone());
            }
        }
    }

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
pub async fn delete_service(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    match api.delete(&name, &delete_params(dry_run)).await {
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
pub async fn ensure_cnpg_cluster(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let managed_db = match &node.spec.managed_database {
        Some(cfg) => cfg,
        None => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Cluster> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    let cluster = build_cnpg_cluster(node, managed_db);

    let patch = Patch::Apply(&cluster);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

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
            external_clusters: None,
            replica: None,
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
                    database: config
                        .database_name
                        .clone()
                        .unwrap_or_else(|| "stellar".to_string()),
                    owner: config
                        .username
                        .clone()
                        .unwrap_or_else(|| "stellar".to_string()),
                    secret: None,
                }),
                recovery: None,
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

    // Handle multi-region replication
    if let Some(repl_cfg) = &node.spec.replication_config {
        if repl_cfg.enabled && repl_cfg.role == ReplicationRole::Passive {
            let remote_name = format!("{}-primary", repl_cfg.remote_cluster_id);

            // Define external cluster pointing to the primary in the remote region
            let external_cluster = ExternalCluster {
                name: remote_name.clone(),
                connection_parameters: {
                    let mut p = BTreeMap::new();
                    p.insert(
                        "host".to_string(),
                        format!("{}.{}.svc", node.name_any(), repl_cfg.remote_cluster_id),
                    );
                    p.insert("user".to_string(), "stellar".to_string());
                    p.insert("dbname".to_string(), "stellar".to_string());
                    p.insert("sslmode".to_string(), "require".to_string());
                    p
                },
                password: CnpgSecretKeySelector {
                    name: format!("{}-app", node.name_any()),
                    key: "password".to_string(),
                },
            };

            cluster.spec.external_clusters = Some(vec![external_cluster]);

            // Configure bootstrap to recover from the external cluster
            if let Some(bootstrap) = &mut cluster.spec.bootstrap {
                bootstrap.initdb = None; // Cannot use initdb with recovery
                bootstrap.recovery = Some(RecoveryConfiguration {
                    source: remote_name.clone(),
                });
            }

            // Set as replica
            cluster.spec.replica = Some(ReplicaConfiguration {
                enabled: true,
                source: remote_name,
            });
        }
    }

    cluster
}

#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_cnpg_pooler(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
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
    api.patch(&name, &patch_params(dry_run), &patch).await?;

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
pub async fn delete_cnpg_resources(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
) -> Result<()> {
    if node.spec.managed_database.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    let pooler_api: Api<Pooler> = Api::namespaced(client.clone(), &namespace);
    let pooler_name = resource_name(node, "pooler");
    let _ = pooler_api
        .delete(&pooler_name, &delete_params(dry_run))
        .await;

    let cluster_api: Api<Cluster> = Api::namespaced(client.clone(), &namespace);
    let cluster_name = node.name_any();
    let _ = cluster_api
        .delete(&cluster_name, &delete_params(dry_run))
        .await;

    Ok(())
}

// ============================================================================
// Ingress — unchanged
// ============================================================================

#[allow(dead_code)]
pub async fn ensure_ingress(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
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

    api.patch(&name, &patch_params(dry_run), &Patch::Apply(&ingress))
        .await?;

    info!("Ingress ensured for {}/{}", namespace, name);

    if let Some(cfg) = node.spec.strategy.canary() {
        if node
            .status
            .as_ref()
            .and_then(|status| status.canary_version.as_ref())
            .is_some()
        {
            let canary_name = format!("{name}-canary");
            let mut canary_ingress = build_ingress(node, ingress_cfg);
            canary_ingress.metadata.name = Some(canary_name.clone());

            // Use the live canary weight from status if available (progressive stepping),
            // otherwise fall back to the configured initial weight.
            let effective_weight = node
                .status
                .as_ref()
                .and_then(|s| s.canary_weight)
                .unwrap_or(cfg.weight);

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
                effective_weight.to_string(),
            );
            annotations.insert(
                "traefik.ingress.kubernetes.io/service.weights".to_string(),
                format!("{}:{}", node.name_any(), effective_weight),
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
                &patch_params(dry_run),
                &Patch::Apply(&canary_ingress),
            )
            .await?;
            info!("Canary Ingress ensured for {}/{}", namespace, canary_name);

            // Istio VirtualService traffic splitting (when ingress class is "istio")
            if ingress_cfg
                .class_name
                .as_deref()
                .map(|c| c == "istio")
                .unwrap_or(false)
            {
                ensure_istio_canary_virtual_service(
                    client,
                    node,
                    ingress_cfg,
                    effective_weight,
                    dry_run,
                )
                .await?;
            }
        } else {
            let canary_name = format!("{name}-canary");
            let _ = api.delete(&canary_name, &delete_params(dry_run)).await;

            // Clean up Istio VirtualService if it exists
            if ingress_cfg
                .class_name
                .as_deref()
                .map(|c| c == "istio")
                .unwrap_or(false)
            {
                delete_istio_canary_virtual_service(client, node, dry_run).await?;
            }
        }
    }

    Ok(())
}

/// Ensure an Istio VirtualService that splits traffic between stable and canary services.
///
/// Creates a VirtualService using the Istio networking API via DynamicObject.
/// The stable service receives `(100 - weight)%` and the canary receives `weight%`.
async fn ensure_istio_canary_virtual_service(
    client: &Client,
    node: &StellarNode,
    ingress_cfg: &IngressConfig,
    canary_weight: i32,
    _dry_run: bool,
) -> Result<()> {
    use kube::api::DynamicObject;
    use kube::discovery::ApiResource;

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let stable_weight = 100 - canary_weight.clamp(0, 100);
    let vs_name = format!("{}-canary-vs", node.name_any());

    let hosts: Vec<String> = ingress_cfg.hosts.iter().map(|h| h.host.clone()).collect();

    let api_resource = ApiResource {
        group: "networking.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "networking.istio.io/v1beta1".to_string(),
        kind: "VirtualService".to_string(),
        plural: "virtualservices".to_string(),
    };

    let mut vs = DynamicObject::new(&vs_name, &api_resource).within(&namespace);
    vs.data = serde_json::json!({
        "spec": {
            "hosts": hosts,
            "http": [{
                "route": [
                    {
                        "destination": {
                            "host": node.name_any(),
                            "port": { "number": 8000 }
                        },
                        "weight": stable_weight
                    },
                    {
                        "destination": {
                            "host": format!("{}-canary", node.name_any()),
                            "port": { "number": 8000 }
                        },
                        "weight": canary_weight
                    }
                ]
            }]
        }
    });

    let api: kube::Api<DynamicObject> =
        kube::Api::namespaced_with(client.clone(), &namespace, &api_resource);

    match api
        .patch(
            &vs_name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Apply(&vs),
        )
        .await
    {
        Ok(_) => {
            info!(
                "Istio VirtualService {}/{} updated: stable={}% canary={}%",
                namespace, vs_name, stable_weight, canary_weight
            );
            Ok(())
        }
        Err(e) => {
            warn!(
                "Failed to apply Istio VirtualService (Istio may not be installed): {}",
                e
            );
            Ok(()) // Non-fatal — Nginx annotations still work
        }
    }
}

/// Delete the Istio VirtualService for a canary rollout.
async fn delete_istio_canary_virtual_service(
    client: &Client,
    node: &StellarNode,
    _dry_run: bool,
) -> Result<()> {
    use kube::api::DynamicObject;
    use kube::discovery::ApiResource;

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let vs_name = format!("{}-canary-vs", node.name_any());

    let api_resource = ApiResource {
        group: "networking.istio.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "networking.istio.io/v1beta1".to_string(),
        kind: "VirtualService".to_string(),
        plural: "virtualservices".to_string(),
    };

    let api: kube::Api<DynamicObject> =
        kube::Api::namespaced_with(client.clone(), &namespace, &api_resource);

    match api.delete(&vs_name, &DeleteParams::default()).await {
        Ok(_) => {
            info!("Deleted Istio VirtualService {}/{}", namespace, vs_name);
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => {
            warn!("Failed to delete Istio VirtualService: {}", e);
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

    if let Some(dns_config) = &config.external_dns {
        annotations.insert(
            "external-dns.alpha.kubernetes.io/hostname".to_string(),
            dns_config.hostname.clone(),
        );
        annotations.insert(
            "external-dns.alpha.kubernetes.io/ttl".to_string(),
            dns_config.ttl.to_string(),
        );
        if let Some(provider) = &dns_config.provider {
            annotations.insert(
                "external-dns.alpha.kubernetes.io/provider".to_string(),
                provider.clone(),
            );
        }
        if let Some(extra_annotations) = &dns_config.annotations {
            for (k, v) in extra_annotations {
                annotations.insert(k.clone(), v.clone());
            }
        }
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

pub async fn delete_ingress(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    if node.spec.ingress.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "ingress");

    match api.delete(&name, &delete_params(dry_run)).await {
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
        affinity: merge_workload_affinity(node),
        security_context: Some(PodSecurityContext {
            run_as_non_root: Some(true),
            run_as_user: Some(10000),
            run_as_group: Some(10000),
            fs_group: Some(10000),
            seccomp_profile: Some(SeccompProfile {
                localhost_profile: None,
                type_: "RuntimeDefault".to_string(),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    if node.spec.node_type == NodeType::Validator {
        if let Some(fs) = &node.spec.forensic_snapshot {
            if fs.enable_share_process_namespace {
                pod_spec.share_process_namespace = Some(true);
            }
        }
    }

    // Add Horizon database migration init container
    if let NodeType::Horizon = node.spec.node_type {
        if let Some(horizon_config) = &node.spec.horizon_config {
            let blue_green_migration = node.spec.strategy.strategy_type == RolloutStrategyType::BlueGreen;
            if horizon_config.auto_migration && !blue_green_migration {
                let init_containers = pod_spec.init_containers.get_or_insert_with(Vec::new);
                init_containers.push(build_horizon_migration_container(node));
            }
        }
    }

    // -------------------------------------------------------------------------
    // Snapshot / compressed-backup restore init container
    //
    // Injected when `spec.storage.snapshotRef.backupUrl` is set.  The init
    // container downloads and extracts the archive into /data before Stellar
    // Core starts, enabling near-instant bootstrap from a compressed DB backup.
    // CSI VolumeSnapshot restores are handled at the PVC level (dataSource) and
    // do NOT need an init container.
    // -------------------------------------------------------------------------
    if let Some(snapshot_ref) = &node.spec.storage.snapshot_ref {
        if let Some(backup_url) = &snapshot_ref.backup_url {
            let init_containers = pod_spec.init_containers.get_or_insert_with(Vec::new);
            init_containers.push(build_snapshot_restore_container(
                node,
                backup_url,
                snapshot_ref.credentials_secret_ref.as_deref(),
                snapshot_ref.restore_image.as_deref(),
            ));
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
                        security_context: Some(SecurityContext {
                            allow_privilege_escalation: Some(false),
                            capabilities: Some(Capabilities {
                                drop: Some(vec!["ALL".to_string()]),
                                add: None,
                            }),
                            run_as_non_root: Some(true),
                            privileged: Some(false),
                            seccomp_profile: Some(SeccompProfile {
                                type_: "RuntimeDefault".to_string(),
                                localhost_profile: None,
                            }),
                            ..Default::default()
                        }),
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
                        security_context: Some(SecurityContext {
                            allow_privilege_escalation: Some(false),
                            capabilities: Some(Capabilities {
                                drop: Some(vec!["ALL".to_string()]),
                                add: None,
                            }),
                            run_as_non_root: Some(true),
                            privileged: Some(false),
                            seccomp_profile: Some(SeccompProfile {
                                type_: "RuntimeDefault".to_string(),
                                localhost_profile: None,
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                } else if hsm_config.provider == HsmProvider::Azure {
                    volumes.push(Volume {
                        name: "dedicatedhsm-socket".to_string(),
                        empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource {
                            medium: Some("Memory".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });

                    let containers = &mut pod_spec.containers;
                    containers.push(Container {
                        name: "dedicatedhsm-client".to_string(),
                        image: Some("azure/dedicated-hsm-client:latest".to_string()),
                        command: Some(
                            vec!["/opt/dedicatedhsm/bin/dedicatedhsm_client".to_string()],
                        ),
                        args: Some(vec!["--foreground".to_string()]),
                        volume_mounts: Some(vec![VolumeMount {
                            name: "dedicatedhsm-socket".to_string(),
                            mount_path: "/var/run/dedicatedhsm".to_string(),
                            ..Default::default()
                        }]),
                        security_context: Some(SecurityContext {
                            allow_privilege_escalation: Some(false),
                            capabilities: Some(Capabilities {
                                drop: Some(vec!["ALL".to_string()]),
                                add: None,
                            }),
                            run_as_non_root: Some(true),
                            privileged: Some(false),
                            seccomp_profile: Some(SeccompProfile {
                                type_: "RuntimeDefault".to_string(),
                                localhost_profile: None,
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Add NAT traversal sidecar
    if let Some(nat_cfg) = &node.spec.nat_traversal {
        if nat_cfg.enabled {
            let mut env = vec![EnvVar {
                name: "ENABLE_ICE".to_string(),
                value: Some(nat_cfg.enable_ice.to_string()),
                ..Default::default()
            }];

            if let Some(stun) = &nat_cfg.stun_server {
                env.push(EnvVar {
                    name: "STUN_SERVER".to_string(),
                    value: Some(stun.clone()),
                    ..Default::default()
                });
            }

            if let Some(turn) = &nat_cfg.turn_server {
                env.push(EnvVar {
                    name: "TURN_SERVER".to_string(),
                    value: Some(turn.clone()),
                    ..Default::default()
                });
            }

            if let Some(secret_ref) = &nat_cfg.turn_credentials_secret_ref {
                env.push(EnvVar {
                    name: "TURN_USERNAME".to_string(),
                    value: None,
                    value_from: Some(EnvVarSource {
                        secret_key_ref: Some(SecretKeySelector {
                            name: Some(secret_ref.clone()),
                            key: "username".to_string(),
                            optional: Some(false),
                        }),
                        ..Default::default()
                    }),
                });
                env.push(EnvVar {
                    name: "TURN_PASSWORD".to_string(),
                    value: None,
                    value_from: Some(EnvVarSource {
                        secret_key_ref: Some(SecretKeySelector {
                            name: Some(secret_ref.clone()),
                            key: "password".to_string(),
                            optional: Some(false),
                        }),
                        ..Default::default()
                    }),
                });
            }

            let sidecar_image = nat_cfg
                .sidecar_image
                .clone()
                .unwrap_or_else(|| "stellar/nat-traversal:latest".to_string());

            let containers = &mut pod_spec.containers;
            containers.push(Container {
                name: "nat-traversal".to_string(),
                image: Some(sidecar_image),
                env: Some(env),
                security_context: Some(SecurityContext {
                    allow_privilege_escalation: Some(false),
                    capabilities: Some(Capabilities {
                        drop: Some(vec!["ALL".to_string()]),
                        add: None,
                    }),
                    run_as_non_root: Some(true),
                    privileged: Some(false),
                    seccomp_profile: Some(SeccompProfile {
                        type_: "RuntimeDefault".to_string(),
                        localhost_profile: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
    }

    // ==========================================================================
    // Merge user-defined sidecar containers into the pod spec
    // ==========================================================================
    if let Some(sidecars) = &node.spec.sidecars {
        pod_spec.containers.extend(sidecars.iter().cloned());
    }

    // ==========================================================================
    // Inject hitless-upgrade handoff sidecar (Validators only, when enabled)
    // ==========================================================================
    if let Some(hu_config) = &node.spec.hitless_upgrade {
        if hu_config.enabled && node.spec.node_type == NodeType::Validator {
            let sidecar_image = hu_config
                .sidecar_image
                .clone()
                .unwrap_or_else(|| "stellar-k8s/handoff-sidecar:latest".to_string());

            // Shared emptyDir volume for the Unix domain socket
            let handoff_vol = k8s_openapi::api::core::v1::Volume {
                name: "handoff-socket".to_string(),
                empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
                ..Default::default()
            };
            pod_spec
                .volumes
                .get_or_insert_with(Vec::new)
                .push(handoff_vol);

            let handoff_mount = k8s_openapi::api::core::v1::VolumeMount {
                name: "handoff-socket".to_string(),
                mount_path: "/handoff".to_string(),
                ..Default::default()
            };

            // Mount the handoff volume into the main container as well
            if let Some(main_container) = pod_spec.containers.first_mut() {
                main_container
                    .volume_mounts
                    .get_or_insert_with(Vec::new)
                    .push(handoff_mount.clone());
            }

            let handoff_sidecar = k8s_openapi::api::core::v1::Container {
                name: "stellar-handoff".to_string(),
                image: Some(sidecar_image),
                args: Some(vec![
                    "handoff".to_string(),
                    "--socket".to_string(),
                    "/handoff/sock".to_string(),
                    "--timeout".to_string(),
                    hu_config.handoff_timeout_seconds.to_string(),
                ]),
                volume_mounts: Some(vec![handoff_mount]),
                liveness_probe: Some(k8s_openapi::api::core::v1::Probe {
                    http_get: Some(k8s_openapi::api::core::v1::HTTPGetAction {
                        path: Some("/healthz".to_string()),
                        port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8080),
                        ..Default::default()
                    }),
                    initial_delay_seconds: Some(5),
                    period_seconds: Some(10),
                    ..Default::default()
                }),
                ..Default::default()
            };
            pod_spec.containers.push(handoff_sidecar);
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

    // ==========================================================================
    // Inject log-shipper sidecar when spec.logShipper.enabled == true
    // ==========================================================================
    if let Some(ls) = &node.spec.log_shipper {
        if ls.enabled {
            // Shared emptyDir volume for log files written by the main container.
            pod_spec.volumes.get_or_insert_with(Vec::new).push(Volume {
                name: "stellar-logs".to_string(),
                empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
                ..Default::default()
            });

            // Mount the shared log volume into the main container.
            if let Some(main) = pod_spec.containers.first_mut() {
                main.volume_mounts
                    .get_or_insert_with(Vec::new)
                    .push(VolumeMount {
                        name: "stellar-logs".to_string(),
                        mount_path: "/var/log/stellar".to_string(),
                        ..Default::default()
                    });
            }

            // Build env vars for the sidecar.
            let mut env = vec![
                EnvVar {
                    name: "S3_BUCKET".to_string(),
                    value: Some(ls.s3_bucket.clone()),
                    ..Default::default()
                },
                EnvVar {
                    name: "S3_PREFIX".to_string(),
                    value: Some(
                        ls.s3_prefix
                            .clone()
                            .unwrap_or_else(|| "stellar-logs".to_string()),
                    ),
                    ..Default::default()
                },
                EnvVar {
                    name: "S3_REGION".to_string(),
                    value: Some(
                        ls.s3_region
                            .clone()
                            .unwrap_or_else(|| "us-east-1".to_string()),
                    ),
                    ..Default::default()
                },
                EnvVar {
                    name: "BATCH_SIZE_LINES".to_string(),
                    value: Some(ls.batch_size_lines.to_string()),
                    ..Default::default()
                },
                EnvVar {
                    name: "FLUSH_INTERVAL_SECS".to_string(),
                    value: Some(ls.flush_interval_secs.to_string()),
                    ..Default::default()
                },
                // Kubernetes downward API: inject the pod name as NODE_NAME.
                EnvVar {
                    name: "NODE_NAME".to_string(),
                    value_from: Some(EnvVarSource {
                        field_ref: Some(k8s_openapi::api::core::v1::ObjectFieldSelector {
                            field_path: "metadata.name".to_string(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ];

            // Inject AWS credentials from a Secret if specified.
            if let Some(secret_ref) = &ls.credentials_secret_ref {
                for key in &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"] {
                    env.push(EnvVar {
                        name: key.to_string(),
                        value_from: Some(EnvVarSource {
                            secret_key_ref: Some(SecretKeySelector {
                                name: Some(secret_ref.clone()),
                                key: key.to_string(),
                                optional: Some(false),
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
            }

            let sidecar_image = ls.image.clone().unwrap_or_else(|| {
                format!("ghcr.io/stellar/stellar-k8s:{}", env!("CARGO_PKG_VERSION"))
            });

            pod_spec.containers.push(Container {
                name: "stellar-log-shipper".to_string(),
                image: Some(sidecar_image),
                command: Some(vec!["/stellar-log-shipper".to_string()]),
                env: Some(env),
                volume_mounts: Some(vec![VolumeMount {
                    name: "stellar-logs".to_string(),
                    mount_path: "/var/log/stellar".to_string(),
                    read_only: Some(true),
                    ..Default::default()
                }]),
                resources: Some(K8sResources {
                    requests: Some(
                        [
                            ("cpu".to_string(), Quantity("50m".to_string())),
                            ("memory".to_string(), Quantity("32Mi".to_string())),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    limits: Some(
                        [
                            ("cpu".to_string(), Quantity("200m".to_string())),
                            ("memory".to_string(), Quantity("128Mi".to_string())),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    ..Default::default()
                }),
                security_context: Some(SecurityContext {
                    allow_privilege_escalation: Some(false),
                    read_only_root_filesystem: Some(true),
                    run_as_non_root: Some(true),
                    capabilities: Some(Capabilities {
                        drop: Some(vec!["ALL".to_string()]),
                        add: None,
                    }),
                    seccomp_profile: Some(SeccompProfile {
                        type_: "RuntimeDefault".to_string(),
                        localhost_profile: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
    }
    // NEW: Inject ebpf-exporter sidecar (Validators only, when enabled)
    // ==========================================================================
    if let Some(ebpf_cfg) = &node.spec.ebpf_config {
        if ebpf_cfg.enabled && node.spec.node_type == NodeType::Validator {
            let exporter_args = vec!["--config.file=/ebpf/ebpf-exporter.yaml".to_string()];

            let sidecar_image = "cloudflare/ebpf_exporter:latest".to_string();

            let ebpf_container = k8s_openapi::api::core::v1::Container {
                name: "ebpf-exporter".to_string(),
                image: Some(sidecar_image),
                args: Some(exporter_args),
                ports: Some(vec![k8s_openapi::api::core::v1::ContainerPort {
                    name: Some("metrics".to_string()),
                    container_port: 9435,
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                }]),
                volume_mounts: Some(vec![
                    k8s_openapi::api::core::v1::VolumeMount {
                        name: "config".to_string(),
                        mount_path: "/ebpf".to_string(),
                        read_only: Some(true),
                        ..Default::default()
                    },
                    k8s_openapi::api::core::v1::VolumeMount {
                        name: "sys-kernel-debug".to_string(),
                        mount_path: "/sys/kernel/debug".to_string(),
                        read_only: Some(false),
                        ..Default::default()
                    },
                    k8s_openapi::api::core::v1::VolumeMount {
                        name: "lib-modules".to_string(),
                        mount_path: "/lib/modules".to_string(),
                        read_only: Some(true),
                        ..Default::default()
                    },
                ]),
                security_context: Some(SecurityContext {
                    privileged: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            };
            pod_spec.containers.push(ebpf_container);

            let vols = pod_spec.volumes.get_or_insert_with(Vec::new);
            vols.push(k8s_openapi::api::core::v1::Volume {
                name: "sys-kernel-debug".to_string(),
                host_path: Some(k8s_openapi::api::core::v1::HostPathVolumeSource {
                    path: "/sys/kernel/debug".to_string(),
                    type_: Some("DirectoryOrCreate".to_string()),
                }),
                ..Default::default()
            });
            vols.push(k8s_openapi::api::core::v1::Volume {
                name: "lib-modules".to_string(),
                host_path: Some(k8s_openapi::api::core::v1::HostPathVolumeSource {
                    path: "/lib/modules".to_string(),
                    type_: Some("Directory".to_string()),
                }),
                ..Default::default()
            });
        }
    }
    // ==========================================================================

    let mut apparmor_annotations = BTreeMap::new();
    if let Some(containers) = &pod_spec.init_containers {
        for container in containers {
            apparmor_annotations.insert(
                format!(
                    "container.apparmor.security.beta.kubernetes.io/{}",
                    container.name
                ),
                "runtime/default".to_string(),
            );
        }
    }
    for container in &pod_spec.containers {
        apparmor_annotations.insert(
            format!(
                "container.apparmor.security.beta.kubernetes.io/{}",
                container.name
            ),
            "runtime/default".to_string(),
        );
    }

    let mut pod_object_meta = ObjectMeta {
        labels: Some(labels.clone()),
        annotations: if apparmor_annotations.is_empty() {
            None
        } else {
            Some(apparmor_annotations)
        },
        ..Default::default()
    };
    if let Some(inj) = seed_injection {
        if let Some(ann) = inj.pod_annotations() {
            let mut merged = pod_object_meta.annotations.unwrap_or_default();
            merged.extend(ann.iter().map(|(k, v)| (k.clone(), v.clone())));
            pod_object_meta.annotations = Some(merged);
        }
    }

    PodTemplateSpec {
        metadata: Some(merge_resource_meta(
            pod_object_meta,
            &node.spec.resource_meta,
        )),
        spec: Some(pod_spec),
    }
}

fn parse_cpu_millicores(cpu: &str) -> Option<u32> {
    let trimmed = cpu.trim();
    if let Some(milli) = trimmed.strip_suffix('m') {
        return milli.parse::<u32>().ok();
    }

    let cores = trimmed.parse::<f64>().ok()?;
    if cores.is_sign_negative() {
        return None;
    }

    Some((cores * 1000.0).round() as u32)
}

fn derive_worker_threads(node: &StellarNode) -> u32 {
    let millicores = parse_cpu_millicores(&node.spec.resources.limits.cpu)
        .or_else(|| parse_cpu_millicores(&node.spec.resources.requests.cpu))
        .unwrap_or(1000);

    let cores = millicores.div_ceil(1000).clamp(1, 32);
    cores.max(1)
}

fn network_spread_label_selector(spec: &StellarNodeSpec) -> LabelSelector {
    LabelSelector {
        match_labels: Some(BTreeMap::from([
            (
                "app.kubernetes.io/name".to_string(),
                "stellar-node".to_string(),
            ),
            (
                "stellar-network".to_string(),
                spec.network
                    .scheduling_label_value(&spec.custom_network_passphrase),
            ),
            (
                "app.kubernetes.io/component".to_string(),
                spec.node_type.to_string().to_lowercase(),
            ),
        ])),
        ..Default::default()
    }
}

pub(crate) fn merge_workload_affinity(node: &StellarNode) -> Option<Affinity> {
    let mut aff = Affinity::default();
    if let Some(na) = node.spec.storage.node_affinity.clone() {
        aff.node_affinity = Some(na);
    }

    // Inject jurisdiction nodeAffinity (overrides storage node_affinity if both set)
    if let Some(jurisdiction) = node.spec.placement.jurisdiction.as_ref() {
        if let Some(jur_affinity) =
            crate::controller::jurisdiction::build_jurisdiction_node_affinity(jurisdiction)
        {
            aff.node_affinity = Some(jur_affinity);
        }
    }

    let mut req_terms = Vec::new();
    let mut pref_terms = Vec::new();

    // 1. Default network-level separation
    if let Some(pa) = build_network_pod_anti_affinity(node) {
        if let Some(mut req) = pa.required_during_scheduling_ignored_during_execution {
            req_terms.append(&mut req);
        }
        if let Some(mut pref) = pa.preferred_during_scheduling_ignored_during_execution {
            pref_terms.append(&mut pref);
        }
    }

    // 2. SCP-aware separation (Validators only)
    if let Some(pa) = build_scp_aware_pod_anti_affinity(node) {
        if let Some(mut req) = pa.required_during_scheduling_ignored_during_execution {
            req_terms.append(&mut req);
        }
        if let Some(mut pref) = pa.preferred_during_scheduling_ignored_during_execution {
            pref_terms.append(&mut pref);
        }
    }

    if !req_terms.is_empty() || !pref_terms.is_empty() {
        aff.pod_anti_affinity = Some(PodAntiAffinity {
            required_during_scheduling_ignored_during_execution: if req_terms.is_empty() {
                None
            } else {
                Some(req_terms)
            },
            preferred_during_scheduling_ignored_during_execution: if pref_terms.is_empty() {
                None
            } else {
                Some(pref_terms)
            },
        });
    }

    if aff.node_affinity.is_none() && aff.pod_anti_affinity.is_none() {
        None
    } else {
        Some(aff)
    }
}

fn build_scp_aware_pod_anti_affinity(node: &StellarNode) -> Option<PodAntiAffinity> {
    // Only applies to Validators when SCP-aware placement is enabled
    if node.spec.node_type != NodeType::Validator || !node.spec.placement.scp_aware_anti_affinity {
        return None;
    }

    let qset = node
        .spec
        .validator_config
        .as_ref()
        .and_then(|c| c.quorum_set.as_ref())?;

    let peer_names = extract_peer_names_from_toml(qset);
    if peer_names.is_empty() {
        return None;
    }

    let mut terms = Vec::new();

    for peer_name in peer_names {
        // We discourage placing this validator on the same node as its quorum set members.
        // Each peer is identified by its instance name label.
        let mut match_labels = BTreeMap::new();
        match_labels.insert("app.kubernetes.io/instance".to_string(), peer_name);

        terms.push(WeightedPodAffinityTerm {
            weight: 100,
            pod_affinity_term: PodAffinityTerm {
                label_selector: Some(LabelSelector {
                    match_labels: Some(match_labels),
                    ..Default::default()
                }),
                topology_key: "kubernetes.io/hostname".to_string(),
                ..Default::default()
            },
        });
    }

    Some(PodAntiAffinity {
        preferred_during_scheduling_ignored_during_execution: Some(terms),
        ..Default::default()
    })
}

fn build_network_pod_anti_affinity(node: &StellarNode) -> Option<PodAntiAffinity> {
    match node.spec.pod_anti_affinity {
        PodAntiAffinityStrength::Disabled => None,
        PodAntiAffinityStrength::Hard => {
            let term = PodAffinityTerm {
                label_selector: Some(network_spread_label_selector(&node.spec)),
                topology_key: "kubernetes.io/hostname".to_string(),
                ..Default::default()
            };
            Some(PodAntiAffinity {
                required_during_scheduling_ignored_during_execution: Some(vec![term]),
                ..Default::default()
            })
        }
        PodAntiAffinityStrength::Soft => {
            let term = PodAffinityTerm {
                label_selector: Some(network_spread_label_selector(&node.spec)),
                topology_key: "kubernetes.io/hostname".to_string(),
                ..Default::default()
            };
            Some(PodAntiAffinity {
                preferred_during_scheduling_ignored_during_execution: Some(vec![
                    WeightedPodAffinityTerm {
                        weight: 100,
                        pod_affinity_term: term,
                    },
                ]),
                ..Default::default()
            })
        }
    }
}

/// Build `TopologySpreadConstraints` for a pod spec.
pub fn build_topology_spread_constraints(
    spec: &crate::crd::StellarNodeSpec,
    _node_name: &str,
) -> Vec<k8s_openapi::api::core::v1::TopologySpreadConstraint> {
    use k8s_openapi::api::core::v1::TopologySpreadConstraint;

    if let Some(constraints) = &spec.topology_spread_constraints {
        if !constraints.is_empty() {
            return constraints.clone();
        }
    }

    let when_unsatisfiable = match spec.pod_anti_affinity {
        PodAntiAffinityStrength::Soft => "ScheduleAnyway".to_string(),
        PodAntiAffinityStrength::Hard | PodAntiAffinityStrength::Disabled => {
            "DoNotSchedule".to_string()
        }
    };

    let selector = network_spread_label_selector(spec);

    vec![
        TopologySpreadConstraint {
            max_skew: 1,
            topology_key: "kubernetes.io/hostname".to_string(),
            when_unsatisfiable: when_unsatisfiable.clone(),
            label_selector: Some(selector.clone()),
            ..Default::default()
        },
        TopologySpreadConstraint {
            max_skew: 1,
            topology_key: "topology.kubernetes.io/zone".to_string(),
            when_unsatisfiable,
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
        value: Some(node.spec.network_passphrase().to_string()),
        ..Default::default()
    }];

    let worker_threads = derive_worker_threads(node);
    match node.spec.node_type {
        NodeType::Validator => {
            env_vars.push(EnvVar {
                name: "STELLAR_CORE_WORKER_THREADS".to_string(),
                value: Some(worker_threads.to_string()),
                ..Default::default()
            });
            env_vars.push(EnvVar {
                name: "STELLAR_CORE_HTTP_QUERY_THREADS".to_string(),
                value: Some((worker_threads.max(2) / 2).max(1).to_string()),
                ..Default::default()
            });
        }
        NodeType::Horizon => {
            let ingest_workers = node
                .spec
                .horizon_config
                .as_ref()
                .map(|cfg| cfg.ingest_workers.max(1))
                .unwrap_or(worker_threads);
            env_vars.push(EnvVar {
                name: "HORIZON_INGEST_WORKERS".to_string(),
                value: Some(ingest_workers.to_string()),
                ..Default::default()
            });
        }
        NodeType::SorobanRpc => {
            env_vars.push(EnvVar {
                name: "SOROBAN_RPC_WORKER_THREADS".to_string(),
                value: Some(worker_threads.to_string()),
                ..Default::default()
            });
            env_vars.push(EnvVar {
                name: "CAPTIVE_CORE_WORKER_THREADS".to_string(),
                value: Some((worker_threads / 2).max(1).to_string()),
                ..Default::default()
            });
        }
    }

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
                secret_key_ref: db_config
                    .secret_key_ref
                    .as_ref()
                    .map(|r| SecretKeySelector {
                        name: Some(r.name.clone()),
                        key: r.key.clone(),
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
                } else if hsm_config.provider == HsmProvider::Azure {
                    // Sidecar bridge for PKCS#11 access to Azure Dedicated HSM.
                    extra_volume_mounts.push(VolumeMount {
                        name: "dedicatedhsm-socket".to_string(),
                        mount_path: "/var/run/dedicatedhsm".to_string(),
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
        security_context: Some(SecurityContext {
            allow_privilege_escalation: Some(false),
            capabilities: Some(Capabilities {
                add: None,
                drop: Some(vec!["ALL".to_string()]),
            }),
            run_as_non_root: Some(true),
            privileged: Some(false),
            read_only_root_filesystem: Some(true),
            seccomp_profile: Some(SeccompProfile {
                localhost_profile: None,
                type_: "RuntimeDefault".to_string(),
            }),
            ..Default::default()
        }),
        volume_mounts: Some(volume_mounts),
        liveness_probe: apply_probe_override(
            None,
            node.spec.probes.as_ref().and_then(|p| p.liveness.as_ref()),
        ),
        readiness_probe: apply_probe_override(
            None,
            node.spec.probes.as_ref().and_then(|p| p.readiness.as_ref()),
        ),
        startup_probe: apply_probe_override(
            None,
            node.spec.probes.as_ref().and_then(|p| p.startup.as_ref()),
        ),
        ..Default::default()
    }
}

/// Build the migration container for Horizon
pub(crate) fn build_horizon_migration_container(node: &StellarNode) -> Container {
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

/// Build the snapshot-restore init container for compressed DB backup bootstrapping.
///
/// This container runs before Stellar Core and:
/// 1. Checks whether `/data` is already populated (idempotent — skips if data exists).
/// 2. Downloads the archive from `backup_url` (S3 or HTTPS).
/// 3. Extracts it into `/data`.
///
/// Supports `.tar.gz` and `.tar.zst` archives.
/// For S3 URLs, AWS CLI credentials are injected from `credentials_secret_ref`.
fn build_snapshot_restore_container(
    _node: &StellarNode,
    backup_url: &str,
    credentials_secret_ref: Option<&str>,
    restore_image: Option<&str>,
) -> Container {
    // Choose a sensible default image based on the URL scheme.
    let image = restore_image.map(|s| s.to_string()).unwrap_or_else(|| {
        if backup_url.starts_with("s3://") {
            "amazon/aws-cli:latest".to_string()
        } else {
            "alpine:3".to_string()
        }
    });

    // Determine the decompression command based on the file extension.
    let decompress_flag = if backup_url.ends_with(".tar.zst") {
        "--use-compress-program=zstd"
    } else {
        "-z" // default: gzip
    };

    // Build the shell script that runs inside the init container.
    // The script is idempotent: if /data already has content it exits immediately.
    let script = if backup_url.starts_with("s3://") {
        format!(
            r#"set -e
# Skip restore if data volume already has content (idempotent)
if [ "$(ls -A /data 2>/dev/null)" ]; then
  echo "Data volume already populated, skipping snapshot restore."
  exit 0
fi
echo "Restoring from S3 snapshot: {url}"
aws s3 cp "{url}" /tmp/snapshot.archive
echo "Extracting archive..."
tar {decompress} -xf /tmp/snapshot.archive -C /data
rm -f /tmp/snapshot.archive
echo "Snapshot restore complete."
"#,
            url = backup_url,
            decompress = decompress_flag,
        )
    } else {
        format!(
            r#"set -e
# Skip restore if data volume already has content (idempotent)
if [ "$(ls -A /data 2>/dev/null)" ]; then
  echo "Data volume already populated, skipping snapshot restore."
  exit 0
fi
echo "Restoring from backup: {url}"
wget -q -O /tmp/snapshot.archive "{url}" || curl -fsSL -o /tmp/snapshot.archive "{url}"
echo "Extracting archive..."
tar {decompress} -xf /tmp/snapshot.archive -C /data
rm -f /tmp/snapshot.archive
echo "Snapshot restore complete."
"#,
            url = backup_url,
            decompress = decompress_flag,
        )
    };

    // Build environment variables — inject AWS credentials if provided.
    let mut env: Vec<EnvVar> = vec![EnvVar {
        name: "BACKUP_URL".to_string(),
        value: Some(backup_url.to_string()),
        ..Default::default()
    }];

    if let Some(secret_name) = credentials_secret_ref {
        // AWS credentials
        for key in &[
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_DEFAULT_REGION",
        ] {
            env.push(EnvVar {
                name: key.to_string(),
                value: None,
                value_from: Some(EnvVarSource {
                    secret_key_ref: Some(SecretKeySelector {
                        name: Some(secret_name.to_string()),
                        key: key.to_string(),
                        optional: Some(true),
                    }),
                    ..Default::default()
                }),
            });
        }
        // Generic bearer token for HTTPS
        env.push(EnvVar {
            name: "BEARER_TOKEN".to_string(),
            value: None,
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: Some(secret_name.to_string()),
                    key: "BEARER_TOKEN".to_string(),
                    optional: Some(true),
                }),
                ..Default::default()
            }),
        });
    }

    Container {
        name: "snapshot-restore".to_string(),
        image: Some(image),
        command: Some(vec!["/bin/sh".to_string(), "-c".to_string(), script]),
        env: Some(env),
        volume_mounts: Some(vec![VolumeMount {
            name: "data".to_string(),
            mount_path: "/data".to_string(),
            ..Default::default()
        }]),
        // Security: run as non-root, read-only root filesystem except /tmp
        security_context: Some(SecurityContext {
            run_as_non_root: Some(false), // aws-cli/alpine may need root for tar
            allow_privilege_escalation: Some(false),
            capabilities: Some(Capabilities {
                drop: Some(vec!["ALL".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        }),
        resources: Some(K8sResources {
            requests: Some({
                let mut m = BTreeMap::new();
                m.insert("cpu".to_string(), Quantity("100m".to_string()));
                m.insert("memory".to_string(), Quantity("256Mi".to_string()));
                m
            }),
            limits: Some({
                let mut m = BTreeMap::new();
                m.insert("cpu".to_string(), Quantity("500m".to_string()));
                m.insert("memory".to_string(), Quantity("512Mi".to_string()));
                m
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

// ============================================================================
// HorizontalPodAutoscaler — unchanged
// ============================================================================

pub async fn ensure_hpa(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
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
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    info!("HPA ensured for {}/{}", namespace, name);
    Ok(())
}

// ============================================================================
// Alerting — unchanged
// ============================================================================

pub async fn ensure_alerting(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "alerts");

    if !node.spec.alerting {
        return delete_alerting(client, node, dry_run).await;
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
    api.patch(&name, &patch_params(dry_run), &patch).await?;

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
        let metric = match metric_name.as_str() {
            "ledger_ingestion_lag" => Some((
                "stellar_node_ingestion_lag".to_string(),
                "Value".to_string(),
                Quantity("5".to_string()),
            )),
            "stellar_horizon_tps" | "requests_per_second" => Some((
                "stellar_horizon_tps".to_string(),
                "Value".to_string(),
                Quantity("1000".to_string()),
            )),
            "stellar_queue_length" | "queue_length" | "horizon_queue_length" => Some((
                "stellar_horizon_queue_length".to_string(),
                "Value".to_string(),
                Quantity("50".to_string()),
            )),
            _ => None,
        };

        if let Some((metric_name, target_type, target_value)) = metric {
            metrics.push(MetricSpec {
                type_: "Object".to_string(),
                object: Some(ObjectMetricSource {
                    described_object: CrossVersionObjectReference {
                        api_version: Some("stellar.org/v1alpha1".to_string()),
                        kind: "StellarNode".to_string(),
                        name: node.name_any(),
                    },
                    metric: MetricIdentifier {
                        name: metric_name,
                        selector: None,
                    },
                    target: MetricTarget {
                        type_: target_type,
                        value: Some(target_value),
                        ..Default::default()
                    },
                }),
                ..Default::default()
            });
        } else {
            warn!("Unrecognized custom metric '{}' configured for node {}; skipping.", metric_name, node.name_any());
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

pub async fn delete_hpa(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    if node.spec.autoscaling.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "hpa");

    match api.delete(&name, &delete_params(dry_run)).await {
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

pub async fn delete_alerting(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = resource_name(node, "alerts");

    let api: Api<ConfigMap> = Api::namespaced(client.clone(), &namespace);
    match api.delete(&name, &delete_params(dry_run)).await {
        Ok(_) => info!("Deleted alerting ConfigMap {}", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

pub async fn delete_canary_resources(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let canary_name = format!("{name}-canary");

    if node.spec.ingress.is_some() {
        let api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
        let _ = api.delete(&canary_name, &delete_params(dry_run)).await;
    }

    let api_svc: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let _ = api_svc.delete(&canary_name, &delete_params(dry_run)).await;

    let api_deploy: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    let _ = api_deploy
        .delete(&canary_name, &delete_params(dry_run))
        .await;

    Ok(())
}

// ============================================================================
// NetworkPolicy — unchanged
// ============================================================================

#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_network_policy(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
) -> Result<()> {
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
        &patch_params(dry_run),
        &Patch::Apply(&network_policy),
    )
    .await?;

    info!("NetworkPolicy ensured for {}/{}", namespace, name);
    Ok(())
}

/// Extract peer addresses (IPs or Hostnames) from QUORUM_SET and KNOWN_PEERS TOML strings.
fn extract_peers_from_config(node: &StellarNode) -> Vec<String> {
    let mut peers = Vec::new();
    let config = match &node.spec.validator_config {
        Some(c) => c,
        None => return peers,
    };

    // 1. Parse KNOWN_PEERS if present
    if let Some(known_peers_toml) = &config.known_peers {
        if let Ok(value) = known_peers_toml.parse::<toml::Value>() {
            if let Some(kp_array) = value.as_array() {
                for v in kp_array {
                    if let Some(s) = v.as_str() {
                        // Extract IP/Hostname from "IP:PORT"
                        let peer = s.split(':').next().unwrap_or(s);
                        peers.push(peer.to_string());
                    }
                }
            } else if let Some(kp_table) = value.get("KNOWN_PEERS").and_then(|v| v.as_array()) {
                for v in kp_table {
                    if let Some(s) = v.as_str() {
                        let peer = s.split(':').next().unwrap_or(s);
                        peers.push(peer.to_string());
                    }
                }
            }
        }
    }

    // 2. Parse QUORUM_SET for any direct IP references (rare but possible in custom setups)
    if let Some(qs_toml) = &config.quorum_set {
        if let Ok(value) = qs_toml.parse::<toml::Value>() {
            // Check for [VALIDATORS] section with IP-like keys
            if let Some(validators) = value.get("VALIDATORS").and_then(|v| v.as_table()) {
                for key in validators.keys() {
                    // If key looks like an IP or hostname (not a public key), add it
                    if !key.starts_with('G') && key.contains('.') {
                        peers.push(key.clone());
                    }
                }
            }
        }
    }

    peers.sort();
    peers.dedup();
    peers
}

pub(crate) fn build_network_policy(
    node: &StellarNode,
    config: &NetworkPolicyConfig,
) -> NetworkPolicy {
    let labels = standard_labels(node);
    let name = resource_name(node, "netpol");

    let mut ingress_rules: Vec<NetworkPolicyIngressRule> = Vec::new();
    let mut egress_rules: Vec<k8s_openapi::api::networking::v1::NetworkPolicyEgressRule> =
        Vec::new();

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

        // --- Stellar-Native Egress Rules ---
        // 1. Allow DNS (essential for hostname resolution)
        egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
            to: None, // Allow to all for port 53
            ports: Some(vec![
                NetworkPolicyPort {
                    port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                    protocol: Some("UDP".to_string()),
                    ..Default::default()
                },
                NetworkPolicyPort {
                    port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
            ]),
        });

        // 2. Allow egress to parsed peers (KNOWN_PEERS / QUORUM_SET)
        let peers = extract_peers_from_config(node);
        if !peers.is_empty() {
            let mut peer_egress_to = Vec::new();
            for peer in peers {
                // If it looks like an IP, use ipBlock. If it's a hostname, we can't
                // do much in standard NetPol without a DNS controller, but we can
                // allow all egress on peer ports as a fallback or if IP is known.
                if peer
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == ':')
                {
                    peer_egress_to.push(NetworkPolicyPeer {
                        ip_block: Some(IPBlock {
                            cidr: if peer.contains('/') {
                                peer
                            } else {
                                format!("{}/32", peer)
                            },
                            except: None,
                        }),
                        ..Default::default()
                    });
                }
            }

            egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
                to: if peer_egress_to.is_empty() {
                    None
                } else {
                    Some(peer_egress_to)
                },
                ports: Some(vec![NetworkPolicyPort {
                    port: Some(
                        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11625),
                    ),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                }]),
            });
        }

        // 3. Allow egress to history archives (HTTP/HTTPS)
        if let Some(vc) = &node.spec.validator_config {
            if vc.enable_history_archive && !vc.history_archive_urls.is_empty() {
                egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
                    to: None, // External history archives
                    ports: Some(vec![
                        NetworkPolicyPort {
                            port: Some(
                                k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(80),
                            ),
                            protocol: Some("TCP".to_string()),
                            ..Default::default()
                        },
                        NetworkPolicyPort {
                            port: Some(
                                k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(443),
                            ),
                            protocol: Some("TCP".to_string()),
                            ..Default::default()
                        },
                    ]),
                });
            }
        }
    } else {
        // Horizon / Soroban RPC egress rules
        // 1. Allow DNS
        egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
            to: None,
            ports: Some(vec![
                NetworkPolicyPort {
                    port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                    protocol: Some("UDP".to_string()),
                    ..Default::default()
                },
                NetworkPolicyPort {
                    port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
            ]),
        });

        // 2. Allow egress to Stellar Core (usually in the same namespace)
        egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
            to: Some(vec![NetworkPolicyPeer {
                pod_selector: Some(LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "app.kubernetes.io/name".to_string(),
                        "stellar-node".to_string(),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ports: Some(vec![
                NetworkPolicyPort {
                    port: Some(
                        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11625),
                    ),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
                NetworkPolicyPort {
                    port: Some(
                        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11626),
                    ),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
            ]),
        });

        // 3. Allow egress to external databases if configured
        if node.spec.database.is_some() || node.spec.managed_database.is_some() {
            egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
                to: None, // External DBs or CNPG
                ports: Some(vec![NetworkPolicyPort {
                    port: Some(
                        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(5432),
                    ),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                }]),
            });
        }
    }

    // -----------------------------------------------------------------------
    // Egress rules — Network Isolation
    //
    // Allow egress only to:
    //   1. Pods in namespaces labelled with the SAME stellar.org/network value.
    //      This is the critical rule: it prevents a Testnet pod from ever
    //      opening a TCP connection to a Mainnet pod, even if both are on the
    //      same cluster.
    //   2. kube-dns (UDP/TCP 53) — required for all pods.
    //   3. The Kubernetes API server (TCP 443/6443) — required for health checks.
    //   4. Intra-namespace traffic (e.g. Horizon → Stellar Core).
    //
    // Any egress not matched by these rules is implicitly denied because we
    // include "Egress" in policy_types.
    // -----------------------------------------------------------------------
    use k8s_openapi::api::networking::v1::NetworkPolicyEgressRule;

    let network_label_value = crate::controller::network_isolation::network_label_value(
        &node.spec.network,
        &node.spec.custom_network_passphrase,
    );

    // Rule 1: Allow egress to pods in same-network namespaces only.
    let same_network_egress = NetworkPolicyEgressRule {
        to: Some(vec![NetworkPolicyPeer {
            namespace_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    crate::controller::network_isolation::NAMESPACE_NETWORK_LABEL.to_string(),
                    network_label_value.clone(),
                )])),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        ports: None,
    };

    // Rule 2: Allow DNS resolution (kube-dns).
    let dns_egress = NetworkPolicyEgressRule {
        to: Some(vec![NetworkPolicyPeer {
            namespace_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    "kubernetes.io/metadata.name".to_string(),
                    "kube-system".to_string(),
                )])),
                ..Default::default()
            }),
            pod_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    "k8s-app".to_string(),
                    "kube-dns".to_string(),
                )])),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        ports: Some(vec![
            NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                protocol: Some("UDP".to_string()),
                ..Default::default()
            },
            NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
        ]),
    };

    // Rule 3: Allow egress within the same namespace (intra-namespace pod communication,
    // e.g. Horizon → Stellar Core, Soroban RPC → Captive Core).
    let intra_namespace_egress = NetworkPolicyEgressRule {
        to: Some(vec![NetworkPolicyPeer {
            namespace_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    "kubernetes.io/metadata.name".to_string(),
                    node.namespace().unwrap_or_else(|| "default".to_string()),
                )])),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        ports: None,
    };

    let egress_rules = vec![same_network_egress, dns_egress, intra_namespace_egress];

    NetworkPolicy {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: node.namespace(),
                labels: Some({
                    let mut l = labels;
                    // Stamp the network label on the NetworkPolicy itself so
                    // cluster-level policies can select it.
                    l.insert(
                        crate::controller::network_isolation::NAMESPACE_NETWORK_LABEL.to_string(),
                        network_label_value,
                    );
                    l
                }),
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
            // Enforce both Ingress and Egress so the egress deny-by-default takes effect.
            policy_types: Some(vec!["Ingress".to_string(), "Egress".to_string()]),
            ingress: if ingress_rules.is_empty() {
                None
            } else {
                Some(ingress_rules)
            },
            egress: if egress_rules.is_empty() {
                None
            } else {
                Some(egress_rules)
            },
        }),
    }
}

#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_network_policy(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "netpol");

    match api.delete(&name, &delete_params(dry_run)).await {
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

pub async fn ensure_pdb(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    if node.spec.replicas <= 1 {
        return delete_pdb(client, node, dry_run).await;
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), &namespace);

    if let Some(pdb) = build_pdb(node) {
        let name = pdb.metadata.name.clone().unwrap();

        info!("Reconciling PodDisruptionBudget {}/{}", namespace, name);
        let params = patch_params(dry_run);
        api.patch(&name, &params, &Patch::Apply(&pdb))
            .await
            .map_err(Error::KubeError)?;
    }

    Ok(())
}

pub async fn delete_pdb(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), &namespace);

    match api.delete(&name, &delete_params(dry_run)).await {
        Ok(_) => info!("Deleted PodDisruptionBudget {}/{}", namespace, name),
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

// ============================================================================
// Test helpers — thin wrappers that expose private builders for unit tests
// (Issue #298)
// ============================================================================

#[cfg(test)]
pub(crate) fn build_pvc_for_test(
    node: &StellarNode,
    storage_class: String,
) -> k8s_openapi::api::core::v1::PersistentVolumeClaim {
    build_pvc(node, storage_class)
}

#[cfg(test)]
pub(crate) fn build_config_map_for_test(node: &StellarNode) -> ConfigMap {
    build_config_map(node, None, false)
}

#[cfg(test)]
pub(crate) fn build_deployment_for_test(
    node: &StellarNode,
) -> k8s_openapi::api::apps::v1::Deployment {
    build_deployment(node, false)
}

#[cfg(test)]
pub(crate) fn build_statefulset_for_test(
    node: &StellarNode,
) -> k8s_openapi::api::apps::v1::StatefulSet {
    build_statefulset(node, false, None)
}

#[cfg(test)]
pub(crate) fn build_service_for_test(node: &StellarNode) -> k8s_openapi::api::core::v1::Service {
    build_service(node, false)
}

#[cfg(test)]
mod ensure_pvc_tests {
    use super::{build_pvc, pvc_needs_update, resolve_pvc_storage_class};
    use crate::crd::{
        types::{ResourceRequirements, ResourceSpec, StorageMode},
        NodeType, StellarNetwork, StellarNode, StellarNodeSpec,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn test_node() -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("stellar-system".to_string()),
                uid: Some("abc-123".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Validator,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
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
                validator_config: None,
                horizon_config: None,
                soroban_config: None,
                replicas: 1,
                min_available: None,
                max_unavailable: None,
                suspended: false,
                alerting: false,
                database: None,
                managed_database: None,
                autoscaling: None,
                vpa_config: None,
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
                strategy: Default::default(),
                maintenance_mode: false,
                network_policy: None,
                dr_config: None,
                pod_anti_affinity: Default::default(),
                placement: Default::default(),
                topology_spread_constraints: None,
                cve_handling: None,
                snapshot_schedule: None,
                restore_from_snapshot: None,
                read_replica_config: None,
                read_pool_endpoint: None,
                db_maintenance_config: None,
                oci_snapshot: None,
                service_mesh: None,
                forensic_snapshot: None,
                label_propagation: None,
                resource_meta: None,
                sidecars: None,
                cert_manager: None,
                nat_traversal: None,
                custom_network_passphrase: None,
                cross_cloud_failover: None,
                hitless_upgrade: None,
                history_mode: Default::default(),
                storage: Default::default(),
                ..Default::default()
            },
            status: None,
        }
    }

    #[test]
    fn resolves_storage_class_with_explicit_value() {
        let mut node = test_node();
        node.spec.storage.mode = StorageMode::Local;
        node.spec.storage.storage_class = "fast-ssd".to_string();

        let resolved = resolve_pvc_storage_class(&node, true, true);
        assert_eq!(resolved, "fast-ssd");
    }

    #[test]
    fn resolves_storage_class_to_local_path_for_local_mode() {
        let mut node = test_node();
        node.spec.storage.mode = StorageMode::Local;
        node.spec.storage.storage_class.clear();

        let resolved = resolve_pvc_storage_class(&node, true, false);
        assert_eq!(resolved, "local-path");
    }

    #[test]
    fn resolves_storage_class_to_local_storage_when_path_missing() {
        let mut node = test_node();
        node.spec.storage.mode = StorageMode::Local;
        node.spec.storage.storage_class.clear();

        let resolved = resolve_pvc_storage_class(&node, false, true);
        assert_eq!(resolved, "local-storage");
    }

    #[test]
    fn resolves_storage_class_to_empty_when_no_local_class_found() {
        let mut node = test_node();
        node.spec.storage.mode = StorageMode::Local;
        node.spec.storage.storage_class.clear();

        let resolved = resolve_pvc_storage_class(&node, false, false);
        assert!(resolved.is_empty());
    }

    #[test]
    fn build_pvc_uses_resolved_storage_class() {
        let node = test_node();
        let pvc = build_pvc(&node, "gp3".to_string());

        assert_eq!(
            pvc.spec
                .as_ref()
                .and_then(|s| s.storage_class_name.as_deref()),
            Some("gp3")
        );
    }

    #[test]
    fn pvc_update_detects_storage_class_change() {
        let node = test_node();
        let existing = build_pvc(&node, "standard".to_string());
        let desired = build_pvc(&node, "gp3".to_string());

        assert!(pvc_needs_update(&existing, &desired));
    }

    #[test]
    fn pvc_update_skips_when_specs_match() {
        let node = test_node();
        let existing = build_pvc(&node, "standard".to_string());
        let desired = build_pvc(&node, "standard".to_string());

        assert!(!pvc_needs_update(&existing, &desired));
    }

    // -----------------------------------------------------------------------
    // Retention policy — Delete scenario
    // -----------------------------------------------------------------------

    #[test]
    fn should_delete_pvc_returns_true_for_delete_policy() {
        use crate::crd::types::RetentionPolicy;
        let mut node = test_node();
        node.spec.storage.retention_policy = RetentionPolicy::Delete;
        assert!(
            node.spec.should_delete_pvc(),
            "Delete policy must trigger PVC deletion"
        );
    }

    // -----------------------------------------------------------------------
    // Retention policy — Retain scenario
    // -----------------------------------------------------------------------

    #[test]
    fn should_delete_pvc_returns_false_for_retain_policy() {
        use crate::crd::types::RetentionPolicy;
        let mut node = test_node();
        node.spec.storage.retention_policy = RetentionPolicy::Retain;
        assert!(
            !node.spec.should_delete_pvc(),
            "Retain policy must prevent PVC deletion"
        );
    }

    #[test]
    fn default_retention_policy_is_delete() {
        // StorageConfig::default() must use Delete so orphaned PVCs are
        // cleaned up unless the user explicitly opts into Retain.
        let node = test_node();
        assert!(
            node.spec.should_delete_pvc(),
            "default retention policy must be Delete"
        );
    }

    #[test]
    fn pvc_built_with_delete_policy_has_correct_storage_class() {
        use crate::crd::types::RetentionPolicy;
        let mut node = test_node();
        node.spec.storage.retention_policy = RetentionPolicy::Delete;
        let pvc = build_pvc(&node, "fast-ssd".to_string());
        assert_eq!(
            pvc.spec
                .as_ref()
                .and_then(|s| s.storage_class_name.as_deref()),
            Some("fast-ssd"),
            "PVC storage class must be preserved regardless of retention policy"
        );
    }

    #[test]
    fn pvc_built_with_retain_policy_has_correct_storage_class() {
        use crate::crd::types::RetentionPolicy;
        let mut node = test_node();
        node.spec.storage.retention_policy = RetentionPolicy::Retain;
        let pvc = build_pvc(&node, "standard".to_string());
        assert_eq!(
            pvc.spec
                .as_ref()
                .and_then(|s| s.storage_class_name.as_deref()),
            Some("standard"),
            "PVC storage class must be preserved regardless of retention policy"
        );
    }

    #[test]
    fn build_hpa_includes_supported_custom_metrics() {
        use crate::crd::types::AutoscalingConfig;

        let mut node = test_node();
        node.spec.autoscaling = Some(AutoscalingConfig {
            min_replicas: 1,
            max_replicas: 5,
            custom_metrics: vec![
                "stellar_horizon_tps".to_string(),
                "stellar_queue_length".to_string(),
            ],
            ..Default::default()
        });

        let hpa = build_hpa(&node).expect("HPA should build with supported custom metrics");
        let metrics = hpa
            .spec
            .as_ref()
            .and_then(|spec| spec.metrics.as_ref())
            .expect("HPA spec metrics should exist");

        let metric_names: Vec<String> = metrics
            .iter()
            .filter_map(|spec| spec.object.as_ref().map(|object| object.metric.name.clone()))
            .collect();

        assert!(metric_names.contains(&"stellar_horizon_tps".to_string()));
        assert!(metric_names.contains(&"stellar_horizon_queue_length".to_string()));
    }
}
