//! Zero-downtime ledger snapshots via CSI VolumeSnapshots
//!
//! When a StellarNode (Validator) has `snapshotSchedule` configured, the operator
//! creates Kubernetes VolumeSnapshot resources targeting the node's data PVC. Optionally
//! flushes the Stellar database before the snapshot for consistency, then resumes normal operations.

use std::collections::BTreeMap;
use std::str::FromStr;

use chrono::Utc;
use kube::api::{Api, DeleteParams, DynamicObject, ListParams, Patch, PatchParams, PostParams};
use kube::discovery::ApiResource;
use kube::{Client, ResourceExt};
use tracing::{info, instrument, warn};

use crate::controller::resource_meta::merge_resource_meta;
use crate::controller::resources::{
    owner_reference, resource_name, standard_labels as node_standard_labels,
};
use crate::crd::{SnapshotScheduleConfig, StellarNode};
use crate::error::{Error, Result};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

const REQUEST_SNAPSHOT_ANNOTATION: &str = "stellar.org/request-snapshot";
const LAST_SNAPSHOT_AT_ANNOTATION: &str = "stellar.org/last-snapshot-at";

/// VolumeSnapshot API resource for snapshot.storage.k8s.io/v1
fn volume_snapshot_api_resource() -> ApiResource {
    ApiResource {
        group: "snapshot.storage.k8s.io".to_string(),
        version: "v1".to_string(),
        api_version: "snapshot.storage.k8s.io/v1".to_string(),
        kind: "VolumeSnapshot".to_string(),
        plural: "volumesnapshots".to_string(),
    }
}

/// If configured, optionally flush the Stellar database for consistency, then create a VolumeSnapshot.
/// Caller should only invoke this for Validator nodes with snapshot_schedule set.
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn reconcile_snapshot(
    client: &Client,
    node: &StellarNode,
    config: &SnapshotScheduleConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let pvc_name = resource_name(node, "data");

    // Check if snapshot was requested via annotation (one-shot)
    let request_snapshot = node
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(REQUEST_SNAPSHOT_ANNOTATION))
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    // If schedule is set, check if cron has fired since last snapshot; otherwise react to annotation only.
    let should_snapshot = request_snapshot || schedule_matches_now(config, node);
    if !should_snapshot {
        return Ok(());
    }

    if config.flush_before_snapshot {
        if let Err(e) = request_db_flush(client, node) {
            warn!(
                "Flush before snapshot requested but failed for {}/{}: {}. Proceeding with snapshot (may be crash-consistent).",
                namespace, name, e
            );
        }
    }

    let snapshot_name = format!(
        "{}-data-{}",
        name,
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    create_volume_snapshot(client, node, &snapshot_name, &pvc_name, config).await?;

    // Enforce retention: list snapshots for this node and delete oldest if over limit
    if config.retention_count > 0 {
        prune_old_snapshots(client, node, config.retention_count).await?;
    }

    // Update last-snapshot-at and clear request annotation so we don't snapshot every reconcile
    update_snapshot_annotations(client, node, request_snapshot).await?;

    Ok(())
}

/// Returns true if the cron schedule has fired (next run time is in the past or within 1 minute of now).
fn schedule_matches_now(config: &SnapshotScheduleConfig, node: &StellarNode) -> bool {
    let schedule = match &config.schedule {
        Some(s) if !s.is_empty() => s,
        _ => return false,
    };
    let s = match cron::Schedule::from_str(schedule) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let now = Utc::now();
    let from = node
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(LAST_SNAPSHOT_AT_ANNOTATION))
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| now - chrono::Duration::days(1));
    let next = s.after(&from).next();
    match next {
        Some(t) => t <= now || t.signed_duration_since(now).num_seconds() < 60,
        None => false,
    }
}

/// Request a graceful flush of the Stellar database (if supported).
/// Stellar Core uses SQLite; we could exec into the pod and run PRAGMA checkpoint, or call an HTTP endpoint if available.
fn request_db_flush(_client: &Client, _node: &StellarNode) -> Result<()> {
    // Optional: exec into the pod and run sqlite3 checkpoint, or call stellar-core HTTP.
    // For now we no-op; storage drivers that support consistent snapshots (e.g. CSI with volume snapshot)
    // may not require application flush. Document in user docs.
    Ok(())
}

/// Create a VolumeSnapshot targeting the node's data PVC.
async fn create_volume_snapshot(
    client: &Client,
    node: &StellarNode,
    snapshot_name: &str,
    pvc_name: &str,
    config: &SnapshotScheduleConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api_resource = volume_snapshot_api_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &api_resource);

    let meta = ObjectMeta {
        name: Some(snapshot_name.to_string()),
        namespace: Some(namespace.clone()),
        labels: Some({
            let mut l = node_standard_labels(node);
            l.insert("stellar.org/snapshot-of".to_string(), node.name_any());
            l
        }),
        owner_references: Some(vec![owner_reference(node)]),
        ..Default::default()
    };

    let spec = serde_json::json!({
        "source": {
            "persistentVolumeClaimName": pvc_name
        },
        "volumeSnapshotClassName": config.volume_snapshot_class_name
    });

    let snapshot = DynamicObject {
        types: Some(kube::core::TypeMeta {
            api_version: api_resource.api_version.clone(),
            kind: api_resource.kind.clone(),
        }),
        metadata: merge_resource_meta(meta, &None),
        data: serde_json::json!({
            "spec": spec
        }),
    };

    match api.get(snapshot_name).await {
        Ok(_) => {
            info!("VolumeSnapshot {} already exists", snapshot_name);
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            info!(
                "Creating VolumeSnapshot {} for PVC {}",
                snapshot_name, pvc_name
            );
            api.create(&PostParams::default(), &snapshot).await?;
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}

/// List VolumeSnapshots owned by this node and delete oldest ones if over retention_count.
async fn prune_old_snapshots(
    client: &Client,
    node: &StellarNode,
    retention_count: u32,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api_resource = volume_snapshot_api_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &api_resource);

    let list_params =
        ListParams::default().labels(&format!("stellar.org/snapshot-of={}", node.name_any()));
    let list = api.list(&list_params).await.map_err(Error::KubeError)?;

    let mut items: Vec<_> = list
        .items
        .into_iter()
        .filter_map(|o| {
            let name = o.name_any();
            let created = o.metadata.creation_timestamp.as_ref()?.0.timestamp();
            Some((name, created))
        })
        .collect();
    items.sort_by_key(|(_, t)| *t);

    let to_remove = items.len().saturating_sub(retention_count as usize);
    for (name, _) in items.into_iter().take(to_remove) {
        info!(
            "Pruning old VolumeSnapshot {} (retention limit {})",
            name, retention_count
        );
        let _ = api.delete(&name, &DeleteParams::default()).await;
    }

    Ok(())
}

/// Update last-snapshot-at and optionally clear the request-snapshot annotation.
async fn update_snapshot_annotations(
    client: &Client,
    node: &StellarNode,
    clear_request: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    let mut patch_meta = node.metadata.clone();
    let ann = patch_meta.annotations.get_or_insert_with(BTreeMap::new);
    ann.insert(
        LAST_SNAPSHOT_AT_ANNOTATION.to_string(),
        Utc::now().to_rfc3339(),
    );
    if clear_request {
        ann.remove(REQUEST_SNAPSHOT_ANNOTATION);
    }

    let patch = serde_json::json!({ "metadata": { "annotations": ann } });
    let _ = api
        .patch(
            &name,
            &PatchParams::apply("stellar-operator").force(),
            &Patch::Merge(patch),
        )
        .await;

    Ok(())
}
