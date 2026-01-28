//! Main reconciler for StellarNode resources
//!
//! Implements the controller pattern using kube-rs runtime.
//! The reconciler watches StellarNode resources and ensures that the desired state
//! (as specified in the StellarNode spec) matches the actual state in the Kubernetes cluster.
//!
//! # Key Components
//!
//! - [`ControllerState`] - Shared state for the controller including the Kubernetes client
//! - [`run_controller`] - Main entry point that starts the controller loop
//!
//! # Reconciliation Workflow
//!
//! 1. Watch for changes to StellarNode resources
//! 2. Validate the StellarNode spec
//! 3. Create/update Kubernetes resources (Deployments, Services, PVCs, etc.)
//! 4. Check node health and sync status
//! 5. Handle node remediation if needed
//! 6. Update StellarNode status with current state
//! 7. Schedule requeue for periodic health checks

use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::policy::v1::PodDisruptionBudget;

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

use crate::crd::{
    DisasterRecoveryStatus, NodeType, RolloutStrategy, SpecValidationError, StellarNode,
    StellarNodeStatus,
};
use crate::error::{Error, Result};

use super::archive_health::{calculate_backoff, check_history_archive_health, ArchiveHealthResult};
use super::conditions;
use super::dr;
use super::finalizers::STELLAR_NODE_FINALIZER;
use super::health;
#[cfg(feature = "metrics")]
use super::metrics;
use super::mtls;
use super::peer_discovery;
use super::remediation;
use super::resources;
use super::vsl;

// Constants
#[allow(dead_code)]
const ARCHIVE_RETRIES_ANNOTATION: &str = "stellar.org/archive-health-retries";

/// Shared state for the controller
///
/// Holds the Kubernetes client and any other shared resources needed by the reconciler.
/// This state is passed to reconcile functions and is used to interact with the Kubernetes API.
pub struct ControllerState {
    /// Kubernetes client for API interactions
    pub client: Client,
    pub enable_mtls: bool,
    pub operator_namespace: String,
    pub mtls_config: Option<crate::MtlsConfig>,
    pub dry_run: bool,
}

/// Main entry point to start the controller
///
/// Initializes and runs the Kubernetes controller loop. The controller:
/// - Watches all StellarNode resources in the cluster
/// - Watches owned resources (Deployments, StatefulSets, Services, PVCs)
/// - Calls the reconcile function whenever a resource changes
/// - Runs until the process receives a shutdown signal
///
/// # Arguments
///
/// * `state` - Controller state containing the Kubernetes client
///
/// # Returns
///
/// Returns `Ok(())` on successful controller shutdown, or an error if the CRD is not installed
/// or another initialization error occurs.
///
/// # Examples
///
/// ```rust,no_run
/// use std::sync::Arc;
/// use stellar_k8s::controller::{ControllerState, run_controller};
/// use kube::Client;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let client = Client::try_default().await?;
///     let state = Arc::new(ControllerState {
///         client,
///         enable_mtls: false,
///         mtls_config: None,
///         operator_namespace: "stellar-operator".to_string(),
///         dry_run: false,
///     });
///     run_controller(state).await?;
///     Ok(())
/// }
/// ```
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
        .owns::<PodDisruptionBudget>(Api::all(client.clone()), Config::default())
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

/// Format structured spec validation errors into a user-friendly message
fn format_spec_validation_errors(errors: &[SpecValidationError]) -> String {
    let mut msg = String::from("Spec validation failed with the following issues:\n");
    for e in errors {
        msg.push_str(&format!(
            "- Field `{}`: {}\n  How to fix: {}\n",
            e.field, e.message, e.how_to_fix
        ));
    }
    msg.trim_end().to_string()
}

/// Emit a single grouped Kubernetes Event for all spec validation errors
async fn emit_spec_validation_event(
    client: &Client,
    node: &StellarNode,
    errors: &[SpecValidationError],
) -> Result<()> {
    let message = format_spec_validation_errors(errors);
    emit_event(client, node, "Warning", "SpecValidationFailed", &message).await
/// Action types for apply_or_emit helper
#[derive(Debug, Clone, Copy)]
pub enum ActionType {
    Create,
    Update,
    Delete,
}

impl std::fmt::Display for ActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionType::Create => write!(f, "create"),
            ActionType::Update => write!(f, "update"),
            ActionType::Delete => write!(f, "delete"),
        }
    }
}

/// Helper to perform an action or emit a "WouldPatch" event in dry-run mode
async fn apply_or_emit<Fut>(
    ctx: &ControllerState,
    node: &StellarNode,
    action: ActionType,
    resource_info: &str,
    fut: Fut,
) -> Result<()>
where
    Fut: std::future::Future<Output = Result<()>>,
{
    if ctx.dry_run {
        let reason = match action {
            ActionType::Create => "WouldCreate",
            ActionType::Update => "WouldUpdate",
            ActionType::Delete => "WouldDelete",
        };
        let message = format!("Dry Run: Would {} {}", action, resource_info);
        info!("{}", message);
        emit_event(&ctx.client, node, "Normal", reason, &message).await?;
        Ok(())
    } else {
        fut.await
    }
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
            FinalizerEvent::Cleanup(node) => cleanup_stellar_node(&client, &node, &ctx).await,
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
    if let Err(errors) = node.spec.validate() {
        let message = format_spec_validation_errors(&errors);
        warn!("Validation failed for {}/{}: {}", namespace, name, message);
        emit_spec_validation_event(client, node, &errors).await?;
        update_status(client, node, "Failed", Some(&message), 0, true).await?;
        return Err(Error::ValidationError(message));
    }

    // 1. Core infrastructure (PVC and ConfigMap) always managed by operator
    apply_or_emit(ctx, node, ActionType::Update, "PVC and ConfigMap", async {
        resources::ensure_pvc(client, node).await?;
        resources::ensure_config_map(client, node, None, ctx.enable_mtls).await?;
        Ok(())
    })
    .await?;

    // 2. Handle suspension
    if node.spec.suspended {
        apply_or_emit(
            ctx,
            node,
            ActionType::Update,
            "Suspended state resources",
            async {
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
                Ok(())
            },
        )
        .await?;

        apply_or_emit(
            ctx,
            node,
            ActionType::Update,
            "Status (Maintenance)",
            async {
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
                Ok(())
            },
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

    // Handle Horizon database migrations
    if node.spec.node_type == NodeType::Horizon {
        if let Some(horizon_config) = &node.spec.horizon_config {
            if horizon_config.auto_migration {
                let current_version = &node.spec.version;
                let last_migrated = node
                    .status
                    .as_ref()
                    .and_then(|s| s.last_migrated_version.as_ref());

                if last_migrated.map(|v| v != current_version).unwrap_or(true) {
                    info!(
                        "Database migration required for Horizon {}/{} (version: {})",
                        namespace, name, current_version
                    );

                    emit_event(
                        client,
                        node,
                        "Normal",
                        "DatabaseMigrationRequired",
                        &format!(
                            "Database migration will be performed via InitContainer for version {}",
                            current_version
                        ),
                    )
                    .await?;
                }
            }
        }
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
                        apply_or_emit(
                            ctx,
                            node,
                            ActionType::Update,
                            "Status (Archive Health Failed)",
                            async {
                                update_archive_health_status(client, node, &health_result).await?;
                                Ok(())
                            },
                        )
                        .await?;

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
                        apply_or_emit(
                            ctx,
                            node,
                            ActionType::Update,
                            "Status (Archive Health Passed)",
                            async {
                                update_archive_health_status(client, node, &health_result).await?;
                                Ok(())
                            },
                        )
                        .await?;
                    }
                }
            }
        }
    }

    // Update status to Creating
    apply_or_emit(ctx, node, ActionType::Update, "Status (Creating)", async {
        update_status(
            client,
            node,
            "Creating",
            Some("Creating resources"),
            0,
            true,
        )
        .await?;
        Ok(())
    })
    .await?;

    // 1. Create/update the PersistentVolumeClaim
    apply_or_emit(ctx, node, ActionType::Create, "PVC", async {
        resources::ensure_pvc(client, node).await?;
        Ok(())
    })
    .await?;
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
    apply_or_emit(ctx, node, ActionType::Update, "ConfigMap", async {
        resources::ensure_config_map(client, node, quorum_override.clone(), ctx.enable_mtls)
            .await?;
        Ok(())
    })
    .await?;
    info!("ConfigMap ensured for {}/{}", namespace, name);

    // 3. Handle suspension or Maintenance
    if node.spec.maintenance_mode {
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

    if node.spec.suspended {
        info!("Node {}/{} is suspended, scaling to 0", namespace, name);
        apply_or_emit(ctx, node, ActionType::Update, "Status (Suspended)", async {
            update_suspended_status(client, node).await?;
            Ok(())
        })
        .await?;
        // Continue to ensure resources exist but with 0 replicas
    }

    // 4. Ensure mTLS certificates
    apply_or_emit(ctx, node, ActionType::Update, "mTLS certificates", async {
        mtls::ensure_ca(client, &namespace).await?;
        mtls::ensure_node_cert(client, node).await?;
        Ok(())
    })
    .await?;

    // 5. Create/update the Deployment/StatefulSet based on node type
    apply_or_emit(
        ctx,
        node,
        ActionType::Update,
        "Workload (Deployment/StatefulSet)",
        async {
            match node.spec.node_type {
                NodeType::Validator => {
                    resources::ensure_statefulset(client, node, ctx.enable_mtls).await?;
                }
                NodeType::Horizon | NodeType::SorobanRpc => {
                    // Handle Canary Deployment
                    if let RolloutStrategy::Canary(_) = &node.spec.strategy {
                        // Determine if we are in a canary state
                        let current_version = get_current_deployment_version(client, node).await?;
                        if let Some(cv) = current_version {
                            if cv != node.spec.version {
                                // We have a version mismatch, ensure canary
                                info!(
                                    "Canary version mismatch: spec={} current={}. Ensuring canary resources.",
                                    node.spec.version, cv
                                );
                            }
                        }

                        resources::ensure_canary_deployment(client, node, ctx.enable_mtls).await?;
                        resources::ensure_canary_service(client, node, ctx.enable_mtls).await?;

                        // For canary, the main deployment should stay at the OLD version
                        // IF we are in the middle of a rollout.
                        if node
                            .status
                            .as_ref()
                            .and_then(|status| status.canary_version.as_ref())
                            .is_some()
                        {
                            let mut stable_node = node.clone();
                            // Recover the stable version from the existing deployment if possible
                            if let Some(cv) = get_current_deployment_version(client, node).await? {
                                stable_node.spec.version = cv;
                            }
                            resources::ensure_deployment(client, &stable_node, ctx.enable_mtls).await?;
                        } else {
                            resources::ensure_deployment(client, node, ctx.enable_mtls).await?;
                        }
                    } else {
                        // RPC nodes use Deployment
                        resources::ensure_deployment(client, node, ctx.enable_mtls).await?;
                        info!("Deployment ensured for RPC node {}/{}", namespace, name);

                        // Clean up canary resources if they exist
                        resources::delete_canary_resources(client, node).await?;
                    }
                }
            }
            Ok(())
        },
    )
    .await?;

    apply_or_emit(
        ctx,
        node,
        ActionType::Update,
        "Service and Ingress",
        async {
            resources::ensure_service(client, node, ctx.enable_mtls).await?;
            resources::ensure_ingress(client, node).await?;
            Ok(())
        },
    )
    .await?;

    // 5a. MetalLB / LoadBalancer
    apply_or_emit(
        ctx,
        node,
        ActionType::Update,
        "MetalLB configuration",
        async {
            resources::ensure_metallb_config(client, node).await?;
            resources::ensure_load_balancer_service(client, node).await?;
            Ok(())
        },
    )
    .await?;

    // 6. Autoscaling and Monitoring
    apply_or_emit(
        ctx,
        node,
        ActionType::Update,
        "Monitoring and Scaling resources",
        async {
            if node.spec.autoscaling.is_some() {
                resources::ensure_service_monitor(client, node).await?;
                resources::ensure_hpa(client, node).await?;
            }
            resources::ensure_pdb(client, node).await?;
            resources::ensure_alerting(client, node).await?;
            resources::ensure_network_policy(client, node).await?;
            Ok(())
        },
    )
    .await?;

    // 7. Perform health check to determine if node is ready
    let health_result = health::check_node_health(client, node, ctx.mtls_config.as_ref()).await?;

    debug!(
        "Health check result for {}/{}: healthy={}, synced={}, message={}",
        namespace, name, health_result.healthy, health_result.synced, health_result.message
    );

    // 6. Trigger peer configuration reload for validators if healthy
    if node.spec.node_type == NodeType::Validator && health_result.healthy {
        if let Err(e) = peer_discovery::trigger_peer_config_reload(client, node).await {
            warn!(
                "Failed to trigger peer config reload for {}/{}: {}",
                namespace, name, e
            );
        }
    }

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

    // 8. Disaster Recovery reconciliation
    if let Some(dr_status) = dr::reconcile_dr(client, node).await? {
        apply_or_emit(ctx, node, ActionType::Update, "Status (DR)", async {
            update_dr_status(client, node, dr_status).await?;
            Ok(())
        })
        .await?;
    }

    // 9. Auto-remediation check
    if health_result.healthy && !node.spec.suspended {
        let stale_check = remediation::check_stale_node(node, health_result.ledger_sequence);
        if stale_check.is_stale && remediation::can_remediate(node) {
            if stale_check.recommended_action == remediation::RemediationLevel::Restart {
                apply_or_emit(
                    ctx,
                    node,
                    ActionType::Update,
                    "Remediation (Restart)",
                    async {
                        remediation::emit_remediation_event(
                            client,
                            node,
                            remediation::RemediationLevel::Restart,
                            "Stale ledger",
                        )
                        .await?;
                        remediation::restart_pod(client, node).await?;
                        remediation::update_remediation_state(
                            client,
                            node,
                            stale_check.current_ledger,
                            remediation::RemediationLevel::Restart,
                            true,
                        )
                        .await?;
                        Ok(())
                    },
                )
                .await?;
                return Ok(Action::requeue(Duration::from_secs(30)));
            }
        } else {
            apply_or_emit(ctx, node, ActionType::Update, "Remediation State", async {
                remediation::update_remediation_state(
                    client,
                    node,
                    health_result.ledger_sequence,
                    remediation::RemediationLevel::None,
                    false,
                )
                .await?;
                Ok(())
            })
            .await?;
        }
    }

    // 10. Final Status Update
    let (phase, message) = if node.spec.suspended {
        ("Suspended", "Node is suspended".to_string())
    } else if !health_result.healthy {
        ("Creating", health_result.message.clone())
    } else if !health_result.synced {
        ("Syncing", health_result.message.clone())
    } else {
        ("Ready", "Node is healthy and synced".to_string())
    };

    apply_or_emit(ctx, node, ActionType::Update, "Status (Final)", async {
        update_status_with_health(client, node, phase, Some(&message), &health_result).await?;

        let ready_replicas = get_ready_replicas(client, node).await.unwrap_or(0);
        update_status(client, node, phase, Some(&message), ready_replicas, true).await?;
        Ok(())
    })
    .await?;

    // 9. Update status with ready replica count
    let phase = if node.spec.suspended {
        "Suspended"
    } else if node
        .status
        .as_ref()
        .and_then(|status| status.canary_version.as_ref())
        .is_some()
    {
        "Canary"
    } else {
        "Running"
    };

    // 10. Update ledger sequence metric if available
    if let Some(ref status) = node.status {
        #[cfg(feature = "metrics")]
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
            if let Ok(network_latest) = get_latest_network_ledger(&node.spec.network).await {
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
    Ok(Action::requeue(Duration::from_secs(if phase == "Ready" {
        60
    } else {
        15
    })))
}

/// Clean up resources when the StellarNode is deleted
#[instrument(skip(client, node, ctx), fields(name = %node.name_any(), namespace = node.namespace()))]
async fn cleanup_stellar_node(
    client: &Client,
    node: &StellarNode,
    ctx: &ControllerState,
) -> Result<Action> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!("Cleaning up StellarNode: {}/{}", namespace, name);

    // Delete resources in reverse order of creation

    // 0. Delete Alerting
    apply_or_emit(ctx, node, ActionType::Delete, "Alerting", async {
        if let Err(e) = resources::delete_alerting(client, node).await {
            warn!("Failed to delete alerting: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 1. Delete HPA (if autoscaling was configured)
    apply_or_emit(ctx, node, ActionType::Delete, "HPA", async {
        if let Err(e) = resources::delete_hpa(client, node).await {
            warn!("Failed to delete HPA: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 2. Delete ServiceMonitor (if autoscaling was configured)
    apply_or_emit(ctx, node, ActionType::Delete, "ServiceMonitor", async {
        if let Err(e) = resources::delete_service_monitor(client, node).await {
            warn!("Failed to delete ServiceMonitor: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 3. Delete Ingress
    apply_or_emit(ctx, node, ActionType::Delete, "Ingress", async {
        if let Err(e) = resources::delete_ingress(client, node).await {
            warn!("Failed to delete Ingress: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 3a. Delete NetworkPolicy
    apply_or_emit(ctx, node, ActionType::Delete, "NetworkPolicy", async {
        if let Err(e) = resources::delete_network_policy(client, node).await {
            warn!("Failed to delete NetworkPolicy: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 3b. Delete MetalLB LoadBalancer Service
    apply_or_emit(
        ctx,
        node,
        ActionType::Delete,
        "MetalLB LoadBalancer",
        async {
            if let Err(e) = resources::delete_load_balancer_service(client, node).await {
                warn!("Failed to delete MetalLB LoadBalancer service: {:?}", e);
            }
            if let Err(e) = resources::delete_metallb_config(client, node).await {
                warn!("Failed to delete MetalLB configuration: {:?}", e);
            }
            Ok(())
        },
    )
    .await?;

    // 3c. Delete PDB
    apply_or_emit(ctx, node, ActionType::Delete, "PDB", async {
        if let Err(e) = resources::delete_pdb(client, node).await {
            warn!("Failed to delete PodDisruptionBudget: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 3b. Delete MetalLB LoadBalancer Service
    apply_or_emit(
        ctx,
        node,
        ActionType::Delete,
        "MetalLB LoadBalancer",
        async {
            if let Err(e) = resources::delete_load_balancer_service(client, node).await {
                warn!("Failed to delete MetalLB LoadBalancer service: {:?}", e);
            }
            if let Err(e) = resources::delete_metallb_config(client, node).await {
                warn!("Failed to delete MetalLB configuration: {:?}", e);
            }
            Ok(())
        },
    )
    .await?;
    // 3c. Delete PDB
    apply_or_emit(ctx, node, ActionType::Delete, "PDB", async {
        if let Err(e) = resources::delete_pdb(client, node).await {
            warn!("Failed to delete PodDisruptionBudget: {:?}", e);
        }
        Ok(())
    })
    .await?;
    // 4. Delete Service
    apply_or_emit(ctx, node, ActionType::Delete, "Service", async {
        if let Err(e) = resources::delete_service(client, node).await {
            warn!("Failed to delete Service: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 5. Delete Deployment/StatefulSet
    apply_or_emit(ctx, node, ActionType::Delete, "Workload", async {
        if let Err(e) = resources::delete_workload(client, node).await {
            warn!("Failed to delete workload: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 6. Delete ConfigMap
    apply_or_emit(ctx, node, ActionType::Delete, "ConfigMap", async {
        if let Err(e) = resources::delete_config_map(client, node).await {
            warn!("Failed to delete ConfigMap: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 7. Delete PVC based on retention policy
    if node.spec.should_delete_pvc() {
        info!(
            "Deleting PVC for node: {}/{} (retention policy: Delete)",
            namespace, name
        );
        apply_or_emit(ctx, node, ActionType::Delete, "PVC", async {
            if let Err(e) = resources::delete_pvc(client, node).await {
                warn!("Failed to delete PVC: {:?}", e);
            }
            Ok(())
        })
        .await?;
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

/// Fetch the ready replicas for the canary deployment
#[allow(dead_code)]
async fn get_canary_ready_replicas(client: &Client, node: &StellarNode) -> Result<i32> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = format!("{}-canary", node.name_any());

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
        Err(_) => Ok(0),
    }
}

/// Get the current version of the stable deployment
async fn get_current_deployment_version(
    client: &Client,
    node: &StellarNode,
) -> Result<Option<String>> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
    match api.get(&name).await {
        Ok(deployment) => {
            let version = deployment
                .spec
                .as_ref()
                .and_then(|s| s.template.spec.as_ref())
                .and_then(|ts| ts.containers.first())
                .and_then(|c| c.image.as_ref())
                .and_then(|img| img.split(':').next_back())
                .map(|v| v.to_string());
            Ok(version)
        }
        Err(_) => Ok(None),
    }
}

/// Check health of canary pods
#[allow(dead_code)]
async fn check_canary_health(
    client: &Client,
    node: &StellarNode,
) -> Result<health::HealthCheckResult> {
    let _namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = format!("{}-canary", node.name_any());

    // Create a temporary node with the canary name to use the existing health check logic
    let mut canary_node = node.clone();
    canary_node.metadata.name = Some(name);

    health::check_node_health(client, &canary_node, None).await
}

/// Update status for suspended nodes
#[allow(deprecated)]
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
#[allow(deprecated)]
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
#[allow(deprecated)]
async fn update_status_with_health(
    client: &Client,
    node: &StellarNode,
    _phase: &str,
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
        last_migrated_version: if health.synced && node.spec.node_type == NodeType::Horizon {
            Some(node.spec.version.clone())
        } else {
            node.status
                .as_ref()
                .and_then(|s| s.last_migrated_version.clone())
        },
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

/// Update the status subresource with canary information
#[allow(dead_code)]
async fn update_status_with_canary(
    client: &Client,
    node: &StellarNode,
    phase: &str,
    message: Option<&str>,
    ready_replicas: i32,
    canary_ready_replicas: i32,
    canary_version: Option<String>,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    #[allow(deprecated)]
    let status = StellarNodeStatus {
        phase: phase.to_string(),
        message: message.map(String::from),
        observed_generation: node.metadata.generation,
        replicas: if node.spec.suspended {
            0
        } else {
            node.spec.replicas
        },
        ready_replicas,
        canary_ready_replicas,
        canary_version,
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
/// Update the status with DR results
async fn update_dr_status(
    client: &Client,
    node: &StellarNode,
    dr_status: DisasterRecoveryStatus,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let patch = serde_json::json!({
        "status": {
            "drStatus": dr_status
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
    error!("Reconciliation error for {}: {:?}", node.name_any(), error);

    // Use shorter retry for retriable errors
    let retry_duration = if error.is_retriable() {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(60)
    };

    Action::requeue(retry_duration)
}
