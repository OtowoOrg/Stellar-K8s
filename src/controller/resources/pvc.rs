//! PersistentVolumeClaim management.

use super::helpers::*;
use super::pdb::{build_pvc, pvc_needs_update, resolve_pvc_storage_class};
use super::prelude::*;

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
