//! Deployment management.

use super::prelude::*;
use super::helpers::*;
use super::pod_template::*;

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

pub(crate) fn build_deployment(node: &StellarNode, enable_mtls: bool) -> Deployment {
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

