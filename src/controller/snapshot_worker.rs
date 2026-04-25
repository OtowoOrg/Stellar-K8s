//! Auto-Snapshot Worker
//!
//! A long-running background task that periodically creates CSI VolumeSnapshots
//! for all Validator nodes that have `spec.snapshotSchedule` configured.
//!
//! # Design
//!
//! The worker runs as an independent Tokio task alongside the main reconciliation
//! loop.  It wakes up every `POLL_INTERVAL_SECS` seconds, lists all `StellarNode`
//! resources in the cluster, and for each Validator with a `snapshotSchedule`
//! calls [`super::snapshot::reconcile_snapshot`].
//!
//! This decouples snapshot creation from the per-node reconcile loop so that
//! snapshots are taken even when no spec change triggers a reconcile.
//!
//! # Bootstrap Status Tracking
//!
//! When a node is bootstrapped from a snapshot (`spec.storage.snapshotRef` or
//! `spec.restoreFromSnapshot`), the worker also monitors the node's health and
//! records the time-to-sync in `status.snapshotBootstrap.secondsToSync`.
//!
//! # Acceptance Criterion
//!
//! The worker emits a Kubernetes Warning event if a bootstrapped node has not
//! reached `Synced` state within 10 minutes of the restore completing.

use std::time::Duration;

use chrono::Utc;
use kube::api::{Api, ListParams, Patch, PatchParams};
use kube::runtime::events::{Event as K8sEvent, EventType, Recorder, Reporter};
use kube::{Client, Resource, ResourceExt};
use tracing::{debug, info, instrument, warn};

use crate::controller::health;
use crate::controller::snapshot::reconcile_snapshot;
#[allow(unused_imports)]
use crate::crd::{NodeType, SnapshotBootstrapStatus, StellarNode};
use crate::error::Result;

/// How often the worker wakes up to check snapshot schedules.
const POLL_INTERVAL_SECS: u64 = 60;

/// Maximum seconds from restore completion to first `Synced` state before we
/// emit a warning event.  Corresponds to the "10 minutes" acceptance criterion.
const SYNC_DEADLINE_SECS: u64 = 600;

/// Annotation written by the restore init container when it finishes.
/// The init container should write the RFC3339 timestamp to this annotation
/// via a `kubectl annotate` call or the operator sets it when the pod becomes
/// Running (init containers completed).
pub const RESTORE_COMPLETED_AT_ANNOTATION: &str = "stellar.org/snapshot-restore-completed-at";

/// Annotation set by the operator when bootstrap is first detected.
pub const BOOTSTRAP_STARTED_AT_ANNOTATION: &str = "stellar.org/snapshot-bootstrap-started-at";

/// Run the auto-snapshot worker loop.
///
/// This function never returns under normal operation.  It should be spawned
/// as a background Tokio task:
///
/// ```rust,ignore
/// tokio::spawn(run_snapshot_worker(client.clone(), reporter.clone()));
/// ```
pub async fn run_snapshot_worker(client: Client, reporter: Reporter) {
    info!(
        "Auto-snapshot worker started (poll interval: {}s)",
        POLL_INTERVAL_SECS
    );

    loop {
        if let Err(e) = tick(&client, &reporter).await {
            warn!("Auto-snapshot worker tick error: {}", e);
        }
        tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
    }
}

/// One iteration of the worker loop.
async fn tick(client: &Client, reporter: &Reporter) -> Result<()> {
    let nodes: Api<StellarNode> = Api::all(client.clone());
    let list = nodes.list(&ListParams::default()).await?;

    for node in list.items {
        // Only process Validator nodes
        if node.spec.node_type != NodeType::Validator {
            continue;
        }

        // --- Auto-snapshot: run scheduled VolumeSnapshot creation ---
        if let Some(ref snapshot_config) = node.spec.snapshot_schedule {
            if let Err(e) = reconcile_snapshot(client, &node, snapshot_config).await {
                warn!(
                    "Auto-snapshot worker: snapshot failed for {}/{}: {}",
                    node.namespace().unwrap_or_default(),
                    node.name_any(),
                    e
                );
            }
        }

        // --- Bootstrap tracking: monitor nodes started from a snapshot ---
        let is_bootstrap_node =
            node.spec.storage.snapshot_ref.is_some() || node.spec.restore_from_snapshot.is_some();

        if is_bootstrap_node {
            if let Err(e) = reconcile_bootstrap_status(client, reporter, &node).await {
                warn!(
                    "Auto-snapshot worker: bootstrap status update failed for {}/{}: {}",
                    node.namespace().unwrap_or_default(),
                    node.name_any(),
                    e
                );
            }
        }
    }

    Ok(())
}

/// Update `status.snapshotBootstrap` for a node that was started from a snapshot.
///
/// State machine:
/// ```text
/// Pending → Restoring (init container running)
///         → Restored  (init container done, pod Running)
///         → Syncing   (node healthy but not yet synced)
///         → Synced    (node healthy and synced)
///         → Failed    (sync deadline exceeded)
/// ```
#[instrument(skip(client, reporter, node), fields(name = %node.name_any(), namespace = node.namespace()))]
async fn reconcile_bootstrap_status(
    client: &Client,
    reporter: &Reporter,
    node: &StellarNode,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    // Determine the snapshot source for display
    let source = node
        .spec
        .storage
        .snapshot_ref
        .as_ref()
        .and_then(|r| {
            r.volume_snapshot_name
                .as_deref()
                .map(|s| s.to_string())
                .or_else(|| r.backup_url.clone())
        })
        .or_else(|| {
            node.spec
                .restore_from_snapshot
                .as_ref()
                .map(|r| r.volume_snapshot_name.clone())
        })
        .unwrap_or_else(|| "unknown".to_string());

    // Read current bootstrap status
    let current = node
        .status
        .as_ref()
        .and_then(|s| s.snapshot_bootstrap.as_ref())
        .cloned()
        .unwrap_or_default();

    // If already in a terminal state, nothing to do
    if current.phase == "Synced" || current.phase == "Failed" {
        return Ok(());
    }

    // Check node health
    let health = health::check_node_health(client, node, None).await;

    let now = Utc::now();
    let now_str = now.to_rfc3339();

    let mut updated = current.clone();
    updated.source = Some(source.clone());

    match health {
        Ok(h) if h.synced => {
            // Node has reached Synced state
            if updated.phase != "Synced" {
                updated.phase = "Synced".to_string();
                updated.synced_at = Some(now_str.clone());
                updated.message =
                    Some("Node reached Synced state after snapshot bootstrap".to_string());

                // Calculate time-to-sync
                if let Some(ref restore_done_str) = updated.restore_completed_at {
                    if let Ok(restore_done) = chrono::DateTime::parse_from_rfc3339(restore_done_str)
                    {
                        let elapsed = now
                            .signed_duration_since(restore_done.with_timezone(&Utc))
                            .num_seconds()
                            .max(0) as u64;
                        updated.seconds_to_sync = Some(elapsed);

                        if elapsed <= SYNC_DEADLINE_SECS {
                            info!(
                                "Bootstrap success for {}/{}: synced in {}s (≤{}s deadline)",
                                namespace, name, elapsed, SYNC_DEADLINE_SECS
                            );
                            emit_bootstrap_event(
                                client,
                                reporter,
                                node,
                                EventType::Normal,
                                "SnapshotBootstrapSynced",
                                &format!(
                                    "Node synced in {}s after snapshot restore from '{}' (deadline: {}s)",
                                    elapsed, source, SYNC_DEADLINE_SECS
                                ),
                            )
                            .await;
                        } else {
                            warn!(
                                "Bootstrap for {}/{} synced but exceeded deadline: {}s > {}s",
                                namespace, name, elapsed, SYNC_DEADLINE_SECS
                            );
                            emit_bootstrap_event(
                                client,
                                reporter,
                                node,
                                EventType::Warning,
                                "SnapshotBootstrapSlowSync",
                                &format!(
                                    "Node synced in {}s after snapshot restore, exceeding the {}s deadline. \
                                     Consider using a more recent snapshot.",
                                    elapsed, SYNC_DEADLINE_SECS
                                ),
                            )
                            .await;
                        }
                    }
                } else {
                    // No restore_completed_at recorded — set it now as a best-effort
                    updated.restore_completed_at = Some(now_str.clone());
                    updated.seconds_to_sync = Some(0);
                }
            }
        }
        Ok(h) if h.healthy => {
            // Node is running but not yet synced
            updated.phase = "Syncing".to_string();
            updated.message = Some(format!("Node is running, waiting for sync: {}", h.message));

            // Check if we've exceeded the sync deadline
            if let Some(ref restore_done_str) = updated.restore_completed_at {
                if let Ok(restore_done) = chrono::DateTime::parse_from_rfc3339(restore_done_str) {
                    let elapsed = now
                        .signed_duration_since(restore_done.with_timezone(&Utc))
                        .num_seconds()
                        .max(0) as u64;

                    if elapsed > SYNC_DEADLINE_SECS && updated.phase != "Failed" {
                        updated.phase = "Failed".to_string();
                        updated.message = Some(format!(
                            "Node did not reach Synced state within {}s of snapshot restore. \
                             Elapsed: {}s. Consider using a more recent snapshot.",
                            SYNC_DEADLINE_SECS, elapsed
                        ));
                        emit_bootstrap_event(
                            client,
                            reporter,
                            node,
                            EventType::Warning,
                            "SnapshotBootstrapDeadlineExceeded",
                            updated.message.as_deref().unwrap_or(""),
                        )
                        .await;
                    }
                }
            }
        }
        Ok(_) => {
            // Node is not healthy yet — still restoring or starting
            if updated.phase.is_empty() || updated.phase == "Pending" {
                updated.phase = "Restoring".to_string();
                updated
                    .restore_started_at
                    .get_or_insert_with(|| now_str.clone());
                updated.message =
                    Some("Waiting for snapshot restore init container to complete".to_string());
            }
        }
        Err(e) => {
            debug!(
                "Health check failed for bootstrapping node {}/{}: {}",
                namespace, name, e
            );
            if updated.phase.is_empty() {
                updated.phase = "Pending".to_string();
                updated
                    .restore_started_at
                    .get_or_insert_with(|| now_str.clone());
                updated.message = Some("Waiting for pod to start".to_string());
            }
        }
    }

    // Check for restore completion via annotation (set by init container or operator)
    if updated.restore_completed_at.is_none() {
        let restore_done = node
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(RESTORE_COMPLETED_AT_ANNOTATION))
            .cloned();
        if let Some(ts) = restore_done {
            updated.restore_completed_at = Some(ts);
            if updated.phase == "Restoring" {
                updated.phase = "Restored".to_string();
                updated.message = Some("Snapshot restore completed, waiting for sync".to_string());
            }
        }
    }

    // Only patch if something changed
    if updated != current {
        let patch = serde_json::json!({
            "status": {
                "snapshotBootstrap": updated
            }
        });
        if let Err(e) = api
            .patch_status(
                &name,
                &PatchParams::apply("stellar-operator"),
                &Patch::Merge(&patch),
            )
            .await
        {
            warn!(
                "Failed to patch snapshotBootstrap status for {}/{}: {}",
                namespace, name, e
            );
        }
    }

    Ok(())
}

/// Emit a Kubernetes Event on the StellarNode for bootstrap lifecycle events.
async fn emit_bootstrap_event(
    client: &Client,
    reporter: &Reporter,
    node: &StellarNode,
    event_type: EventType,
    reason: &str,
    note: &str,
) {
    let recorder = Recorder::new(client.clone(), reporter.clone(), node.object_ref(&()));
    if let Err(e) = recorder
        .publish(K8sEvent {
            type_: event_type,
            reason: reason.to_string(),
            action: "SnapshotBootstrap".to_string(),
            note: Some(note.to_string()),
            secondary: None,
        })
        .await
    {
        warn!("Failed to emit bootstrap event '{}': {}", reason, e);
    }
}
