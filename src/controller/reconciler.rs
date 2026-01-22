//! Main reconciler for StellarNode resources
//!
//! Implements the controller pattern using kube-rs runtime.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{Event, PersistentVolumeClaim, Service};
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
use tracing::{debug, error, info, instrument, warn};

use crate::controller::archive_health::{
    calculate_backoff, check_history_archive_health, ArchiveHealthResult,
};
use crate::crd::{Condition, NodeType, StellarNode, StellarNodeStatus};
use crate::error::{Error, Result};

const ARCHIVE_RETRIES_ANNOTATION: &str = "stellar.org/archive-retries";

use super::finalizers::STELLAR_NODE_FINALIZER;
use super::health;
use super::mtls;
use super::resources;

/// Shared state for the controller
pub struct ControllerState {
    pub client: Client,
    pub enable_mtls: bool,
    pub operator_namespace: String,
    pub mtls_config: Option<crate::MtlsConfig>,
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
            error!(
                "StellarNode CRD not found. Please install the CRD first: {:?}",
                e
            );
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
            FinalizerEvent::Apply(node) => apply_stellar_node(&client, &node, &ctx).await,
            FinalizerEvent::Cleanup(node) => cleanup_stellar_node(&client, &node).await,
        }
    })
    .await
    .map_err(Error::from)
}

/// Apply/create/update the StellarNode resources
#[instrument(skip(client, node, ctx), fields(name = %node.name_any(), namespace = node.namespace()))]
async fn apply_stellar_node(
    client: &Client,
    node: &StellarNode,
    ctx: &ControllerState,
) -> Result<Action> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!("Applying StellarNode: {}/{}", namespace, name);

    // Validate the spec
    if let Err(e) = node.spec.validate() {
        warn!("Validation failed for {}/{}: {}", namespace, name, e);
        update_status(client, node, "Failed", Some(&e), 0, true).await?;
        return Err(Error::ValidationError(e));
    }

    // 1. Core infrastructure (PVC and ConfigMap) always managed by operator
    resources::ensure_pvc(client, node).await?;
    resources::ensure_config_map(client, node, ctx.enable_mtls).await?;

    // 2. Maintenance Mode Check
    // If active, we skip workload management (Step 3) and suspension checks.
    // This allows a human to manually scale the node up or down as needed.
    if node.spec.maintenance_mode {
        info!(
            "Node {}/{} in Maintenance Mode. Skipping workload updates.",
            namespace, name
        );

        resources::ensure_service(client, node, ctx.enable_mtls).await?;

        update_status(
            client,
            node,
            "Maintenance",
            Some("Manual maintenance mode active; workload management paused"),
            0,
            true,
        )
        .await?;

        return Ok(Action::requeue(Duration::from_secs(60)));
    }

    // 3. Normal Mode: Handle suspension
    // This only runs if NOT in maintenance mode.
    if node.spec.suspended {
        info!("Node {}/{} is suspended, scaling to 0", namespace, name);
        update_status(
            client,
            node,
            "Suspended",
            Some("Node is suspended"),
            0,
            true,
        )
        .await?;
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
                    info!(
                        "Running history archive health check for {}/{}",
                        namespace, name
                    );

                    let health_result =
                        check_history_archive_health(&validator_config.history_archive_urls, None)
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
                            namespace,
                            name,
                            delay,
                            retries + 1
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
    update_status(
        client,
        node,
        "Creating",
        Some("Creating resources"),
        0,
        true,
    )
    .await?;

    // 1. Create/update the PersistentVolumeClaim
    resources::ensure_pvc(client, node).await?;
    info!("PVC ensured for {}/{}", namespace, name);

    // 2. Create/update the ConfigMap for node configuration
    resources::ensure_config_map(client, node, ctx.enable_mtls).await?;
    info!("ConfigMap ensured for {}/{}", namespace, name);

    // 2.5 Ensure mTLS certificates (if strict mTLS will be handled at rest_api level)
    // We always ensure them for now if the operator supports it
    mtls::ensure_ca(client, &namespace).await?;
    mtls::ensure_node_cert(client, node).await?;
    info!("mTLS certificates ensured for {}/{}", namespace, name);

    // 3. Create/update the Deployment/StatefulSet based on node type
    match node.spec.node_type {
        NodeType::Validator => {
            resources::ensure_statefulset(client, node, ctx.enable_mtls).await?;
        }
        NodeType::Horizon | NodeType::SorobanRpc => {
            resources::ensure_deployment(client, node, ctx.enable_mtls).await?;
        }
    }

    // 5. Ensure Service and finalize status
    resources::ensure_service(client, node, ctx.enable_mtls).await?;

    // 5. Perform health check to determine if node is ready
    let health_result = health::check_node_health(client, node, ctx.mtls_config.as_ref()).await?;

    debug!(
        "Health check result for {}/{}: healthy={}, synced={}, message={}",
        namespace, name, health_result.healthy, health_result.synced, health_result.message
    );

    // Determine the phase based on health check
    let (phase, message) = if node.spec.suspended {
        ("Suspended", "Node is suspended".to_string())
    } else if !health_result.healthy {
        ("Creating", health_result.message.clone())
    } else if !health_result.synced {
        ("Syncing", health_result.message.clone())
    } else {
        ("Ready", "Node is healthy and synced".to_string())
    };

    // 6. Update status with health check results
    update_status_with_health(client, node, phase, Some(&message), &health_result).await?;

    info!(
        "Node {}/{} status updated to: {} - {}",
        namespace, name, phase, message
    );
    // 5. Create/update Ingress if configured
    resources::ensure_ingress(client, node).await?;
    info!("Ingress ensured for {}/{}", namespace, name);

    // 6. Create/update ServiceMonitor for Prometheus scraping (if autoscaling enabled)
    if node.spec.autoscaling.is_some() {
        resources::ensure_service_monitor(client, node).await?;
        info!("ServiceMonitor ensured for {}/{}", namespace, name);

        // 7. Create/update HPA for autoscaling
        resources::ensure_hpa(client, node).await?;
        info!("HPA ensured for {}/{}", namespace, name);
    }

    // 7. Create/update alerting rules
    resources::ensure_alerting(client, node).await?;
    info!("Alerting ensured for {}/{}", namespace, name);
    // 8. Fetch the ready replicas from Deployment/StatefulSet status
    let ready_replicas = get_ready_replicas(client, node).await.unwrap_or(0);

    // 9. Update status to Running with ready replica count
    let phase = if node.spec.suspended {
        "Suspended"
    } else {
        "Running"
    };
    update_status(
        client,
        node,
        phase,
        Some("Resources created successfully"),
        ready_replicas,
        true,
    )
    .await?;

    // Requeue based on current state
    let requeue_duration = if phase == "Ready" {
        // Check less frequently when ready
        Duration::from_secs(60)
    } else {
        // Check more frequently when syncing
        Duration::from_secs(15)
    };

    Ok(Action::requeue(requeue_duration))
}

/// Clean up resources when the StellarNode is deleted
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
async fn cleanup_stellar_node(client: &Client, node: &StellarNode) -> Result<Action> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!("Cleaning up StellarNode: {}/{}", namespace, name);

    // Delete resources in reverse order of creation

    // 0. Delete Alerting
    if let Err(e) = resources::delete_alerting(client, node).await {
        warn!("Failed to delete alerting: {:?}", e);
    }

    // 1. Delete HPA (if autoscaling was configured)
    if let Err(e) = resources::delete_hpa(client, node).await {
        warn!("Failed to delete HPA: {:?}", e);
    }

    // 2. Delete ServiceMonitor (if autoscaling was configured)
    if let Err(e) = resources::delete_service_monitor(client, node).await {
        warn!("Failed to delete ServiceMonitor: {:?}", e);
    }

    // 3. Delete Ingress
    if let Err(e) = resources::delete_ingress(client, node).await {
        warn!("Failed to delete Ingress: {:?}", e);
    }

    // 4. Delete Service
    if let Err(e) = resources::delete_service(client, node).await {
        warn!("Failed to delete Service: {:?}", e);
    }

    // 5. Delete Deployment/StatefulSet
    if let Err(e) = resources::delete_workload(client, node).await {
        warn!("Failed to delete workload: {:?}", e);
    }

    // 6. Delete ConfigMap
    if let Err(e) = resources::delete_config_map(client, node).await {
        warn!("Failed to delete ConfigMap: {:?}", e);
    }

    // 7. Delete PVC based on retention policy
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

/// Fetch the ready replicas from the Deployment or StatefulSet status
async fn get_ready_replicas(client: &Client, node: &StellarNode) -> Result<i32> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    match node.spec.node_type {
        NodeType::Validator => {
            // Validators use StatefulSet
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
            match api.get(&name).await {
                Ok(statefulset) => {
                    let ready_replicas = statefulset
                        .status
                        .as_ref()
                        .and_then(|s| s.ready_replicas)
                        .unwrap_or(0);
                    Ok(ready_replicas)
                }
                Err(e) => {
                    warn!("Failed to get StatefulSet {}/{}: {:?}", namespace, name, e);
                    Ok(0)
                }
            }
        }
        NodeType::Horizon | NodeType::SorobanRpc => {
            // RPC nodes use Deployment
            let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
            match api.get(&name).await {
                Ok(deployment) => {
                    let ready_replicas = deployment
                        .status
                        .as_ref()
                        .and_then(|s| s.ready_replicas)
                        .unwrap_or(0);
                    Ok(ready_replicas)
                }
                Err(e) => {
                    warn!("Failed to get Deployment {}/{}: {:?}", namespace, name, e);
                    Ok(0)
                }
            }
        }
    }
}

/// Update the status subresource of a StellarNode
async fn update_status(
    client: &Client,
    node: &StellarNode,
    phase: &str,
    message: Option<&str>,
    ready_replicas: i32,
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

    let mut status_patch = serde_json::json!({
        "phase": phase,
        "observedGeneration": observed_generation,
        "replicas": if node.spec.suspended { 0 } else { node.spec.replicas },
        "readyReplicas": ready_replicas,
    });

    if let Some(msg) = message {
        status_patch["message"] = serde_json::Value::String(msg.to_string());
    }

    let patch = serde_json::json!({ "status": status_patch });
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
        status: if result.any_healthy { "True" } else { "False" }.to_string(),
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

/// Update the status subresource with health check results
async fn update_status_with_health(
    client: &Client,
    node: &StellarNode,
    phase: &str,
    message: Option<&str>,
    health: &health::HealthCheckResult,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    // Build conditions based on health check
    let mut conditions = Vec::new();

    // Ready condition
    let ready_condition = if health.synced {
        crate::crd::Condition::ready(true, "NodeSynced", "Node is fully synced and operational")
    } else if health.healthy {
        crate::crd::Condition::ready(false, "NodeSyncing", &health.message)
    } else {
        crate::crd::Condition::ready(false, "NodeNotHealthy", &health.message)
    };
    conditions.push(ready_condition);

    // Progressing condition
    if !health.synced && health.healthy {
        conditions.push(crate::crd::Condition::progressing(
            "Syncing",
            &health.message,
        ));
    }

    let status = StellarNodeStatus {
        phase: phase.to_string(),
        message: message.map(String::from),
        observed_generation: node.metadata.generation,
        replicas: if node.spec.suspended {
            0
        } else {
            node.spec.replicas
        },
        ready_replicas: if health.synced && !node.spec.suspended {
            node.spec.replicas
        } else {
            0
        },
        ledger_sequence: health.ledger_sequence,
        conditions,
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

/// Error policy determines how to handle reconciliation errors
fn error_policy(node: Arc<StellarNode>, error: &Error, _ctx: Arc<ControllerState>) -> Action {
    error!("Reconciliation error for {}: {:?}", node.name_any(), error);

    // Use shorter retry for retriable errors
    let retry_duration = if error.is_retriable() {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(60)
    };

    Action::requeue(retry_duration)
}
