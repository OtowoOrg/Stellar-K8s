//! Pruning Reconciler - Integrates pruning worker with main reconciliation loop
//!
//! Handles scheduled and manual pruning of history archives based on StellarNode pruning policies.
//! Coordinates with the pruning_worker for policy validation and the archive_prune module for
//! actual deletion operations.

use chrono::Utc;
use kube::client::Client;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::crd::StellarNode;
use crate::error::Result;

use super::archive_prune::{
    identify_deletable_checkpoints, scan_checkpoints, ArchiveLocation, PruneResult,
};
use super::pruning_worker::PruningWorker;

/// Reconcile pruning policy for a StellarNode
///
/// This function:
/// 1. Checks if pruning is enabled and should run
/// 2. Validates the pruning policy
/// 3. Scans the archive for checkpoints
/// 4. Identifies deletable checkpoints
/// 5. Executes pruning (or dry-run)
/// 6. Updates node status with results
pub async fn reconcile_pruning(
    _client: &Client,
    node: &StellarNode,
) -> Result<Option<PruneResult>> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    // Check if pruning is configured
    let pruning_policy = match &node.spec.pruning_policy {
        Some(policy) => policy,
        None => {
            debug!("No pruning policy configured for {}/{}", namespace, name);
            return Ok(None);
        }
    };

    // Create and validate pruning worker
    let worker = match PruningWorker::new(pruning_policy.clone()) {
        Ok(w) => w,
        Err(e) => {
            error!(
                "Invalid pruning policy for {}/{}: {}",
                namespace, name, e
            );
            return Err(e);
        }
    };

    // Check if pruning should run based on schedule
    let last_run = node
        .status
        .as_ref()
        .and_then(|s| s.pruning_status.as_ref())
        .and_then(|ps| ps.last_run_time.as_ref())
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|dt| dt.with_timezone(&Utc));

    if !worker.should_run_scheduled(last_run) {
        debug!(
            "Pruning not scheduled to run for {}/{} at this time",
            namespace, name
        );
        return Ok(None);
    }

    info!(
        "Running pruning reconciliation for {}/{} (auto_delete={})",
        namespace,
        name,
        worker.auto_delete_enabled()
    );

    // Get history archive URLs from validator config
    let archive_urls = match &node.spec.validator_config {
        Some(config) if !config.history_archive_urls.is_empty() => {
            config.history_archive_urls.clone()
        }
        _ => {
            debug!(
                "No history archives configured for {}/{}, skipping pruning",
                namespace, name
            );
            return Ok(None);
        }
    };

    // Process each archive
    let mut overall_result = None;
    for archive_url in archive_urls {
        match process_archive(&worker, &archive_url, node).await {
            Ok(result) => {
                overall_result = Some(result);
            }
            Err(e) => {
                warn!(
                    "Failed to prune archive {} for {}/{}: {}",
                    archive_url, namespace, name, e
                );
            }
        }
    }

    Ok(overall_result)
}

/// Process a single archive for pruning
async fn process_archive(
    worker: &PruningWorker,
    archive_url: &str,
    node: &StellarNode,
) -> Result<PruneResult> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!(
        "Processing archive {} for {}/{} (dry_run={})",
        archive_url,
        namespace,
        name,
        !worker.auto_delete_enabled()
    );

    // Parse archive location
    let location = ArchiveLocation::from_url(archive_url)?;

    // Scan for checkpoints
    let checkpoints = scan_checkpoints(&location).await?;
    debug!(
        "Found {} checkpoints in archive {}",
        checkpoints.len(),
        archive_url
    );

    if checkpoints.is_empty() {
        info!("No checkpoints found in archive {}", archive_url);
        return Ok(PruneResult {
            total_checkpoints: 0,
            eligible_for_deletion: 0,
            deleted_count: 0,
            retained_count: 0,
            bytes_freed: 0,
            deleted_ledgers: vec![],
            retained_ledgers: vec![],
            errors: vec![],
            dry_run: !worker.auto_delete_enabled(),
        });
    }

    // Identify deletable checkpoints using worker's retention criteria
    let (deletable, retained) = identify_deletable_checkpoints(
        &checkpoints,
        worker.policy().retention_days,
        worker.policy().retention_ledgers,
        worker.policy().min_checkpoints,
        worker.policy().max_age_days,
    )?;

    info!(
        "Archive {}: {} total, {} eligible for deletion, {} will be retained",
        archive_url,
        checkpoints.len(),
        deletable.len(),
        retained.len()
    );

    // Execute pruning (or dry-run)
    let result = if worker.auto_delete_enabled() {
        // Actual deletion
        super::archive_prune::execute_prune(
            deletable,
            &location,
            true, // force=true (we already validated)
            worker.policy().concurrency as usize,
        )
        .await?
    } else {
        // Dry-run mode
        super::archive_prune::execute_prune(
            deletable,
            &location,
            false, // force=false (dry-run)
            worker.policy().concurrency as usize,
        )
        .await?
    };

    Ok(result)
}

/// Update node status with pruning results
pub async fn update_pruning_status(
    client: &kube::client::Client,
    node: &StellarNode,
    result: &PruneResult,
) -> Result<()> {
    use kube::api::{Api, Patch, PatchParams};
    use kube::ResourceExt;

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let pruning_status = crate::crd::types::PruningStatus {
        last_run_time: Some(Utc::now().to_rfc3339()),
        last_run_status: Some(if result.errors.is_empty() {
            "Success".to_string()
        } else {
            "PartialSuccess".to_string()
        }),
        total_checkpoints: Some(result.total_checkpoints as u32),
        deleted_count: Some(result.deleted_count as u32),
        retained_count: Some(result.retained_count as u32),
        bytes_freed: Some(result.bytes_freed),
        message: Some(format!(
            "Pruned {} checkpoints, freed {}",
            result.deleted_count,
            format_bytes(result.bytes_freed)
        )),
        dry_run: Some(result.dry_run),
    };

    let patch = serde_json::json!({
        "status": {
            "pruningStatus": pruning_status
        }
    });

    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(crate::error::Error::KubeError)?;

    Ok(())
}

/// Format bytes into human-readable string
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    match bytes {
        b if b >= TB => format!("{:.2} TB", b as f64 / TB as f64),
        b if b >= GB => format!("{:.2} GB", b as f64 / GB as f64),
        b if b >= MB => format!("{:.2} MB", b as f64 / MB as f64),
        b if b >= KB => format!("{:.2} KB", b as f64 / KB as f64),
        b => format!("{b} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(1073741824), "1.00 GB");
        assert_eq!(format_bytes(500), "500 B");
    }
}
