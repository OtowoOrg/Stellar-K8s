//! Automated Horizon-to-Soroban-RPC Migration Controller
//!
//! Implements issue #242: zero-downtime migration from `nodeType: Horizon` to
//! `nodeType: SorobanRpc`. The migration runs the two node types **in parallel**
//! during the transition so traffic can be shifted without a gap in service.
//!
//! # Migration Phases
//!
//! ```text
//! Pending → Provisioning → WaitingForSorobanReady → TrafficShifting
//!         → DrainHorizon → Cleanup → Completed
//!                                 ↘ Failed (any phase can roll back here)
//! ```
//!
//! # Annotation-Driven Trigger
//!
//! Operators trigger the migration by annotating the `StellarNode`:
//!
//! ```yaml
//! metadata:
//!   annotations:
//!     stellar.org/migrate-to: soroban-rpc
//! ```
//!
//! The controller detects this annotation, creates a parallel SorobanRpc
//! `Deployment`, waits for it to become Ready, shifts the Service selector,
//! drains and deletes the legacy Horizon workload, then removes the annotation.

use std::time::Duration;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Service;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::crd::{NodeType, StellarNode};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during a Horizon→SorobanRpc migration.
#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("Kubernetes API error: {0}")]
    Kube(#[from] kube::Error),
    #[error("Migration precondition failed: {0}")]
    Precondition(String),
    #[error("Migration timed out after {0}s")]
    Timeout(u64),
    #[error("Migration rolled back: {0}")]
    RolledBack(String),
}

// ---------------------------------------------------------------------------
// Phase tracking
// ---------------------------------------------------------------------------

/// Ordered phases of the Horizon→SorobanRpc migration state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationPhase {
    /// Waiting to start — annotation detected, validation pending.
    Pending,
    /// SorobanRpc `Deployment` is being provisioned alongside Horizon.
    Provisioning,
    /// Waiting for the new SorobanRpc pods to pass their readiness probe.
    WaitingForSorobanReady,
    /// Service selector is being updated to point to SorobanRpc.
    TrafficShifting,
    /// Legacy Horizon `Deployment` replicas are being scaled to zero.
    DrainHorizon,
    /// Horizon workload and migration annotation are being removed.
    Cleanup,
    /// Migration complete; node is now fully SorobanRpc.
    Completed,
    /// An unrecoverable error occurred; original Horizon workload preserved.
    Failed,
}

impl std::fmt::Display for MigrationPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "Pending"),
            Self::Provisioning => write!(f, "Provisioning"),
            Self::WaitingForSorobanReady => write!(f, "WaitingForSorobanReady"),
            Self::TrafficShifting => write!(f, "TrafficShifting"),
            Self::DrainHorizon => write!(f, "DrainHorizon"),
            Self::Cleanup => write!(f, "Cleanup"),
            Self::Completed => write!(f, "Completed"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration & state
// ---------------------------------------------------------------------------

/// Configuration for a single Horizon→SorobanRpc migration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationConfig {
    /// Namespace of the `StellarNode` being migrated.
    pub namespace: String,
    /// Name of the source `StellarNode` (currently `nodeType: Horizon`).
    pub node_name: String,
    /// How long to wait for the SorobanRpc pods to become Ready (seconds).
    pub readiness_timeout_secs: u64,
    /// How long to allow the Horizon drain to complete (seconds).
    pub drain_timeout_secs: u64,
    /// When `true`, roll back and preserve Horizon if any phase fails.
    pub rollback_on_failure: bool,
}

impl MigrationConfig {
    /// Create a config with sensible defaults.
    pub fn new(namespace: impl Into<String>, node_name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            node_name: node_name.into(),
            readiness_timeout_secs: 600, // 10 minutes
            drain_timeout_secs: 300,     // 5 minutes
            rollback_on_failure: true,
        }
    }
}

/// Point-in-time snapshot of migration progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationState {
    pub phase: MigrationPhase,
    pub soroban_deployment_name: Option<String>,
    pub started_at_unix: Option<i64>,
    pub completed_at_unix: Option<i64>,
    pub error_message: Option<String>,
}

impl Default for MigrationState {
    fn default() -> Self {
        Self {
            phase: MigrationPhase::Pending,
            soroban_deployment_name: None,
            started_at_unix: None,
            completed_at_unix: None,
            error_message: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Controller
// ---------------------------------------------------------------------------

/// Annotation that triggers the migration when set to `"soroban-rpc"`.
pub const MIGRATE_TO_ANNOTATION: &str = "stellar.org/migrate-to";

/// Controller that drives the Horizon→SorobanRpc zero-downtime migration.
pub struct HorizonToSorobanMigrationController {
    client: Client,
}

impl HorizonToSorobanMigrationController {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Return `true` when the `StellarNode` carries the migration annotation
    /// and is currently of type `Horizon`.
    pub fn migration_requested(node: &StellarNode) -> bool {
        node.spec.node_type == NodeType::Horizon
            && node
                .annotations()
                .get(MIGRATE_TO_ANNOTATION)
                .map(|v| v == "soroban-rpc")
                .unwrap_or(false)
    }

    /// Drive the migration state machine to completion.
    ///
    /// Returns the final [`MigrationState`] (either `Completed` or `Failed`).
    pub async fn run(&self, config: &MigrationConfig) -> Result<MigrationState, MigrationError> {
        let mut state = MigrationState {
            started_at_unix: Some(chrono::Utc::now().timestamp()),
            ..Default::default()
        };

        info!(
            node = %config.node_name,
            ns = %config.namespace,
            "Starting Horizon→SorobanRpc migration"
        );

        // Run the state machine, rolling back on any error when configured.
        let result = self.execute_phases(config, &mut state).await;
        match result {
            Ok(()) => {
                state.phase = MigrationPhase::Completed;
                state.completed_at_unix = Some(chrono::Utc::now().timestamp());
                info!(
                    node = %config.node_name,
                    "Horizon→SorobanRpc migration completed successfully"
                );
            }
            Err(e) => {
                let msg = e.to_string();
                warn!(node = %config.node_name, error = %msg, "Migration failed");
                state.phase = MigrationPhase::Failed;
                state.error_message = Some(msg.clone());

                if config.rollback_on_failure {
                    if let Err(rb_err) = self.rollback(config, &state).await {
                        warn!(error = %rb_err, "Rollback also failed — manual intervention required");
                    }
                }

                return Err(MigrationError::RolledBack(msg));
            }
        }

        Ok(state)
    }

    // -----------------------------------------------------------------------
    // Internal state-machine phases
    // -----------------------------------------------------------------------

    async fn execute_phases(
        &self,
        config: &MigrationConfig,
        state: &mut MigrationState,
    ) -> Result<(), MigrationError> {
        // Phase 1 — Validate preconditions
        self.validate_preconditions(config, state).await?;

        // Phase 2 — Provision parallel SorobanRpc Deployment
        state.phase = MigrationPhase::Provisioning;
        let soroban_name = self.provision_soroban_deployment(config, state).await?;
        state.soroban_deployment_name = Some(soroban_name.clone());

        // Phase 3 — Wait for SorobanRpc to become Ready
        state.phase = MigrationPhase::WaitingForSorobanReady;
        self.wait_for_soroban_ready(config, &soroban_name).await?;

        // Phase 4 — Shift traffic: update Service selector to SorobanRpc pods
        state.phase = MigrationPhase::TrafficShifting;
        self.shift_traffic(config, &soroban_name).await?;

        // Phase 5 — Drain legacy Horizon workload (scale to 0)
        state.phase = MigrationPhase::DrainHorizon;
        self.drain_horizon(config).await?;

        // Phase 6 — Remove Horizon Deployment and migration annotation
        state.phase = MigrationPhase::Cleanup;
        self.cleanup(config).await?;

        Ok(())
    }

    /// Verify the source node exists and is of type Horizon.
    async fn validate_preconditions(
        &self,
        config: &MigrationConfig,
        _state: &mut MigrationState,
    ) -> Result<(), MigrationError> {
        let nodes: Api<StellarNode> = Api::namespaced(self.client.clone(), &config.namespace);

        let node = nodes.get(&config.node_name).await?;

        if node.spec.node_type != NodeType::Horizon {
            return Err(MigrationError::Precondition(format!(
                "StellarNode '{}' is not of type Horizon (got: {})",
                config.node_name, node.spec.node_type
            )));
        }

        debug!(node = %config.node_name, "Preconditions satisfied");
        Ok(())
    }

    /// Create a parallel SorobanRpc `Deployment` derived from the Horizon spec.
    ///
    /// The new deployment is named `<node>-soroban-migration` and runs
    /// side-by-side with the existing Horizon deployment until traffic is cut over.
    async fn provision_soroban_deployment(
        &self,
        config: &MigrationConfig,
        _state: &mut MigrationState,
    ) -> Result<String, MigrationError> {
        let soroban_name = format!("{}-soroban-migration", config.node_name);
        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), &config.namespace);

        // Check if a migration deployment already exists (idempotent).
        if deployments.get(&soroban_name).await.is_ok() {
            info!(
                deployment = %soroban_name,
                "SorobanRpc migration deployment already exists — continuing"
            );
            return Ok(soroban_name);
        }

        // Fetch the current Horizon deployment to derive labels/resource settings.
        let horizon_deploy = deployments.get(&config.node_name).await?;

        // Build a minimal SorobanRpc deployment.
        // In production the image/args would be sourced from the StellarNode spec.
        let soroban_deploy: Deployment = serde_json::from_value(serde_json::json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": soroban_name,
                "namespace": config.namespace,
                "labels": {
                    "app": config.node_name,
                    "stellar.org/node-type": "SorobanRpc",
                    "stellar.org/migration": "true"
                },
                "ownerReferences": horizon_deploy.metadata.owner_references
            },
            "spec": {
                "replicas": horizon_deploy
                    .spec
                    .as_ref()
                    .and_then(|s| s.replicas)
                    .unwrap_or(1),
                "selector": {
                    "matchLabels": {
                        "app": config.node_name,
                        "stellar.org/node-type": "SorobanRpc"
                    }
                },
                "template": {
                    "metadata": {
                        "labels": {
                            "app": config.node_name,
                            "stellar.org/node-type": "SorobanRpc"
                        }
                    },
                    "spec": {
                        "containers": [{
                            "name": "soroban-rpc",
                            "image": "stellar/soroban-rpc:latest",
                            "ports": [{ "containerPort": 8000 }],
                            "readinessProbe": {
                                "httpGet": { "path": "/", "port": 8000 },
                                "initialDelaySeconds": 15,
                                "periodSeconds": 5,
                                "failureThreshold": 12
                            }
                        }]
                    }
                }
            }
        }))
        .map_err(|e| {
            MigrationError::Precondition(format!(
                "Failed to build SorobanRpc deployment manifest: {e}"
            ))
        })?;

        deployments
            .create(&Default::default(), &soroban_deploy)
            .await?;

        info!(deployment = %soroban_name, "SorobanRpc migration deployment created");
        Ok(soroban_name)
    }

    /// Poll until the SorobanRpc deployment reports all replicas as Available.
    async fn wait_for_soroban_ready(
        &self,
        config: &MigrationConfig,
        soroban_name: &str,
    ) -> Result<(), MigrationError> {
        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), &config.namespace);

        let deadline =
            std::time::Instant::now() + Duration::from_secs(config.readiness_timeout_secs);

        loop {
            let deploy = deployments.get(soroban_name).await?;
            let status = deploy.status.as_ref();

            let desired = deploy.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
            let available = status.and_then(|s| s.available_replicas).unwrap_or(0);

            if available >= desired {
                info!(
                    deployment = %soroban_name,
                    replicas = desired,
                    "SorobanRpc deployment is Ready"
                );
                return Ok(());
            }

            if std::time::Instant::now() > deadline {
                return Err(MigrationError::Timeout(config.readiness_timeout_secs));
            }

            debug!(
                deployment = %soroban_name,
                available,
                desired,
                "Waiting for SorobanRpc replicas…"
            );
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }

    /// Update the node's `Service` selector so traffic flows to SorobanRpc pods.
    async fn shift_traffic(
        &self,
        config: &MigrationConfig,
        _soroban_name: &str,
    ) -> Result<(), MigrationError> {
        let services: Api<Service> = Api::namespaced(self.client.clone(), &config.namespace);

        let patch = serde_json::json!({
            "spec": {
                "selector": {
                    "app": config.node_name,
                    "stellar.org/node-type": "SorobanRpc"
                }
            }
        });

        services
            .patch(
                &config.node_name,
                &PatchParams::default(),
                &Patch::Merge(&patch),
            )
            .await?;

        info!(
            service = %config.node_name,
            "Traffic shifted to SorobanRpc pods"
        );
        Ok(())
    }

    /// Scale the legacy Horizon `Deployment` to 0 replicas.
    async fn drain_horizon(&self, config: &MigrationConfig) -> Result<(), MigrationError> {
        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), &config.namespace);

        let patch = serde_json::json!({ "spec": { "replicas": 0 } });

        deployments
            .patch(
                &config.node_name,
                &PatchParams::default(),
                &Patch::Merge(&patch),
            )
            .await?;

        // Wait for pods to terminate before proceeding.
        let deadline = std::time::Instant::now() + Duration::from_secs(config.drain_timeout_secs);

        loop {
            let deploy = deployments.get(&config.node_name).await?;
            let ready = deploy
                .status
                .as_ref()
                .and_then(|s| s.ready_replicas)
                .unwrap_or(0);

            if ready == 0 {
                info!(
                    deployment = %config.node_name,
                    "Horizon deployment drained"
                );
                return Ok(());
            }

            if std::time::Instant::now() > deadline {
                warn!(
                    deployment = %config.node_name,
                    "Drain timed out — continuing anyway"
                );
                return Ok(());
            }

            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    /// Delete the Horizon workload and remove the migration annotation.
    async fn cleanup(&self, config: &MigrationConfig) -> Result<(), MigrationError> {
        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), &config.namespace);

        // Delete the Horizon deployment.
        deployments
            .delete(&config.node_name, &Default::default())
            .await?;

        info!(
            deployment = %config.node_name,
            "Legacy Horizon deployment deleted"
        );

        // Remove migration annotation from the StellarNode.
        let nodes: Api<StellarNode> = Api::namespaced(self.client.clone(), &config.namespace);

        let annotation_patch = serde_json::json!({
            "metadata": {
                "annotations": {
                    MIGRATE_TO_ANNOTATION: serde_json::Value::Null
                }
            }
        });

        nodes
            .patch(
                &config.node_name,
                &PatchParams::default(),
                &Patch::Merge(&annotation_patch),
            )
            .await?;

        info!(node = %config.node_name, "Migration annotation removed");
        Ok(())
    }

    /// Restore Horizon replica count if migration fails (best-effort).
    async fn rollback(
        &self,
        config: &MigrationConfig,
        state: &MigrationState,
    ) -> Result<(), MigrationError> {
        warn!(
            node = %config.node_name,
            phase = %state.phase,
            "Rolling back migration — restoring Horizon deployment"
        );

        let deployments: Api<Deployment> = Api::namespaced(self.client.clone(), &config.namespace);

        // Restore Horizon replicas.
        let patch = serde_json::json!({ "spec": { "replicas": 1 } });
        deployments
            .patch(
                &config.node_name,
                &PatchParams::default(),
                &Patch::Merge(&patch),
            )
            .await?;

        // Revert service selector back to Horizon pods.
        let services: Api<Service> = Api::namespaced(self.client.clone(), &config.namespace);
        let svc_patch = serde_json::json!({
            "spec": {
                "selector": {
                    "app": config.node_name,
                    "stellar.org/node-type": "Horizon"
                }
            }
        });
        services
            .patch(
                &config.node_name,
                &PatchParams::default(),
                &Patch::Merge(&svc_patch),
            )
            .await?;

        // Attempt to delete the in-progress SorobanRpc deployment if it exists.
        if let Some(soroban_name) = &state.soroban_deployment_name {
            let _ = deployments.delete(soroban_name, &Default::default()).await;
        }

        info!(node = %config.node_name, "Rollback complete — Horizon restored");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_phase_display() {
        assert_eq!(MigrationPhase::Pending.to_string(), "Pending");
        assert_eq!(MigrationPhase::Provisioning.to_string(), "Provisioning");
        assert_eq!(
            MigrationPhase::WaitingForSorobanReady.to_string(),
            "WaitingForSorobanReady"
        );
        assert_eq!(
            MigrationPhase::TrafficShifting.to_string(),
            "TrafficShifting"
        );
        assert_eq!(MigrationPhase::DrainHorizon.to_string(), "DrainHorizon");
        assert_eq!(MigrationPhase::Cleanup.to_string(), "Cleanup");
        assert_eq!(MigrationPhase::Completed.to_string(), "Completed");
        assert_eq!(MigrationPhase::Failed.to_string(), "Failed");
    }

    #[test]
    fn migration_state_default_is_pending() {
        let state = MigrationState::default();
        assert_eq!(state.phase, MigrationPhase::Pending);
        assert!(state.soroban_deployment_name.is_none());
        assert!(state.error_message.is_none());
    }

    #[test]
    fn migration_config_defaults() {
        let cfg = MigrationConfig::new("stellar", "my-node");
        assert_eq!(cfg.namespace, "stellar");
        assert_eq!(cfg.node_name, "my-node");
        assert_eq!(cfg.readiness_timeout_secs, 600);
        assert!(cfg.rollback_on_failure);
    }
}
