//! Automated Horizon to Soroban-RPC migration controller
//!
//! Handles zero-downtime migration from Horizon nodes to Soroban RPC nodes
//! by running both in parallel during transition.

use kube::{api::Patch, api::PatchParams, Api, Client, ResourceExt};
use tracing::info;

use crate::crd::{MigrationPhase, MigrationStatus, NodeType, SorobanConfig, StellarNode};
use crate::error::Result;

use super::conditions;

const MIGRATION_ANNOTATION: &str = "stellar.org/migration-in-progress";
pub const MIGRATION_SOURCE_TYPE: &str = "stellar.org/migration-source-type";

/// Reconcile migration from Horizon to Soroban RPC
///
/// Returns true if migration is in progress and requires requeue
pub async fn reconcile_migration(client: &Client, node: &StellarNode) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    // Check if this is a migration scenario
    let is_migrating = node
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(MIGRATION_ANNOTATION))
        .map(|v| v == "true")
        .unwrap_or(false);

    let source_type = node
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(MIGRATION_SOURCE_TYPE));

    // Scenario 1: User changed nodeType from Horizon to SorobanRpc
    if node.spec.node_type == NodeType::SorobanRpc
        && !is_migrating
        && node.spec.soroban_config.is_some()
    {
        // Check if this was previously a Horizon node by looking at status or history
        let was_horizon = node
            .status
            .as_ref()
            .and_then(|s| s.message.as_ref())
            .map(|m| m.contains("Horizon"))
            .unwrap_or(false);

        if was_horizon || source_type == Some(&"Horizon".to_string()) {
            info!(
                "Detected Horizon to Soroban RPC migration for {}/{}",
                namespace, name
            );
            start_migration(client, node).await?;
            return Ok(true);
        }
    }

    // Scenario 2: Migration in progress - monitor and complete
    if is_migrating && node.spec.node_type == NodeType::SorobanRpc {
        let migration_complete = check_migration_complete(client, node).await?;

        if migration_complete {
            info!("Migration complete for {}/{}", namespace, name);
            complete_migration(client, node).await?;
            return Ok(false);
        }

        info!("Migration in progress for {}/{}", namespace, name);
        return Ok(true);
    }

    Ok(false)
}

/// Start the migration process
async fn start_migration(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    // Mark migration as in progress
    let mut annotations = node.metadata.annotations.clone().unwrap_or_default();
    annotations.insert(MIGRATION_ANNOTATION.to_string(), "true".to_string());
    annotations.insert(MIGRATION_SOURCE_TYPE.to_string(), "Horizon".to_string());

    let migration_status = MigrationStatus {
        from_type: "Horizon".to_string(),
        to_type: "SorobanRpc".to_string(),
        phase: MigrationPhase::Starting,
        start_time: chrono::Utc::now().to_rfc3339(),
        completion_time: None,
        message: "Initiating migration from Horizon to Soroban RPC".to_string(),
    };

    let patch = serde_json::json!({
        "metadata": {
            "annotations": annotations
        }
    });

    api.patch(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await?;

    // Update status
    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    conditions::set_condition(
        &mut conditions,
        "Migrating",
        conditions::CONDITION_STATUS_TRUE,
        "HorizonToSorobanRpc",
        "Migration from Horizon to Soroban RPC in progress",
    );

    let status_patch = serde_json::json!({
        "status": {
            "conditions": conditions,
            "message": "Migrating from Horizon to Soroban RPC",
            "migrationStatus": migration_status
        }
    });

    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&status_patch),
    )
    .await?;

    info!("Migration started for {}/{}", namespace, node.name_any());
    Ok(())
}

/// Check if migration is complete
async fn check_migration_complete(client: &Client, node: &StellarNode) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    // Check if Soroban RPC deployment is ready
    let deploy_api: Api<k8s_openapi::api::apps::v1::Deployment> =
        Api::namespaced(client.clone(), &namespace);

    match deploy_api.get(&node.name_any()).await {
        Ok(deployment) => {
            let ready = deployment
                .status
                .as_ref()
                .and_then(|s| s.ready_replicas)
                .unwrap_or(0);
            let desired = deployment
                .spec
                .as_ref()
                .and_then(|s| s.replicas)
                .unwrap_or(0);

            Ok(ready >= desired && ready > 0)
        }
        Err(_) => Ok(false),
    }
}

/// Complete the migration and cleanup
async fn complete_migration(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    // Remove migration annotations
    let mut annotations = node.metadata.annotations.clone().unwrap_or_default();
    annotations.remove(MIGRATION_ANNOTATION);
    annotations.remove(MIGRATION_SOURCE_TYPE);

    let patch = serde_json::json!({
        "metadata": {
            "annotations": annotations
        }
    });

    api.patch(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await?;

    // Update status
    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    conditions::remove_condition(&mut conditions, "Migrating");
    conditions::set_condition(
        &mut conditions,
        conditions::CONDITION_TYPE_READY,
        conditions::CONDITION_STATUS_TRUE,
        "MigrationComplete",
        "Successfully migrated from Horizon to Soroban RPC",
    );

    let migration_status = MigrationStatus {
        from_type: "Horizon".to_string(),
        to_type: "SorobanRpc".to_string(),
        phase: MigrationPhase::Complete,
        start_time: node
            .status
            .as_ref()
            .and_then(|s| s.migration_status.as_ref())
            .map(|m| m.start_time.clone())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        completion_time: Some(chrono::Utc::now().to_rfc3339()),
        message: "Migration completed successfully".to_string(),
    };

    let status_patch = serde_json::json!({
        "status": {
            "conditions": conditions,
            "message": "Migration complete",
            "migrationStatus": migration_status
        }
    });

    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&status_patch),
    )
    .await?;

    info!("Migration completed for {}/{}", namespace, node.name_any());
    Ok(())
}

/// Migrate Horizon config to Soroban config
#[allow(deprecated)]
pub fn migrate_config(horizon_config: &crate::crd::HorizonConfig) -> SorobanConfig {
    SorobanConfig {
        stellar_core_url: horizon_config.stellar_core_url.clone(),
        captive_core_config: None,
        captive_core_structured_config: None,
        enable_preflight: true,
        max_events_per_request: 10000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_config() {
        let horizon = crate::crd::HorizonConfig {
            database_secret_ref: "test".to_string(),
            enable_ingest: true,
            stellar_core_url: "http://core:11626".to_string(),
            ingest_workers: 2,
            enable_experimental_ingestion: false,
            auto_migration: true,
        };

        let soroban = migrate_config(&horizon);
        assert_eq!(soroban.stellar_core_url, "http://core:11626");
        assert!(soroban.enable_preflight);
    }
}
