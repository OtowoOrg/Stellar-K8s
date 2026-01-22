//! Main reconciler for StellarNode resources
//!
//! Implements the controller pattern using kube-rs runtime.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Service, Event};
use kube::{
    api::{Api, Patch, PatchParams, PostParams},
    client::Client,
    runtime::{
        controller::{Action, Controller},
        finalizer::{finalizer, Event as FinalizerEvent},
        watcher::Config,
    },
    Resource, ResourceExt,
};
use tracing::{error, info, instrument, warn};

use crate::crd::{NodeType, StellarNode, StellarNodeStatus, Condition};
use crate::error::{Error, Result};

use super::finalizers::STELLAR_NODE_FINALIZER;
use super::{resources, check_history_archive_health, calculate_backoff, ArchiveHealthResult};

/// Annotation key for tracking history archive health check retries
const ARCHIVE_RETRIES_ANNOTATION: &str = "stellar.org/archive-retries";

/// Shared state for the controller
pub struct ControllerState {
    pub client: Client,
}

/// Main entry point to start the controller
pub async fn run_controller(state: Arc<ControllerState>) -> Result<()> {
    let client = state.client.clone();
    let stellar_nodes: Api<StellarNode> = Api::all(client.clone());

    info!("Starting StellarNode controller");

    // Verify CRD exists
    match stellar_nodes.list(&Default::default()).await {
        Ok(_) => info!("StellarNode CRD is available"),
        Err(e) => {
            error!("StellarNode CRD not found. Please install the CRD first: {:?}", e);
            return Err(Error::ConfigError(
                "StellarNode CRD not installed".to_string(),
            ));
        }
    }

    Controller::new(stellar_nodes, Config::default())
        // Watch owned resources for changes
        .owns::<Deployment>(Api::all(client.clone()), Config::default())
        .owns::<StatefulSet>(Api::all(client.clone()), Config::default())
        .owns::<Service>(Api::all(client.clone()), Config::default())
        .owns::<PersistentVolumeClaim>(Api::all(client.clone()), Config::default())
        .shutdown_on_signal()
        .run(reconcile, error_policy, state)
        .for_each(|res| async move {
            match res {
                Ok(obj) => info!("Reconciled: {:?}", obj),
                Err(e) => error!("Reconcile error: {:?}", e),
            }
        })
        .await;

    Ok(())
}

/// Helper to emit a Kubernetes Event
async fn emit_event(
    client: &Client,
    node: &StellarNode,
    event_type: &str,
    reason: &str,
    message: &str,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let events: Api<Event> = Api::namespaced(client.clone(), &namespace);

    let time = chrono::Utc::now();
    let event = Event {
        metadata: kube::api::ObjectMeta {
            generate_name: Some(format!("{}-event-", node.name_any())),
            ..Default::default()
        },
        type_: Some(event_type.to_string()),
        reason: Some(reason.to_string()),
        message: Some(message.to_string()),
        involved_object: node.object_ref(&()),
        first_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(time)),
        last_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(time)),
        count: Some(1),
        ..Default::default()
    };

    events
        .create(&PostParams::default(), &event)
        .await
        .map_err(Error::KubeError)?;
    Ok(())
}

/// Get the current retry count for archive health checks from annotations
fn get_retry_count(node: &StellarNode) -> u32 {
    node.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(ARCHIVE_RETRIES_ANNOTATION))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Set the retry count for archive health checks in annotations
async fn set_retry_count(client: &Client, node: &StellarNode, count: u32) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let mut annotations = node.metadata.annotations.clone().unwrap_or_default();
    if count == 0 {
        annotations.remove(ARCHIVE_RETRIES_ANNOTATION);
    } else {
        annotations.insert(ARCHIVE_RETRIES_ANNOTATION.to_string(), count.to_string());
    }

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
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

/// The main reconciliation function
///
/// This function is called whenever:
/// - A StellarNode is created, updated, or deleted
/// - An owned resource (Deployment, Service, PVC) changes
/// - The requeue timer expires
#[instrument(skip(ctx), fields(name = %obj.name_any(), namespace = obj.namespace()))]
async fn reconcile(obj: Arc<StellarNode>, ctx: Arc<ControllerState>) -> Result<Action> {
    let client = ctx.client.clone();
    let namespace = obj.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    info!(
        "Reconciling StellarNode {}/{} (type: {:?})",
        namespace,
        obj.name_any(),
        obj.spec.node_type
    );

    // Use kube-rs built-in finalizer helper for clean lifecycle management
    finalizer(&api, STELLAR_NODE_FINALIZER, obj, |event| async {
        match event {
            FinalizerEvent::Apply(node) => apply_stellar_node(&client, &node).await,
            FinalizerEvent::Cleanup(node) => cleanup_stellar_node(&client, &node).await,
        }
    })
    .await
    .map_err(Error::from)
}

/// Apply/create/update the StellarNode resources
async fn apply_stellar_node(client: &Client, node: &StellarNode) -> Result<Action> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!("Applying StellarNode: {}/{}", namespace, name);

    // Validate the spec
    if let Err(e) = node.spec.validate() {
        warn!("Validation failed for {}/{}: {}", namespace, name, e);
        update_status(client, node, "Failed", Some(&e), true).await?;
        return Err(Error::ValidationError(e));
    }

    // Check if suspended
    if node.spec.suspended {
        info!("Node {}/{} is suspended, scaling to 0", namespace, name);
        update_status(client, node, "Suspended", Some("Node is suspended"), true).await?;
        // Still create resources but with 0 replicas
    }

    // History Archive Health Check for Validators
    if node.spec.node_type == NodeType::Validator {
        if let Some(validator_config) = &node.spec.validator_config {
            if validator_config.enable_history_archive
                && !validator_config.history_archive_urls.is_empty()
            {
                let is_startup_or_update = node
                    .status
                    .as_ref()
                    .and_then(|s| s.observed_generation)
                    .map(|og| og < node.metadata.generation.unwrap_or(0))
                    .unwrap_or(true);

                if is_startup_or_update {
                    info!("Running history archive health check for {}/{}", namespace, name);

                    let health_result = check_history_archive_health(
                        &validator_config.history_archive_urls,
                        None,
                    )
                    .await?;

                    if !health_result.any_healthy {
                        warn!(
                            "Archive health check failed for {}/{}: {}",
                            namespace,
                            name,
                            health_result.summary()
                        );

                        // Emit Kubernetes Event
                        emit_event(
                            client,
                            node,
                            "Warning",
                            "ArchiveHealthCheckFailed",
                            &format!(
                                "None of the configured archives are reachable:\n{}",
                                health_result.error_details()
                            ),
                        )
                        .await?;

                        // Update status with condition (observed_generation NOT updated to trigger retry)
                        update_archive_health_status(client, node, &health_result).await?;

                        // Requeue with exponential backoff
                        let retries = get_retry_count(node);
                        let delay = calculate_backoff(retries, None, None);

                        info!(
                            "Requeuing {}/{} in {:?} (retry attempt {})",
                            namespace, name, delay, retries + 1
                        );

                        // Increment retry count in annotations
                        set_retry_count(client, node, retries + 1).await?;

                        return Ok(Action::requeue(delay));
                    } else {
                        info!(
                            "Archive health check passed for {}/{}: {}",
                            namespace,
                            name,
                            health_result.summary()
                        );
                        // Update status with archive health condition
                        update_archive_health_status(client, node, &health_result).await?;

                        // Reset retry count on success
                        if get_retry_count(node) > 0 {
                            set_retry_count(client, node, 0).await?;
                        }
                    }
                }
            }
        }
    }

    // Update status to Creating
    update_status(client, node, "Creating", Some("Creating resources"), true).await?;

    // 1. Create/update the PersistentVolumeClaim
    resources::ensure_pvc(client, node).await?;
    info!("PVC ensured for {}/{}", namespace, name);

    // 2. Create/update the ConfigMap for node configuration
    resources::ensure_config_map(client, node).await?;
    info!("ConfigMap ensured for {}/{}", namespace, name);

    // 3. Create/update the Deployment/StatefulSet based on node type
    match node.spec.node_type {
        NodeType::Validator => {
            // Validators use StatefulSet for stable identity
            resources::ensure_statefulset(client, node).await?;
            info!("StatefulSet ensured for validator {}/{}", namespace, name);
        }
        NodeType::Horizon | NodeType::SorobanRpc => {
            // RPC nodes use Deployment for easy scaling
            resources::ensure_deployment(client, node).await?;
            info!("Deployment ensured for RPC node {}/{}", namespace, name);
        }
    }

    // 4. Create/update the Service
    resources::ensure_service(client, node).await?;
    info!("Service ensured for {}/{}", namespace, name);

    // 5. Update status to Running
    let phase = if node.spec.suspended { "Suspended" } else { "Running" };
    update_status(client, node, phase, Some("Resources created successfully"), true).await?;

    // Requeue after 30 seconds to check node health and sync status
    Ok(Action::requeue(Duration::from_secs(30)))
}

/// Clean up resources when the StellarNode is deleted
async fn cleanup_stellar_node(client: &Client, node: &StellarNode) -> Result<Action> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!("Cleaning up StellarNode: {}/{}", namespace, name);

    // Delete resources in reverse order of creation

    // 1. Delete Service
    if let Err(e) = resources::delete_service(client, node).await {
        warn!("Failed to delete Service: {:?}", e);
    }

    // 2. Delete Deployment/StatefulSet
    if let Err(e) = resources::delete_workload(client, node).await {
        warn!("Failed to delete workload: {:?}", e);
    }

    // 3. Delete ConfigMap
    if let Err(e) = resources::delete_config_map(client, node).await {
        warn!("Failed to delete ConfigMap: {:?}", e);
    }

    // 4. Delete PVC based on retention policy
    if node.spec.should_delete_pvc() {
        info!(
            "Deleting PVC for node: {}/{} (retention policy: Delete)",
            namespace, name
        );
        if let Err(e) = resources::delete_pvc(client, node).await {
            warn!("Failed to delete PVC: {:?}", e);
        }
    } else {
        info!(
            "Retaining PVC for node: {}/{} (retention policy: Retain)",
            namespace, name
        );
    }

    info!("Cleanup complete for StellarNode: {}/{}", namespace, name);

    // Return await_change to signal finalizer completion
    Ok(Action::await_change())
}

/// Update the status subresource of a StellarNode
async fn update_status(
    client: &Client,
    node: &StellarNode,
    phase: &str,
    message: Option<&str>,
    update_obs_gen: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let observed_generation = if update_obs_gen {
        node.metadata.generation
    } else {
        node.status
            .as_ref()
            .and_then(|status| status.observed_generation)
    };

    let status = StellarNodeStatus {
        phase: phase.to_string(),
        message: message.map(String::from),
        observed_generation,
        replicas: if node.spec.suspended { 0 } else { node.spec.replicas },
        ..Default::default()
    };

    let patch = serde_json::json!({ "status": status });
    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

/// Update the status with archive health check results
async fn update_archive_health_status(
    client: &Client,
    node: &StellarNode,
    result: &ArchiveHealthResult,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let condition = Condition {
        type_: "ArchiveHealthCheck".to_string(),
        status: if result.any_healthy {
            "True"
        } else {
            "False"
        }
        .to_string(),
        last_transition_time: chrono::Utc::now().to_rfc3339(),
        reason: if result.any_healthy {
            "ArchiveHealthy"
        } else {
            "ArchiveUnreachable"
        }
        .to_string(),
        message: if result.any_healthy {
            result.summary()
        } else {
            format!("{}\n{}", result.summary(), result.error_details())
        },
    };

    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    // Update or append the condition
    if let Some(pos) = conditions
        .iter()
        .position(|c| c.type_ == "ArchiveHealthCheck")
    {
        conditions[pos] = condition;
    } else {
        conditions.push(condition);
    }

    let patch = serde_json::json!({
        "status": {
            "conditions": conditions,
            "phase": if result.any_healthy { "Creating" } else { "WaitingForArchive" },
            "message": result.summary()
        }
    });

    api.patch_status(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

/// Error policy determines how to handle reconciliation errors
fn error_policy(node: Arc<StellarNode>, error: &Error, _ctx: Arc<ControllerState>) -> Action {
    error!(
        "Reconciliation error for {}: {:?}",
        node.name_any(),
        error
    );

    // Use shorter retry for retriable errors
    let retry_duration = if error.is_retriable() {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(60)
    };

    Action::requeue(retry_duration)
}
