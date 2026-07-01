//! StatefulSet management.

use super::helpers::*;
use super::pod_template::*;
use super::prelude::*;

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
pub(crate) fn build_statefulset(
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
