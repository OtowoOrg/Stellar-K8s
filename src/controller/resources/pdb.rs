//! PodDisruptionBudget management.

use super::prelude::*;
use super::helpers::*;

// ============================================================================
// PodDisruptionBudget
// ============================================================================

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

pub(crate) fn build_pvc(node: &StellarNode, storage_class_name: String) -> PersistentVolumeClaim {
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

