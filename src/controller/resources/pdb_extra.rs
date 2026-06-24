//! Additional PDB builders.

use super::prelude::*;
use super::helpers::*;
use super::pdb::build_pdb;

// ============================================================================
// PodDisruptionBudget
// ============================================================================

/// Build a PodDisruptionBudget for a StellarNode.
///
/// For **Validator** nodes the PDB is always generated to protect quorum:
/// - `replicas == 1`: `minAvailable: 1` (prevents all disruptions while still
///   allowing the single pod to be evicted when the node is deleted).
/// - `replicas > 1`: `minAvailable = (replicas / 2) + 1` so that a strict
///   majority of validators is always available during maintenance.
///
/// For non-Validator nodes the existing user-controlled behaviour is preserved:
/// - If neither `minAvailable` nor `maxUnavailable` is set, defaults to
///   `maxUnavailable: 1`.
/// - Returns `None` when `replicas <= 1` (no PDB needed for single-replica
///   non-validator workloads).
pub(crate) fn build_pdb(node: &StellarNode) -> Option<PodDisruptionBudget> {
    let labels = standard_labels(node);
    let name = node.name_any();

    let (min_available, max_unavailable) = if node.spec.node_type == NodeType::Validator {
        // Auto-calculate quorum-safe minAvailable for Stellar-Core validators.
        let replicas = node.spec.replicas.max(1);
        let min_avail = (replicas / 2) + 1;
        (Some(IntOrString::Int(min_avail)), None)
    } else {
        if node.spec.replicas <= 1 {
            return None;
        }
        if node.spec.min_available.is_none() && node.spec.max_unavailable.is_none() {
            (None, Some(IntOrString::Int(1)))
        } else {
            (
                node.spec.min_available.clone(),
                node.spec.max_unavailable.clone(),
            )
        }
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
    // For non-Validator nodes with replicas <= 1, delete any existing PDB.
    if node.spec.node_type != NodeType::Validator && node.spec.replicas <= 1 {
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

