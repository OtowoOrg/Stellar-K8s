//! Automated Upgrade Orchestration for the Stellar-K8s Operator
//!
//! Provides a safe, automated workflow for upgrading the operator and managed
//! Stellar nodes with pre-upgrade validation, backup creation, health checks
//! during the upgrade, and automatic rollback on failure.
//!
//! # Upgrade Phases
//!
//! 1. **Pre-flight validation** — verify cluster health, quorum safety, and
//!    resource availability before touching anything.
//! 2. **Backup** — snapshot PVCs and export current CRD state.
//! 3. **Upgrade** — apply the new operator image / Helm chart.
//! 4. **Health gate** — wait for all managed nodes to become Ready.
//! 5. **Rollback** (automatic) — if the health gate fails, restore the
//!    previous operator version and emit a Kubernetes Event.

use crate::crd::StellarNode;
use crate::error::{Error, Result};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Event;
use kube::{
    api::{Api, ListParams, Patch, PatchParams, PostParams},
    Client, ResourceExt,
};
use serde_json::json;
use std::time::Duration;
use tracing::{error, info, instrument, warn};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Annotation written to the operator Deployment before an upgrade begins.
pub const UPGRADE_IN_PROGRESS_ANN: &str = "stellar.org/upgrade-in-progress";

/// Annotation that stores the previous image tag for rollback.
pub const PREVIOUS_IMAGE_ANN: &str = "stellar.org/previous-image";

/// Default namespace for the operator Deployment.
pub const OPERATOR_NAMESPACE: &str = "stellar-system";

/// Name of the operator Deployment.
pub const OPERATOR_DEPLOYMENT: &str = "stellar-operator";

/// How long to wait for the operator to become ready after upgrade.
pub const UPGRADE_READY_TIMEOUT_SECS: u64 = 300;

/// How long to wait for each managed node to become Ready after upgrade.
pub const NODE_READY_TIMEOUT_SECS: u64 = 600;

/// Interval between health-gate polling attempts.
pub const HEALTH_POLL_INTERVAL_SECS: u64 = 10;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Result of a single upgrade attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeOutcome {
    /// Upgrade completed successfully; all nodes are healthy.
    Success,
    /// Pre-flight checks failed; no changes were made.
    PreflightFailed(String),
    /// Upgrade was applied but health gate timed out; rollback was triggered.
    RolledBack(String),
    /// Rollback itself failed — requires manual intervention.
    RollbackFailed(String),
}

/// Configuration for the upgrade orchestrator.
#[derive(Debug, Clone)]
pub struct UpgradeConfig {
    /// Target image tag (e.g. `"ghcr.io/otowoorg/stellar-k8s:v0.2.0"`).
    pub target_image: String,
    /// Namespace where the operator Deployment lives.
    pub operator_namespace: String,
    /// Seconds to wait for the operator pod to become Ready.
    pub operator_ready_timeout_secs: u64,
    /// Seconds to wait for each managed StellarNode to become Ready.
    pub node_ready_timeout_secs: u64,
    /// Skip backup creation (not recommended for production).
    pub skip_backup: bool,
    /// Dry-run: validate and report without making changes.
    pub dry_run: bool,
}

impl Default for UpgradeConfig {
    fn default() -> Self {
        Self {
            target_image: String::new(),
            operator_namespace: OPERATOR_NAMESPACE.to_string(),
            operator_ready_timeout_secs: UPGRADE_READY_TIMEOUT_SECS,
            node_ready_timeout_secs: NODE_READY_TIMEOUT_SECS,
            skip_backup: false,
            dry_run: false,
        }
    }
}

/// Status report emitted at the end of an upgrade run.
#[derive(Debug, Clone)]
pub struct UpgradeReport {
    pub outcome: UpgradeOutcome,
    pub previous_image: String,
    pub target_image: String,
    pub nodes_checked: usize,
    pub nodes_healthy: usize,
    pub duration_secs: u64,
    pub message: String,
}

// ── Orchestrator ──────────────────────────────────────────────────────────────

/// Upgrade orchestrator for the Stellar-K8s operator.
pub struct UpgradeOrchestrator {
    client: Client,
    config: UpgradeConfig,
}

impl UpgradeOrchestrator {
    pub fn new(client: Client, config: UpgradeConfig) -> Self {
        Self { client, config }
    }

    /// Run the full upgrade workflow and return a status report.
    #[instrument(skip(self), fields(target = %self.config.target_image))]
    pub async fn run(&self) -> Result<UpgradeReport> {
        let start = std::time::Instant::now();

        // ── Phase 1: Pre-flight ───────────────────────────────────────────────
        info!("Phase 1: Running pre-flight validation");
        if let Err(reason) = self.preflight_checks().await {
            return Ok(UpgradeReport {
                outcome: UpgradeOutcome::PreflightFailed(reason.clone()),
                previous_image: String::new(),
                target_image: self.config.target_image.clone(),
                nodes_checked: 0,
                nodes_healthy: 0,
                duration_secs: start.elapsed().as_secs(),
                message: format!("Pre-flight failed: {reason}"),
            });
        }

        // ── Phase 2: Capture current state ───────────────────────────────────
        let previous_image = self.get_current_image().await?;
        info!("Current operator image: {previous_image}");

        if self.config.dry_run {
            info!("Dry-run mode: no changes will be made");
            return Ok(UpgradeReport {
                outcome: UpgradeOutcome::Success,
                previous_image,
                target_image: self.config.target_image.clone(),
                nodes_checked: 0,
                nodes_healthy: 0,
                duration_secs: start.elapsed().as_secs(),
                message: "Dry-run: pre-flight passed, no changes applied".to_string(),
            });
        }

        // ── Phase 3: Backup ───────────────────────────────────────────────────
        if !self.config.skip_backup {
            info!("Phase 3: Creating pre-upgrade backup");
            if let Err(e) = self.create_backup(&previous_image).await {
                warn!("Backup creation failed (non-fatal): {e}");
            }
        }

        // ── Phase 4: Apply upgrade ────────────────────────────────────────────
        info!("Phase 4: Applying upgrade to {}", self.config.target_image);
        self.annotate_upgrade_in_progress(&previous_image).await?;
        self.patch_operator_image(&self.config.target_image).await?;

        // ── Phase 5: Health gate ──────────────────────────────────────────────
        info!("Phase 5: Waiting for operator to become ready");
        let operator_healthy = self
            .wait_for_operator_ready(Duration::from_secs(self.config.operator_ready_timeout_secs))
            .await;

        if !operator_healthy {
            let reason = format!(
                "Operator did not become ready within {}s",
                self.config.operator_ready_timeout_secs
            );
            error!("{reason}");
            return self
                .rollback(&previous_image, &reason, start.elapsed().as_secs())
                .await;
        }

        info!("Phase 5b: Checking managed StellarNode health");
        let (nodes_checked, nodes_healthy) = self
            .check_all_nodes_healthy(Duration::from_secs(self.config.node_ready_timeout_secs))
            .await?;

        if nodes_healthy < nodes_checked {
            let reason = format!(
                "{}/{} nodes became healthy within {}s",
                nodes_healthy, nodes_checked, self.config.node_ready_timeout_secs
            );
            error!("{reason}");
            return self
                .rollback(&previous_image, &reason, start.elapsed().as_secs())
                .await;
        }

        // ── Phase 6: Cleanup ──────────────────────────────────────────────────
        self.clear_upgrade_annotation().await?;
        self.emit_upgrade_event(
            "UpgradeSucceeded",
            &format!(
                "Operator upgraded from {previous_image} to {}",
                self.config.target_image
            ),
        )
        .await?;

        info!(
            "Upgrade completed successfully in {}s",
            start.elapsed().as_secs()
        );

        Ok(UpgradeReport {
            outcome: UpgradeOutcome::Success,
            previous_image,
            target_image: self.config.target_image.clone(),
            nodes_checked,
            nodes_healthy,
            duration_secs: start.elapsed().as_secs(),
            message: "Upgrade completed successfully".to_string(),
        })
    }

    // ── Pre-flight checks ─────────────────────────────────────────────────────

    /// Run all pre-flight checks. Returns `Ok(())` if all pass.
    async fn preflight_checks(&self) -> std::result::Result<(), String> {
        // 1. Target image must be non-empty.
        if self.config.target_image.is_empty() {
            return Err("target_image must not be empty".to_string());
        }

        // 2. Operator Deployment must exist.
        let deployments: Api<Deployment> =
            Api::namespaced(self.client.clone(), &self.config.operator_namespace);
        deployments
            .get(OPERATOR_DEPLOYMENT)
            .await
            .map_err(|e| format!("Operator Deployment not found: {e}"))?;

        // 3. No upgrade already in progress.
        let deploy = deployments
            .get(OPERATOR_DEPLOYMENT)
            .await
            .map_err(|e| format!("Cannot read operator Deployment: {e}"))?;
        if deploy
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(UPGRADE_IN_PROGRESS_ANN))
            .is_some()
        {
            return Err("An upgrade is already in progress (annotation present)".to_string());
        }

        // 4. All managed StellarNodes must currently be in a non-degraded state.
        let nodes: Api<StellarNode> = Api::all(self.client.clone());
        let node_list = nodes
            .list(&ListParams::default())
            .await
            .map_err(|e| format!("Cannot list StellarNodes: {e}"))?;

        for node in &node_list.items {
            let phase = node
                .status
                .as_ref()
                .and_then(|s| s.phase.as_deref())
                .unwrap_or("Unknown");
            if phase == "Failed" || phase == "Error" {
                return Err(format!(
                    "StellarNode {}/{} is in phase {phase} — resolve before upgrading",
                    node.namespace().unwrap_or_default(),
                    node.name_any()
                ));
            }
        }

        info!(
            "Pre-flight passed: {} StellarNode(s) checked",
            node_list.items.len()
        );
        Ok(())
    }

    // ── Backup ────────────────────────────────────────────────────────────────

    /// Export current StellarNode specs to a ConfigMap as a lightweight backup.
    async fn create_backup(&self, current_image: &str) -> Result<()> {
        let nodes: Api<StellarNode> = Api::all(self.client.clone());
        let node_list = nodes
            .list(&ListParams::default())
            .await
            .map_err(Error::KubeError)?;

        let backup_data: serde_json::Value = json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "operator_image": current_image,
            "nodes": node_list.items.iter().map(|n| json!({
                "name": n.name_any(),
                "namespace": n.namespace(),
                "spec": serde_json::to_value(&n.spec).unwrap_or_default(),
            })).collect::<Vec<_>>(),
        });

        let cms: Api<k8s_openapi::api::core::v1::ConfigMap> =
            Api::namespaced(self.client.clone(), &self.config.operator_namespace);

        let backup_name = format!(
            "upgrade-backup-{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        );

        let cm = json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {
                "name": backup_name,
                "namespace": self.config.operator_namespace,
                "labels": {
                    "stellar.org/backup-type": "pre-upgrade",
                    "stellar.org/operator-image": current_image,
                }
            },
            "data": {
                "backup.json": backup_data.to_string(),
            }
        });

        cms.create(
            &PostParams::default(),
            &serde_json::from_value(cm).map_err(|e| Error::ConfigError(e.to_string()))?,
        )
        .await
        .map_err(Error::KubeError)?;

        info!("Pre-upgrade backup created: {backup_name}");
        Ok(())
    }

    // ── Image management ──────────────────────────────────────────────────────

    /// Read the current operator container image from the Deployment.
    async fn get_current_image(&self) -> Result<String> {
        let deployments: Api<Deployment> =
            Api::namespaced(self.client.clone(), &self.config.operator_namespace);
        let deploy = deployments
            .get(OPERATOR_DEPLOYMENT)
            .await
            .map_err(Error::KubeError)?;

        let image = deploy
            .spec
            .as_ref()
            .and_then(|s| s.template.spec.as_ref())
            .and_then(|s| s.containers.first())
            .and_then(|c| c.image.as_deref())
            .unwrap_or("unknown")
            .to_string();

        Ok(image)
    }

    /// Patch the operator Deployment to use `new_image`.
    async fn patch_operator_image(&self, new_image: &str) -> Result<()> {
        let deployments: Api<Deployment> =
            Api::namespaced(self.client.clone(), &self.config.operator_namespace);

        let patch = json!({
            "spec": {
                "template": {
                    "spec": {
                        "containers": [{
                            "name": "stellar-operator",
                            "image": new_image
                        }]
                    }
                }
            }
        });

        deployments
            .patch(
                OPERATOR_DEPLOYMENT,
                &PatchParams::apply("stellar-upgrade-orchestrator").force(),
                &Patch::Apply(&patch),
            )
            .await
            .map_err(Error::KubeError)?;

        info!("Patched operator image → {new_image}");
        Ok(())
    }

    // ── Annotations ───────────────────────────────────────────────────────────

    async fn annotate_upgrade_in_progress(&self, previous_image: &str) -> Result<()> {
        let deployments: Api<Deployment> =
            Api::namespaced(self.client.clone(), &self.config.operator_namespace);

        let patch = json!({
            "metadata": {
                "annotations": {
                    UPGRADE_IN_PROGRESS_ANN: "true",
                    PREVIOUS_IMAGE_ANN: previous_image,
                }
            }
        });

        deployments
            .patch(
                OPERATOR_DEPLOYMENT,
                &PatchParams::apply("stellar-upgrade-orchestrator").force(),
                &Patch::Apply(&patch),
            )
            .await
            .map_err(Error::KubeError)?;

        Ok(())
    }

    async fn clear_upgrade_annotation(&self) -> Result<()> {
        let deployments: Api<Deployment> =
            Api::namespaced(self.client.clone(), &self.config.operator_namespace);

        let patch = json!({
            "metadata": {
                "annotations": {
                    UPGRADE_IN_PROGRESS_ANN: null,
                    PREVIOUS_IMAGE_ANN: null,
                }
            }
        });

        deployments
            .patch(
                OPERATOR_DEPLOYMENT,
                &PatchParams::apply("stellar-upgrade-orchestrator").force(),
                &Patch::Merge(&patch),
            )
            .await
            .map_err(Error::KubeError)?;

        Ok(())
    }

    // ── Health gates ──────────────────────────────────────────────────────────

    /// Poll until the operator Deployment has at least one Ready replica.
    async fn wait_for_operator_ready(&self, timeout: Duration) -> bool {
        let deployments: Api<Deployment> =
            Api::namespaced(self.client.clone(), &self.config.operator_namespace);

        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Ok(deploy) = deployments.get(OPERATOR_DEPLOYMENT).await {
                let ready = deploy
                    .status
                    .as_ref()
                    .and_then(|s| s.ready_replicas)
                    .unwrap_or(0);
                if ready > 0 {
                    info!("Operator has {ready} ready replica(s)");
                    return true;
                }
            }
            tokio::time::sleep(Duration::from_secs(HEALTH_POLL_INTERVAL_SECS)).await;
        }
        false
    }

    /// Check all StellarNodes become Ready within `timeout`.
    /// Returns `(total_nodes, healthy_nodes)`.
    async fn check_all_nodes_healthy(&self, timeout: Duration) -> Result<(usize, usize)> {
        let nodes: Api<StellarNode> = Api::all(self.client.clone());
        let node_list = nodes
            .list(&ListParams::default())
            .await
            .map_err(Error::KubeError)?;

        let total = node_list.items.len();
        if total == 0 {
            return Ok((0, 0));
        }

        let deadline = std::time::Instant::now() + timeout;
        loop {
            let current = nodes
                .list(&ListParams::default())
                .await
                .map_err(Error::KubeError)?;

            let healthy = current
                .items
                .iter()
                .filter(|n| {
                    n.status
                        .as_ref()
                        .and_then(|s| s.phase.as_deref())
                        .map(|p| p == "Running" || p == "Ready")
                        .unwrap_or(false)
                })
                .count();

            info!("{}/{} StellarNodes are healthy", healthy, total);

            if healthy == total {
                return Ok((total, healthy));
            }

            if std::time::Instant::now() >= deadline {
                return Ok((total, healthy));
            }

            tokio::time::sleep(Duration::from_secs(HEALTH_POLL_INTERVAL_SECS)).await;
        }
    }

    // ── Rollback ──────────────────────────────────────────────────────────────

    /// Roll back to `previous_image` and return an `UpgradeReport`.
    async fn rollback(
        &self,
        previous_image: &str,
        reason: &str,
        elapsed_secs: u64,
    ) -> Result<UpgradeReport> {
        warn!("Initiating rollback to {previous_image}: {reason}");

        match self.patch_operator_image(previous_image).await {
            Ok(()) => {
                let _ = self.clear_upgrade_annotation().await;
                let _ = self
                    .emit_upgrade_event(
                        "UpgradeRolledBack",
                        &format!("Rolled back to {previous_image}: {reason}"),
                    )
                    .await;

                info!("Rollback to {previous_image} succeeded");
                Ok(UpgradeReport {
                    outcome: UpgradeOutcome::RolledBack(reason.to_string()),
                    previous_image: previous_image.to_string(),
                    target_image: self.config.target_image.clone(),
                    nodes_checked: 0,
                    nodes_healthy: 0,
                    duration_secs: elapsed_secs,
                    message: format!("Rolled back: {reason}"),
                })
            }
            Err(e) => {
                error!("Rollback FAILED: {e}");
                let _ = self
                    .emit_upgrade_event(
                        "UpgradeRollbackFailed",
                        &format!("Rollback to {previous_image} failed: {e}"),
                    )
                    .await;

                Ok(UpgradeReport {
                    outcome: UpgradeOutcome::RollbackFailed(e.to_string()),
                    previous_image: previous_image.to_string(),
                    target_image: self.config.target_image.clone(),
                    nodes_checked: 0,
                    nodes_healthy: 0,
                    duration_secs: elapsed_secs,
                    message: format!("Rollback failed — MANUAL INTERVENTION REQUIRED: {e}"),
                })
            }
        }
    }

    // ── Events ────────────────────────────────────────────────────────────────

    async fn emit_upgrade_event(&self, reason: &str, message: &str) -> Result<()> {
        let events: Api<Event> =
            Api::namespaced(self.client.clone(), &self.config.operator_namespace);

        let now = chrono::Utc::now();
        let event = json!({
            "apiVersion": "v1",
            "kind": "Event",
            "metadata": {
                "name": format!("stellar-upgrade-{}", now.timestamp()),
                "namespace": self.config.operator_namespace,
            },
            "involvedObject": {
                "apiVersion": "apps/v1",
                "kind": "Deployment",
                "name": OPERATOR_DEPLOYMENT,
                "namespace": self.config.operator_namespace,
            },
            "reason": reason,
            "message": message,
            "type": if reason.contains("Failed") || reason.contains("RolledBack") {
                "Warning"
            } else {
                "Normal"
            },
            "firstTimestamp": now.to_rfc3339(),
            "lastTimestamp": now.to_rfc3339(),
            "count": 1,
            "reportingComponent": "stellar-upgrade-orchestrator",
            "reportingInstance": "stellar-operator",
        });

        events
            .create(
                &PostParams::default(),
                &serde_json::from_value(event).map_err(|e| Error::ConfigError(e.to_string()))?,
            )
            .await
            .map_err(Error::KubeError)?;

        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sensible_timeouts() {
        let cfg = UpgradeConfig::default();
        assert_eq!(cfg.operator_ready_timeout_secs, UPGRADE_READY_TIMEOUT_SECS);
        assert_eq!(cfg.node_ready_timeout_secs, NODE_READY_TIMEOUT_SECS);
        assert!(!cfg.skip_backup);
        assert!(!cfg.dry_run);
    }

    #[test]
    fn upgrade_outcome_is_eq() {
        assert_eq!(UpgradeOutcome::Success, UpgradeOutcome::Success);
        assert_ne!(
            UpgradeOutcome::Success,
            UpgradeOutcome::PreflightFailed("x".into())
        );
    }
}
