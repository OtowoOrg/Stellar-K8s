//! Background controller that drives dynamic PVC auto-expansion.
//!
//! Spawns a Tokio task that periodically iterates over all `StellarNode`
//! resources and calls [`VolumeResizerController::reconcile_node_pvc`] for
//! each one.  Prometheus metrics are emitted for every expansion attempt.
//!
//! # Usage
//!
//! ```rust,ignore
//! use stellar_k8s::controller::pvc_autoscaler::run_pvc_autoscaler;
//! use stellar_k8s::controller::volume_resizer::VolumeResizerConfig;
//!
//! let config = VolumeResizerConfig::default();
//! tokio::spawn(run_pvc_autoscaler(client.clone(), config));
//! ```

use crate::controller::volume_resizer::{
    ExpansionOutcome, VolumeResizerConfig, VolumeResizerController,
};
use crate::crd::StellarNode;
use crate::error::Result;
use k8s_openapi::api::core::v1::Event;
use kube::{
    api::{Api, ListParams, PostParams},
    Client, ResourceExt,
};
use serde_json::json;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// How often the autoscaler polls all PVCs (default: every 5 minutes).
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 300;

/// Run the PVC autoscaler loop indefinitely.
///
/// This function never returns under normal operation. Spawn it with
/// `tokio::spawn`.
pub async fn run_pvc_autoscaler(client: Client, config: VolumeResizerConfig) {
    let poll_interval = Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS);
    let controller = VolumeResizerController::new(client.clone(), config);

    info!(
        "PVC autoscaler started (poll interval: {}s)",
        DEFAULT_POLL_INTERVAL_SECS
    );

    loop {
        if let Err(e) = reconcile_all_pvcs(&client, &controller).await {
            error!("PVC autoscaler reconcile error: {e}");
        }
        tokio::time::sleep(poll_interval).await;
    }
}

/// Iterate over every StellarNode and attempt PVC expansion where needed.
async fn reconcile_all_pvcs(client: &Client, controller: &VolumeResizerController) -> Result<()> {
    let nodes: Api<StellarNode> = Api::all(client.clone());
    let node_list = nodes
        .list(&ListParams::default())
        .await
        .map_err(crate::error::Error::KubeError)?;

    debug!(
        "PVC autoscaler: checking {} StellarNode(s)",
        node_list.items.len()
    );

    for node in &node_list.items {
        let name = node.name_any();
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

        match controller.reconcile_node_pvc(node).await {
            Ok(ExpansionOutcome::Expanded { new_size_gi }) => {
                info!(
                    "Auto-expanded PVC for {}/{} → {}Gi",
                    namespace, name, new_size_gi
                );
                emit_expansion_event(client, node, new_size_gi).await;
            }
            Ok(ExpansionOutcome::BelowThreshold) => {
                debug!(
                    "{}/{}: disk usage below threshold, no action",
                    namespace, name
                );
            }
            Ok(ExpansionOutcome::InFlight) => {
                debug!("{}/{}: expansion already in-flight", namespace, name);
            }
            Ok(ExpansionOutcome::StorageClassUnsupported) => {
                warn!(
                    "{}/{}: StorageClass does not support expansion — \
                     set allowVolumeExpansion: true to enable auto-expansion",
                    namespace, name
                );
            }
            Ok(ExpansionOutcome::MaxExpansionsReached) => {
                warn!(
                    "{}/{}: maximum auto-expansion count reached — \
                     manual intervention required to increase PVC size further",
                    namespace, name
                );
                emit_quota_event(client, node, "MaxExpansionsReached").await;
            }
            Ok(ExpansionOutcome::TooSoon) => {
                debug!("{}/{}: too soon since last expansion", namespace, name);
            }
            Ok(ExpansionOutcome::QuotaExceeded) => {
                warn!(
                    "{}/{}: storage quota would be exceeded — skipping expansion",
                    namespace, name
                );
                emit_quota_event(client, node, "StorageQuotaExceeded").await;
            }
            Err(e) => {
                error!("Error checking PVC for {}/{}: {e}", namespace, name);
            }
        }
    }

    Ok(())
}

/// Emit a Kubernetes Warning event when a quota or limit is hit.
async fn emit_quota_event(client: &Client, node: &StellarNode, reason: &str) {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let events: Api<Event> = Api::namespaced(client.clone(), &namespace);
    let now = chrono::Utc::now();

    let event = json!({
        "apiVersion": "v1",
        "kind": "Event",
        "metadata": {
            "name": format!("pvc-autoscaler-{}-{}", node.name_any(), now.timestamp()),
            "namespace": namespace,
        },
        "involvedObject": {
            "apiVersion": "stellar.org/v1alpha1",
            "kind": "StellarNode",
            "name": node.name_any(),
            "namespace": namespace,
        },
        "reason": reason,
        "message": format!(
            "PVC auto-expansion blocked for {}: {}. Manual intervention may be required.",
            node.name_any(), reason
        ),
        "type": "Warning",
        "firstTimestamp": now.to_rfc3339(),
        "lastTimestamp": now.to_rfc3339(),
        "count": 1,
        "reportingComponent": "stellar-pvc-autoscaler",
        "reportingInstance": "stellar-operator",
    });

    if let Ok(ev) = serde_json::from_value(event) {
        if let Err(e) = events.create(&PostParams::default(), &ev).await {
            warn!("Failed to emit quota event for {}: {e}", node.name_any());
        }
    }
}

/// Emit a Normal event when a PVC is successfully expanded.
async fn emit_expansion_event(client: &Client, node: &StellarNode, new_size_gi: u64) {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let events: Api<Event> = Api::namespaced(client.clone(), &namespace);
    let now = chrono::Utc::now();

    let event = json!({
        "apiVersion": "v1",
        "kind": "Event",
        "metadata": {
            "name": format!("pvc-expanded-{}-{}", node.name_any(), now.timestamp()),
            "namespace": namespace,
        },
        "involvedObject": {
            "apiVersion": "stellar.org/v1alpha1",
            "kind": "StellarNode",
            "name": node.name_any(),
            "namespace": namespace,
        },
        "reason": "PvcAutoExpanded",
        "message": format!(
            "PVC for {} automatically expanded to {}Gi by the disk autoscaler.",
            node.name_any(), new_size_gi
        ),
        "type": "Normal",
        "firstTimestamp": now.to_rfc3339(),
        "lastTimestamp": now.to_rfc3339(),
        "count": 1,
        "reportingComponent": "stellar-pvc-autoscaler",
        "reportingInstance": "stellar-operator",
    });

    if let Ok(ev) = serde_json::from_value(event) {
        if let Err(e) = events.create(&PostParams::default(), &ev).await {
            warn!(
                "Failed to emit expansion event for {}: {e}",
                node.name_any()
            );
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::volume_resizer::VolumeResizerConfig;

    #[test]
    fn default_poll_interval_is_reasonable() {
        assert!(
            DEFAULT_POLL_INTERVAL_SECS >= 60,
            "poll interval should be at least 60s"
        );
        assert!(
            DEFAULT_POLL_INTERVAL_SECS <= 3600,
            "poll interval should be at most 1h"
        );
    }

    #[test]
    fn volume_resizer_config_defaults() {
        let cfg = VolumeResizerConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.expansion_threshold_pct > 0);
        assert!(cfg.expansion_threshold_pct <= 100);
        assert!(cfg.max_expansions > 0);
    }
}
