//! GitOps-driven protocol upgrade orchestration.
//!
//! This controller models protocol-version timelines and emits deterministic
//! GitOps synchronization intents for ArgoCD or Flux. It also provides an
//! automatic rollback hook when consensus health degrades after an upgrade.

use crate::controller::cve::{rollback_version, ConsensusHealthMonitor};
use crate::crd::StellarNode;
use crate::error::{Error, Result};
use kube::api::{Patch, PatchParams};
use kube::{Api, Client, ResourceExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;

/// Supported GitOps engines.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GitOpsEngine {
    ArgoCd,
    Flux,
}

impl std::fmt::Display for GitOpsEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArgoCd => write!(f, "argocd"),
            Self::Flux => write!(f, "flux"),
        }
    }
}

/// A protocol upgrade step in a timeline.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolUpgradeStep {
    /// Stellar protocol version to activate.
    pub protocol_version: u32,
    /// Unix timestamp when this step becomes eligible.
    pub activate_at_unix: i64,
    /// Git reference (branch/tag/commit/path) to sync.
    pub config_ref: String,
}

/// Ordered protocol upgrade timeline.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolUpgradeTimeline {
    pub steps: Vec<ProtocolUpgradeStep>,
}

/// Materialized plan emitted by the controller.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitOpsUpgradePlan {
    pub target_protocol: u32,
    pub config_ref: String,
    pub sync_annotations: BTreeMap<String, String>,
}

/// Controller for timeline-aware, GitOps-synchronized upgrades.
#[derive(Clone, Debug)]
pub struct GitOpsUpgradeController {
    pub engine: GitOpsEngine,
    pub sync_timeout: Duration,
    pub consensus_health_threshold: f64,
}

impl Default for GitOpsUpgradeController {
    fn default() -> Self {
        Self {
            engine: GitOpsEngine::ArgoCd,
            sync_timeout: Duration::from_secs(300),
            consensus_health_threshold: 0.95,
        }
    }
}

impl GitOpsUpgradeController {
    pub fn new(
        engine: GitOpsEngine,
        sync_timeout: Duration,
        consensus_health_threshold: f64,
    ) -> Self {
        Self {
            engine,
            sync_timeout,
            consensus_health_threshold,
        }
    }

    /// Select the next protocol step that is due for execution.
    pub fn next_due_step(
        &self,
        timeline: &ProtocolUpgradeTimeline,
        current_protocol: u32,
        now_unix: i64,
    ) -> Option<ProtocolUpgradeStep> {
        timeline
            .steps
            .iter()
            .filter(|step| {
                step.protocol_version > current_protocol && step.activate_at_unix <= now_unix
            })
            .min_by_key(|step| step.protocol_version)
            .cloned()
    }

    /// Build engine-specific synchronization annotations for a step.
    pub fn build_sync_annotations(
        &self,
        node: &StellarNode,
        step: &ProtocolUpgradeStep,
    ) -> BTreeMap<String, String> {
        let mut annotations = BTreeMap::new();
        annotations.insert(
            "stellar.org/protocol-target".to_string(),
            step.protocol_version.to_string(),
        );
        annotations.insert(
            "stellar.org/protocol-config-ref".to_string(),
            step.config_ref.clone(),
        );
        annotations.insert(
            "stellar.org/upgrade-sync-timeout-seconds".to_string(),
            self.sync_timeout.as_secs().to_string(),
        );
        annotations.insert(
            "stellar.org/upgrade-managed-by".to_string(),
            self.engine.to_string(),
        );
        annotations.insert("stellar.org/upgrade-node".to_string(), node.name_any());

        match self.engine {
            GitOpsEngine::ArgoCd => {
                annotations.insert(
                    "argocd.argoproj.io/sync-wave".to_string(),
                    step.protocol_version.to_string(),
                );
                annotations.insert(
                    "argocd.argoproj.io/sync-options".to_string(),
                    "ApplyOutOfSyncOnly=true,PruneLast=true".to_string(),
                );
            }
            GitOpsEngine::Flux => {
                annotations.insert(
                    "kustomize.toolkit.fluxcd.io/force".to_string(),
                    "enabled".to_string(),
                );
                annotations.insert(
                    "reconcile.fluxcd.io/requestedAt".to_string(),
                    step.activate_at_unix.to_string(),
                );
            }
        }

        annotations
    }

    /// Apply timeline-driven GitOps synchronization intent to the StellarNode.
    pub async fn plan_and_sync(
        &self,
        client: &Client,
        node: &StellarNode,
        timeline: &ProtocolUpgradeTimeline,
        current_protocol: u32,
        now_unix: i64,
    ) -> Result<Option<GitOpsUpgradePlan>> {
        let Some(step) = self.next_due_step(timeline, current_protocol, now_unix) else {
            return Ok(None);
        };

        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let name = node.name_any();
        let annotations = self.build_sync_annotations(node, &step);

        let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
        let patch = json!({
            "metadata": {
                "annotations": annotations
            }
        });

        api.patch(
            &name,
            &PatchParams::apply("stellar-gitops-upgrade-controller"),
            &Patch::Merge(&patch),
        )
        .await
        .map_err(Error::KubeError)?;

        Ok(Some(GitOpsUpgradePlan {
            target_protocol: step.protocol_version,
            config_ref: step.config_ref,
            sync_annotations: self.build_sync_annotations(node, &step),
        }))
    }

    /// Roll back to a previous version when consensus health degrades.
    pub async fn rollback_on_consensus_failure(
        &self,
        client: &Client,
        node: &StellarNode,
        baseline_health: f64,
        previous_version: &str,
        reason: &str,
    ) -> Result<bool> {
        let degraded = ConsensusHealthMonitor::detect_degradation(
            client,
            node,
            baseline_health,
            self.consensus_health_threshold,
        )
        .await?;

        if !degraded {
            return Ok(false);
        }

        rollback_version(client, node, previous_version, reason).await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::StellarNodeSpec;

    fn sample_timeline() -> ProtocolUpgradeTimeline {
        ProtocolUpgradeTimeline {
            steps: vec![
                ProtocolUpgradeStep {
                    protocol_version: 21,
                    activate_at_unix: 1_700_000_000,
                    config_ref: "main@sha256:aaa".to_string(),
                },
                ProtocolUpgradeStep {
                    protocol_version: 22,
                    activate_at_unix: 1_800_000_000,
                    config_ref: "main@sha256:bbb".to_string(),
                },
            ],
        }
    }

    #[test]
    fn picks_next_due_protocol_step() {
        let controller = GitOpsUpgradeController::default();
        let step = controller
            .next_due_step(&sample_timeline(), 20, 1_750_000_000)
            .expect("expected due step");

        assert_eq!(step.protocol_version, 21);
    }

    #[test]
    fn skips_future_steps() {
        let controller = GitOpsUpgradeController::default();
        let step = controller.next_due_step(&sample_timeline(), 21, 1_750_000_000);
        assert!(step.is_none());
    }

    #[test]
    fn emits_argocd_annotations() {
        let mut node = StellarNode::new("validator-1", StellarNodeSpec::default());
        node.spec.version = "v21.0.0".to_string();

        let controller =
            GitOpsUpgradeController::new(GitOpsEngine::ArgoCd, Duration::from_secs(120), 0.95);

        let step = ProtocolUpgradeStep {
            protocol_version: 21,
            activate_at_unix: 1_700_000_000,
            config_ref: "main@sha256:aaa".to_string(),
        };

        let ann = controller.build_sync_annotations(&node, &step);
        assert_eq!(
            ann.get("stellar.org/protocol-target"),
            Some(&"21".to_string())
        );
        assert_eq!(
            ann.get("argocd.argoproj.io/sync-wave"),
            Some(&"21".to_string())
        );
    }
}
