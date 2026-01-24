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

use crate::crd::{NodeType, StellarNode, StellarNodeStatus};
use crate::error::{Error, Result};

use super::archive_health::{calculate_backoff, check_history_archive_health, ArchiveHealthResult};
use super::conditions;
use super::finalizers::STELLAR_NODE_FINALIZER;
use super::health;
use super::metrics;
use super::mtls;
use super::remediation;
use super::resources;
use super::vsl;

// Constants
const ARCHIVE_RETRIES_ANNOTATION: &str = "stellar.org/archive-health-retries";

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

    let res = finalizer(&api, STELLAR_NODE_FINALIZER, obj.clone(), |event| async {
        match event {
            FinalizerEvent::Apply(node) => apply_stellar_node(&client, &node, &ctx).await,
            FinalizerEvent::Cleanup(node) => cleanup_stellar_node(&client, &node).await,
        }
    })
    .await;

    if res.is_ok() {
        // Track total reconciliations with differential privacy
        // We use a mock "cumulative" value for this example
        metrics::inc_reconciliation_count(
            &namespace,
            &obj.name_any(),
            &obj.spec.node_type.to_string(),
            obj.spec.network.passphrase(),
            1, // In a real app, this would be a persistent counter
        );
    }

    res.map_err(Error::from)
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
        update_status(client, node, "Failed", Some(e.as_str()), 0, true).await?;
        return Err(Error::ValidationError(e));
    }

    // 1. Core infrastructure (PVC and ConfigMap) always managed by operator
    resources::ensure_pvc(client, node).await?;
    resources::ensure_config_map(client, node, None, ctx.enable_mtls).await?;
    // 2. Handle suspension
    if node.spec.suspended {
        info!("Node {}/{} is suspended, scaling to 0", namespace, name);

        resources::ensure_pvc(client, node).await?;
        resources::ensure_config_map(client, node, None, ctx.enable_mtls).await?;

        match node.spec.node_type {
            NodeType::Validator => {
                resources::ensure_statefulset(client, node, ctx.enable_mtls).await?;
            }
            NodeType::Horizon | NodeType::SorobanRpc => {
                resources::ensure_deployment(client, node, ctx.enable_mtls).await?;
            }
        }

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
        update_suspended_status(client, node).await?;

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

                        // Update status with archive health condition (observed_generation NOT updated to trigger retry)
                        update_archive_health_status(client, node, &health_result).await?;

                        let delay = calculate_backoff(0, None, None);
                        info!(
                            "Archive health check failed for {}/{}, requeuing in {:?}",
                            namespace, name, delay
                        );

                        return Ok(Action::requeue(delay));
                    } else {
                        info!(
                            "Archive health check passed for {}/{}: {}",
                            namespace,
                            name,
                            health_result.summary()
                        );
                        update_archive_health_status(client, node, &health_result).await?;
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

    // 2. Handle VSL Fetching for Validators
    let mut quorum_override = None;
    if node.spec.node_type == NodeType::Validator {
        if let Some(config) = &node.spec.validator_config {
            if let Some(vl_source) = &config.vl_source {
                match vsl::fetch_vsl(vl_source).await {
                    Ok(quorum) => {
                        quorum_override = Some(quorum);
                    }
                    Err(e) => {
                        warn!("Failed to fetch VSL for {}/{}: {}", namespace, name, e);
                        emit_event(
                            client,
                            node,
                            "Warning",
                            "VSLFetchFailed",
                            &format!("Failed to fetch VSL from {}: {}", vl_source, e),
                        )
                        .await?;
                    }
                }
            }
        }
    }

    // 3. Create/update the ConfigMap for node configuration
    resources::ensure_config_map(client, node, quorum_override.clone(), ctx.enable_mtls).await?;
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
    resources::ensure_service(client, node, ctx.enable_mtls).await?;

    // 5. Perform health check to determine if node is ready
    let health_result = health::check_node_health(client, node, ctx.mtls_config.as_ref()).await?;

    debug!(
        "Health check result for {}/{}: healthy={}, synced={}, message={}",
        namespace, name, health_result.healthy, health_result.synced, health_result.message
    );

    // 7. Trigger config-reload if VSL was updated and pod is ready
    if let Some(_quorum) = quorum_override {
        if health_result.healthy {
            // Get pod IP to trigger reload
            let pod_api: Api<k8s_openapi::api::core::v1::Pod> =
                Api::namespaced(client.clone(), &namespace);
            let lp = kube::api::ListParams::default()
                .labels(&format!("app.kubernetes.io/instance={}", name));
            if let Ok(pods) = pod_api.list(&lp).await {
                if let Some(pod) = pods.items.first() {
                    if let Some(status) = &pod.status {
                        if let Some(ip) = &status.pod_ip {
                            if let Err(e) = vsl::trigger_config_reload(ip).await {
                                warn!(
                                    "Failed to trigger config-reload for {}/{}: {}",
                                    namespace, name, e
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // 6. Auto-remediation check for stale nodes
    // Only check if node is healthy but potentially stale (ledger not progressing)
    if health_result.healthy && !node.spec.suspended {
        let stale_check = remediation::check_stale_node(node, health_result.ledger_sequence);

        if stale_check.is_stale {
            warn!(
                "Node {}/{} is stale: ledger stuck at {:?} for {:?} minutes",
                namespace, name, stale_check.current_ledger, stale_check.minutes_since_progress
            );

            if remediation::can_remediate(node) {
                match stale_check.recommended_action {
                    remediation::RemediationLevel::Restart => {
                        info!(
                            "Initiating pod restart remediation for {}/{}",
                            namespace, name
                        );

                        // Emit event before remediation
                        remediation::emit_remediation_event(
                            client,
                            node,
                            remediation::RemediationLevel::Restart,
                            &format!(
                                "Ledger stuck at {} for {} minutes",
                                stale_check.current_ledger.unwrap_or(0),
                                stale_check.minutes_since_progress.unwrap_or(0)
                            ),
                        )
                        .await?;

                        // Perform restart
                        remediation::restart_pod(client, node).await?;

                        // Update remediation state
                        remediation::update_remediation_state(
                            client,
                            node,
                            stale_check.current_ledger,
                            remediation::RemediationLevel::Restart,
                            true,
                        )
                        .await?;

                        // Update status
                        update_status(
                            client,
                            node,
                            "Remediating",
                            Some("Pod restarted due to stale ledger"),
                            0,
                            false,
                        )
                        .await?;

                        return Ok(Action::requeue(Duration::from_secs(30)));
                    }
                    remediation::RemediationLevel::ClearAndResync => {
                        // This requires manual intervention - just emit event
                        warn!(
                            "Node {}/{} requires manual intervention: restart didn't help",
                            namespace, name
                        );

                        remediation::emit_remediation_event(
                            client,
                            node,
                            remediation::RemediationLevel::ClearAndResync,
                            "Restart didn't resolve stale state. Manual database clear may be required.",
                        )
                        .await?;

                        remediation::update_remediation_state(
                            client,
                            node,
                            stale_check.current_ledger,
                            remediation::RemediationLevel::ClearAndResync,
                            true,
                        )
                        .await?;

                        update_status(
                            client,
                            node,
                            "Degraded",
                            Some("Node stale after restart. Manual intervention required."),
                            0,
                            false,
                        )
                        .await?;

                        return Ok(Action::requeue(Duration::from_secs(60)));
                    }
                    remediation::RemediationLevel::None => {}
                }
            } else {
                debug!(
                    "Remediation cooldown active for {}/{}, skipping",
                    namespace, name
                );
            }
        } else {
            // Node is healthy - update ledger tracking
            remediation::update_remediation_state(
                client,
                node,
                health_result.ledger_sequence,
                remediation::RemediationLevel::None,
                false,
            )
            .await?;
        }
    }

    // Determine the phase based on health check
    let (phase, message) = if false {
        ("Suspended", "Node is suspended".to_string())
    } else if !health_result.healthy {
        ("Creating", health_result.message.clone())
    } else if !health_result.synced {
        ("Syncing", health_result.message.clone())
    } else {
        ("Ready", "Node is healthy and synced".to_string())
    };

    // 6. Update status with health check results

    // 7. Update status with health check results
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

    // 8. Create/update NetworkPolicy if configured
    resources::ensure_network_policy(client, node).await?;

    // 9. Fetch the ready replicas from Deployment/StatefulSet status
    let ready_replicas = get_ready_replicas(client, node).await.unwrap_or(0);

    // 9. Update status to Running with ready replica count
    // 9. Update ledger sequence metric if available
    if let Some(ref status) = node.status {
        if let Some(seq) = status.ledger_sequence {
            metrics::set_ledger_sequence(
                &namespace,
                &name,
                &node.spec.node_type.to_string(),
                node.spec.network.passphrase(),
                seq,
            );

            // Calculate ingestion lag if we can get the latest network ledger
            // For now we assume we have a way to track the "latest" known ledger across the cluster
            // or fetch it from a public horizon.
            if let Some(network_latest) = get_latest_network_ledger(&node.spec.network).await.ok() {
                let lag = (network_latest as i64) - (seq as i64);
                metrics::set_ingestion_lag(
                    &namespace,
                    &name,
                    &node.spec.node_type.to_string(),
                    node.spec.network.passphrase(),
                    lag.max(0),
                );
            }
        }
    }

    // 10. Update status to Running with ready replica count
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

    // 3a. Delete NetworkPolicy
    if let Err(e) = resources::delete_network_policy(client, node).await {
        warn!("Failed to delete NetworkPolicy: {:?}", e);
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

/// Update status for suspended nodes
async fn update_suspended_status(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    // Set conditions for suspended state
    conditions::set_condition(
        &mut conditions,
        conditions::CONDITION_TYPE_READY,
        conditions::CONDITION_STATUS_FALSE,
        "NodeSuspended",
        "Node is offline - replicas scaled to 0. Service remains active for peer discovery.",
    );
    conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_PROGRESSING);
    conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);

    // Set observed generation on conditions
    if let Some(gen) = node.metadata.generation {
        for condition in &mut conditions {
            condition.observed_generation = Some(gen);
        }
    }

    let status = StellarNodeStatus {
        phase: "Suspended".to_string(),
        message: Some("Node suspended - scaled to 0 replicas".to_string()),
        observed_generation: node.metadata.generation,
        replicas: 0,
        ready_replicas: 0,
        ledger_sequence: None,
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

/// Update the status subresource of a StellarNode using Kubernetes conditions pattern
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

    // Build conditions based on phase
    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    // Map phase to conditions
    match phase {
        "Ready" => {
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_TRUE,
                "AllSubresourcesHealthy",
                message.unwrap_or("All sub-resources are healthy and operational"),
            );
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_FALSE,
                "ReconcileComplete",
                "Reconciliation completed successfully",
            );
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_FALSE,
                "NoIssues",
                "No degradation detected",
            );
        }
        "Creating" | "Pending" => {
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Creating",
                message.unwrap_or("Resources are being created"),
            );
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_TRUE,
                "Creating",
                message.unwrap_or("Creating resources"),
            );
            conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        "Syncing" => {
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Syncing",
                message.unwrap_or("Node is syncing with the network"),
            );
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_TRUE,
                "Syncing",
                message.unwrap_or("Syncing data"),
            );
            conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        "Running" => {
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_TRUE,
                "ResourcesCreated",
                message.unwrap_or("Resources created successfully"),
            );
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_FALSE,
                "Complete",
                "Resource creation complete",
            );
            conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        "Degraded" => {
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Degraded",
                message.unwrap_or("Node is experiencing issues"),
            );
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_TRUE,
                "IssuesDetected",
                message.unwrap_or("Node is degraded"),
            );
            conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_PROGRESSING);
        }
        "Failed" => {
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Failed",
                message.unwrap_or("Node operation failed"),
            );
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_TRUE,
                "Failed",
                message.unwrap_or("Operation failed"),
            );
            conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_PROGRESSING);
        }
        "Remediating" => {
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Remediating",
                message.unwrap_or("Auto-remediation in progress"),
            );
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_TRUE,
                "Remediating",
                message.unwrap_or("Remediation in progress"),
            );
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_TRUE,
                "Remediating",
                "Node required remediation",
            );
        }
        "Suspended" => {
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Suspended",
                message.unwrap_or("Node is suspended"),
            );
            conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_PROGRESSING);
            conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        "Maintenance" => {
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Maintenance",
                message.unwrap_or("Node is in maintenance mode"),
            );
            conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_PROGRESSING);
            conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        _ => {
            conditions::set_condition(
                &mut conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_UNKNOWN,
                "Unknown",
                message.unwrap_or("Status unknown"),
            );
        }
    }

    // Set observed generation on all conditions
    if let Some(gen) = observed_generation {
        for condition in &mut conditions {
            condition.observed_generation = Some(gen);
        }
    }

    let mut status_patch = serde_json::json!({
        "phase": phase,
        "observedGeneration": observed_generation,
        "replicas": if node.spec.suspended { 0 } else { node.spec.replicas },
        "readyReplicas": ready_replicas,
        "conditions": conditions,
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

    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    // Update ArchiveHealthCheck condition
    let archive_message = if result.any_healthy {
        result.summary()
    } else {
        format!("{}\n{}", result.summary(), result.error_details())
    };

    conditions::set_condition(
        &mut conditions,
        "ArchiveHealthCheck",
        if result.any_healthy {
            conditions::CONDITION_STATUS_TRUE
        } else {
            conditions::CONDITION_STATUS_FALSE
        },
        if result.any_healthy {
            "ArchiveHealthy"
        } else {
            "ArchiveUnreachable"
        },
        &archive_message,
    );

    // Set observed generation on conditions
    if let Some(gen) = node.metadata.generation {
        for condition in &mut conditions {
            condition.observed_generation = Some(gen);
        }
    }

    let mut status_patch = serde_json::json!({
        "conditions": conditions,
        "phase": if result.any_healthy { "Creating" } else { "WaitingForArchive" },
    });

    // Don't update observed_generation if archive is unhealthy (to trigger retry)
    if result.any_healthy {
        status_patch["observedGeneration"] = serde_json::json!(node.metadata.generation);
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
    let mut conditions = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    // Ready condition based on health status
    if health.synced {
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_READY,
            conditions::CONDITION_STATUS_TRUE,
            "NodeSynced",
            "Node is fully synced and operational",
        );
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_PROGRESSING,
            conditions::CONDITION_STATUS_FALSE,
            "SyncComplete",
            "Node sync completed",
        );
        conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);
    } else if health.healthy {
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_READY,
            conditions::CONDITION_STATUS_FALSE,
            "NodeSyncing",
            &health.message,
        );
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_PROGRESSING,
            conditions::CONDITION_STATUS_TRUE,
            "Syncing",
            &health.message,
        );
        conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_DEGRADED);
    } else {
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_READY,
            conditions::CONDITION_STATUS_FALSE,
            "NodeNotHealthy",
            &health.message,
        );
        conditions::set_condition(
            &mut conditions,
            conditions::CONDITION_TYPE_DEGRADED,
            conditions::CONDITION_STATUS_TRUE,
            "HealthCheckFailed",
            &health.message,
        );
        conditions::remove_condition(&mut conditions, conditions::CONDITION_TYPE_PROGRESSING);
    }

    // Set observed generation on all conditions
    if let Some(gen) = node.metadata.generation {
        for condition in &mut conditions {
            condition.observed_generation = Some(gen);
        }
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

/// Helper to get the latest ledger from the Stellar network
async fn get_latest_network_ledger(network: &crate::crd::StellarNetwork) -> Result<u64> {
    let url = match network {
        crate::crd::StellarNetwork::Mainnet => "https://horizon.stellar.org",
        crate::crd::StellarNetwork::Testnet => "https://horizon-testnet.stellar.org",
        crate::crd::StellarNetwork::Futurenet => "https://horizon-futurenet.stellar.org",
        crate::crd::StellarNetwork::Custom(_) => {
            return Err(Error::ConfigError(
                "Custom network not supported for lag calculation yet".to_string(),
            ))
        }
    };

    let client = reqwest::Client::new();
    let resp = client.get(url).send().await.map_err(Error::HttpError)?;
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| Error::ConfigError(e.to_string()))?;

    let ledger = json["history_latest_ledger"].as_u64().ok_or_else(|| {
        Error::ConfigError("Failed to get latest ledger from horizon".to_string())
    })?;
    Ok(ledger)
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
