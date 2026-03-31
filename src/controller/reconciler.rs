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

use chrono::Utc;
use k8s_openapi::api::policy::v1::PodDisruptionBudget;

use futures::StreamExt;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Service};
use kube::{
    api::{Api, Patch, PatchParams},
    client::Client,
    runtime::{
        controller::{Action, Controller},
        events::{Event as K8sRecorderEvent, EventType, Recorder, Reporter},
        finalizer::{finalizer, Event as FinalizerEvent},
        watcher::Config,
    },
    Resource, ResourceExt,
};
use tracing::{debug, error, info, info_span, instrument, warn};
use tracing_subscriber::{reload::Handle, EnvFilter, Registry};

use crate::crd::{
    Condition, DisasterRecoveryStatus, NodeType, RolloutStrategy, SpecValidationError, StellarNode,
    StellarNodeStatus,
};
use crate::error::{Error, Result};
use crate::infra;

use super::archive_health::{
    calculate_backoff, check_archive_integrity, check_archive_integrity_random,
    check_history_archive_health, ArchiveHealthResult, ArchiveIntegrityCheckResult,
    ARCHIVE_LAG_THRESHOLD,
};
use super::conditions;
use super::cve_reconciler;
use super::dr;
use super::dr_drill;
use super::finalizers::STELLAR_NODE_FINALIZER;
use super::health;
use super::kms_secret;
use super::label_propagation::LabelPropagator;
#[cfg(feature = "metrics")]
use super::metrics;
use super::mtls;
use super::oci_snapshot;
use super::operator_config::{hardcoded_defaults, OperatorConfig};
use super::peer_discovery;
use super::remediation;
use super::resources;
use super::service_mesh;
use super::vpa as vpa_controller;
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
    /// Restrict the operator to only watch and manage StellarNode resources in this namespace.
    /// If None, the operator watches all namespaces.
    pub watch_namespace: Option<String>,
    pub mtls_config: Option<crate::MtlsConfig>,
    pub dry_run: bool,
    pub is_leader: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Identifies this operator when publishing Kubernetes Events via [`Recorder`].
    pub event_reporter: Reporter,
    /// Operator-level config loaded from the Helm-rendered ConfigMap (defaultResources).
    pub operator_config: std::sync::Arc<OperatorConfig>,
    /// Counter for generating unique reconcile IDs
    pub reconcile_id_counter: std::sync::atomic::AtomicU64,
    /// Timestamp of the last successful reconcile
    pub last_reconcile_success: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Handle to reload the tracing filter
    pub log_reload_handle: Handle<EnvFilter, Registry>,
    /// Optional expiration time for a temporary log level change
    pub log_level_expires_at:
        std::sync::Arc<tokio::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>>,
    pub log_level_expires_at: std::sync::Arc<tokio::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>>,
    /// Timestamp of the last event received from the K8s watch stream
    pub last_event_received: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl ControllerState {
    /// Generate a unique reconcile ID
    pub fn next_reconcile_id(&self) -> u64 {
        self.reconcile_id_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
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
/// use std::sync::atomic::{AtomicBool, AtomicU64};
/// use stellar_k8s::controller::{ControllerState, run_controller};
/// use kube::Client;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let client = Client::try_default().await?;
///     let env_filter = tracing_subscriber::EnvFilter::from_default_env();
///     let (_layer, reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);
///     let state = Arc::new(ControllerState {
///         client,
///         enable_mtls: false,
///         operator_namespace: "stellar-operator".to_string(),
///         watch_namespace: None,
///         mtls_config: None,
///         dry_run: false,
///         is_leader: Arc::new(AtomicBool::new(true)),
///         event_reporter: kube::runtime::events::Reporter {
///             controller: "stellar-operator".to_string(),
///             instance: None,
///         },
///         operator_config: Arc::new(Default::default()),
///         reconcile_id_counter: AtomicU64::new(0),
///         last_reconcile_success: Arc::new(AtomicU64::new(0)),
///         log_reload_handle: reload_handle,
///         log_level_expires_at: Arc::new(tokio::sync::Mutex::new(None)),
///     });
///     run_controller(state).await?;
///     Ok(())
/// }
/// ```
pub async fn run_controller(state: Arc<ControllerState>) -> Result<()> {
    let client = state.client.clone();
    let stellar_nodes: Api<StellarNode> = if let Some(ns) = &state.watch_namespace {
        Api::namespaced(client.clone(), ns)
    } else {
        Api::all(client.clone())
    };

    info!(
        "Starting StellarNode controller (mode: {})",
        if let Some(ns) = &state.watch_namespace {
            format!("namespace-scoped: {}", ns)
        } else {
            "cluster-scoped".to_string()
        }
    );

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
        .owns::<Deployment>(
            if let Some(ns) = &state.watch_namespace {
                Api::namespaced(client.clone(), ns)
            } else {
                Api::all(client.clone())
            },
            Config::default(),
        )
        .owns::<StatefulSet>(
            if let Some(ns) = &state.watch_namespace {
                Api::namespaced(client.clone(), ns)
            } else {
                Api::all(client.clone())
            },
            Config::default(),
        )
        .owns::<Service>(
            if let Some(ns) = &state.watch_namespace {
                Api::namespaced(client.clone(), ns)
            } else {
                Api::all(client.clone())
            },
            Config::default(),
        )
        .owns::<PersistentVolumeClaim>(
            if let Some(ns) = &state.watch_namespace {
                Api::namespaced(client.clone(), ns)
            } else {
                Api::all(client.clone())
            },
            Config::default(),
        )
        .owns::<PodDisruptionBudget>(
            if let Some(ns) = &state.watch_namespace {
                Api::namespaced(client.clone(), ns)
            } else {
                Api::all(client.clone())
            },
            Config::default(),
        )
        .shutdown_on_signal()
        .run(reconcile, error_policy, state.clone())
        .inspect({
            let state = state.clone();
            move |_| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                state
                    .last_event_received
                    .store(now, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .for_each(|_res| async {})
        .await;

    Ok(())
}

fn recorder_for(client: &Client, reporter: &Reporter, node: &StellarNode) -> Recorder {
    Recorder::new(client.clone(), reporter.clone(), node.object_ref(&()))
}

/// Publish a Kubernetes Event attached to the StellarNode using kube-rs [`Recorder`].
async fn publish_object_event(
    recorder: &Recorder,
    type_: EventType,
    reason: &str,
    action: &str,
    note: &str,
) -> Result<()> {
    recorder
        .publish(K8sRecorderEvent {
            type_,
            reason: reason.to_string(),
            action: action.to_string(),
            note: Some(note.to_string()),
            secondary: None,
        })
        .await
        .map_err(Error::KubeError)
}

/// Helper to emit a Kubernetes Event
#[instrument(skip(client, reporter, node, reason, note), fields(name = %node.name_any(), namespace = node.namespace()))]
async fn emit_event(
    client: &Client,
    reporter: &Reporter,
    node: &StellarNode,
    type_: EventType,
    reason: &str,
    action: &str,
    note: &str,
) -> Result<()> {
    let recorder = recorder_for(client, reporter, node);
    publish_object_event(&recorder, type_, reason, action, note).await
}

/// Convenience wrapper — identical to [`emit_event`]; used by callers that
/// prefer the `publish_stellar_event` name for clarity.
async fn publish_stellar_event(
    client: &Client,
    reporter: &Reporter,
    node: &StellarNode,
    type_: EventType,
    reason: &str,
    action: &str,
    note: &str,
) -> Result<()> {
    emit_event(client, reporter, node, type_, reason, action, note).await
}

/// Returns whether the primary workload (Deployment or StatefulSet) for this node already exists.
async fn workload_resource_exists(client: &Client, node: &StellarNode) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    match node.spec.node_type {
        NodeType::Validator => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
            match api.get(&name).await {
                Ok(_) => Ok(true),
                Err(kube::Error::Api(e)) if e.code == 404 => Ok(false),
                Err(e) => Err(Error::KubeError(e)),
            }
        }
        NodeType::Horizon | NodeType::SorobanRpc => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
            match api.get(&name).await {
                Ok(_) => Ok(true),
                Err(kube::Error::Api(e)) if e.code == 404 => Ok(false),
                Err(e) => Err(Error::KubeError(e)),
            }
        }
    }
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
    reporter: &Reporter,
    node: &StellarNode,
    errors: &[SpecValidationError],
) -> Result<()> {
    let message = format_spec_validation_errors(errors);
    publish_stellar_event(
        client,
        reporter,
        node,
        EventType::Warning,
        "SpecValidationFailed",
        "ValidationFailed",
        &message,
    )
    .await
}
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
        let message = format!("Dry Run: Would {action} {resource_info}");
        info!("{}", message);
        publish_stellar_event(
            &ctx.client,
            &ctx.event_reporter,
            node,
            EventType::Normal,
            reason,
            "DryRun",
            &message,
        )
        .await?;
        // Enhanced logging with resource type and namespace
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let name = node.name_any();
        debug!(
            "Dry Run: {} {}/{} - {}",
            action, namespace, name, resource_info
        );
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
async fn reconcile(obj: Arc<StellarNode>, ctx: Arc<ControllerState>) -> Result<Action> {
    let node_name = obj.name_any();
    let namespace = obj.namespace().unwrap_or_else(|| "default".to_string());
    let reconcile_id = ctx.next_reconcile_id();

    let node_name_for_span = node_name.clone();
    let namespace_for_span = namespace.clone();
    let resource_version = obj
        .metadata
        .resource_version
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    // Attach per-reconcile structured fields so every log event during reconciliation
    // can be correlated in JSON logs (node_name/namespace/reconcile_id/resource_version).
    let _reconcile_span = info_span!(
        "reconcile_attempt",
        node_name = %node_name_for_span,
        namespace = %namespace_for_span,
        reconcile_id = %reconcile_id,
        resource_version = %resource_version
    );
    let _reconcile_enter = _reconcile_span.enter();

    #[cfg(feature = "metrics")]
    let reconcile_start = std::time::Instant::now();

    if !ctx.is_leader.load(std::sync::atomic::Ordering::Relaxed) {
        debug!("Not the leader, skipping reconciliation");
        return Ok(Action::requeue(Duration::from_secs(5)));
    }

    let res = {
        let client = ctx.client.clone();
        let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

        info!(
            "Reconciling StellarNode {}/{} (type: {:?})",
            namespace, node_name, obj.spec.node_type
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
    };

    #[cfg(feature = "metrics")]
    {
        let seconds = reconcile_start.elapsed().as_secs_f64();
        metrics::observe_reconcile_duration_seconds("stellarnode", seconds);
        if let Err(err) = &res {
            // Keep the label cardinality low: a few broad error kinds.
            let kind = match err {
                Error::KubeError(_) => "kube",
                Error::ValidationError(_) => "validation",
                Error::ConfigError(_) => "config",
                _ => "unknown",
            };
            metrics::inc_reconcile_error("stellarnode", kind);
            metrics::inc_operator_reconcile_error("stellarnode", kind);
        } else {
            // Record successful reconciliation timestamp
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            ctx.last_reconcile_success
                .store(now, std::sync::atomic::Ordering::Relaxed);
        }
    }

    res
}

/// Apply/create/update the StellarNode resources
#[instrument(skip(client, node, ctx), fields(name = %node.name_any(), namespace = node.namespace()))]
pub(crate) async fn apply_stellar_node(
    client: &Client,
    node: &StellarNode,
    ctx: &ControllerState,
) -> Result<Action> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!("Applying StellarNode: {}/{}", namespace, name);

    // Resolve effective resource requirements:
    // Precedence: spec.resources (non-empty) > Helm defaults > hardcoded fallback.
    let effective_resources = {
        let spec_resources = &node.spec.resources;
        if !spec_resources.requests.cpu.is_empty() {
            // Spec wins — use as-is
            spec_resources.clone()
        } else if let Some(helm_d) = ctx.operator_config.defaults_for(&node.spec.node_type) {
            crate::crd::ResourceRequirements {
                requests: crate::crd::ResourceSpec {
                    cpu: helm_d.requests.cpu.clone(),
                    memory: helm_d.requests.memory.clone(),
                },
                limits: crate::crd::ResourceSpec {
                    cpu: helm_d.limits.cpu.clone(),
                    memory: helm_d.limits.memory.clone(),
                },
            }
        } else {
            hardcoded_defaults(&node.spec.node_type)
        }
    };
    debug!(
        "Effective resources for {}/{}: requests={}/{} limits={}/{}",
        namespace,
        name,
        effective_resources.requests.cpu,
        effective_resources.requests.memory,
        effective_resources.limits.cpu,
        effective_resources.limits.memory,
    );

    // Validate the spec
    if let Err(errors) = node.spec.validate() {
        let message = format_spec_validation_errors(&errors);
        warn!("Validation failed for {}/{}: {}", namespace, name, message);
        emit_spec_validation_event(client, &ctx.event_reporter, node, &errors).await?;
        update_status(client, node, "Failed", Some(&message), 0, true).await?;
        return Err(Error::ValidationError(message));
    }

    let propagated_labels = LabelPropagator::new(node).compute();

    // 1. Core infrastructure (PVC and ConfigMap) always managed by operator
    apply_or_emit(ctx, node, ActionType::Update, "PVC and ConfigMap", async {
        resources::ensure_pvc(client, node, &propagated_labels, ctx.dry_run).await?;
        resources::ensure_config_map(client, node, None, ctx.enable_mtls, ctx.dry_run).await?;
        Ok(())
    })
    .await?;

    // 1a. Managed Database (CloudNativePG)
    apply_or_emit(ctx, node, ActionType::Update, "Managed Database", async {
        resources::ensure_cnpg_cluster(client, node, ctx.dry_run).await?;
        resources::ensure_cnpg_pooler(client, node, ctx.dry_run).await?;
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
                resources::ensure_pvc(client, node, &propagated_labels, ctx.dry_run).await?;
                resources::ensure_config_map(client, node, None, ctx.enable_mtls, ctx.dry_run)
                    .await?;

                match node.spec.node_type {
                    NodeType::Validator => {
                        // Suspended validators don't need seed injection resolved
                        resources::ensure_statefulset(
                            client,
                            node,
                            ctx.enable_mtls,
                            None,
                            &propagated_labels,
                            ctx.dry_run,
                        )
                        .await?;
                    }
                    NodeType::Horizon | NodeType::SorobanRpc => {
                        resources::ensure_deployment(
                            client,
                            node,
                            ctx.enable_mtls,
                            &propagated_labels,
                            ctx.dry_run,
                        )
                        .await?;
                    }
                }

                resources::ensure_service(
                    client,
                    node,
                    ctx.enable_mtls,
                    &propagated_labels,
                    ctx.dry_run,
                )
                .await?;
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

                    publish_stellar_event(
                        client,
                        &ctx.event_reporter,
                        node,
                        EventType::Normal,
                        "DatabaseMigrationRequired",
                        "Migrate",
                        &format!(
                            "Database migration will be performed via InitContainer for version {current_version}"
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
                        publish_stellar_event(
                            client,
                            &ctx.event_reporter,
                            node,
                            EventType::Warning,
                            "ArchiveHealthCheckFailed",
                            "ArchiveHealth",
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

    // Periodic archive integrity check (every 1 hour) for validators with archive enabled.
    // This compares stellar-history.json ledger sequences against the validator's current
    // ledger and sets/clears the ArchiveIntegrityDegraded condition + Prometheus alert metric.
    if node.spec.node_type == NodeType::Validator {
        if let Some(validator_config) = &node.spec.validator_config {
            if validator_config.enable_history_archive
                && !validator_config.history_archive_urls.is_empty()
            {
                const ARCHIVE_CHECK_INTERVAL_SECS: i64 = 3600;
                let last_check_time = node
                    .status
                    .as_ref()
                    .and_then(|s| {
                        s.conditions
                            .iter()
                            .find(|c| c.type_ == "ArchiveIntegrityDegraded")
                            .map(|c| c.last_transition_time.clone())
                    })
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                let should_run = match last_check_time {
                    None => true, // never checked
                    Some(last) => {
                        let age_secs = (chrono::Utc::now() - last).num_seconds();
                        age_secs >= ARCHIVE_CHECK_INTERVAL_SECS
                    }
                };

                if should_run {
                    if let Err(e) = run_archive_integrity_check(
                        client,
                        &ctx.event_reporter,
                        node,
                        &validator_config.history_archive_urls,
                    )
                    .await
                    {
                        warn!(
                            "Archive integrity check error for {}/{}: {}",
                            namespace, name, e
                        );
                    }
                }
            }

            // Automatic Checkpoint Integrity Check
            if let Some(archive_config) = &validator_config.archive_integrity_config {
                if archive_config.enabled && !validator_config.history_archive_urls.is_empty() {
                    let interval = match parse_duration(&archive_config.interval) {
                        Ok(d) => d,
                        Err(_) => Duration::from_secs(21600), // Default 6h
                    };

                    let last_check_time = node
                        .status
                        .as_ref()
                        .and_then(|s| {
                            s.conditions
                                .iter()
                                .find(|c| c.type_ == "ArchiveIntegrityCheck")
                                .map(|c| c.last_transition_time.clone())
                        })
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
                        .map(|dt| dt.with_timezone(&chrono::Utc));

                    let should_run = match last_check_time {
                        None => true,
                        Some(last) => {
                            let age = chrono::Utc::now() - last;
                            age.to_std().unwrap_or(Duration::from_secs(0)) >= interval
                        }
                    };

                    if should_run {
                        if let Err(e) = run_archive_checkpoint_verification(
                            client,
                            &ctx.event_reporter,
                            node,
                            &validator_config.history_archive_urls,
                            archive_config,
                        )
                        .await
                        {
                            warn!(
                                "Archive checkpoint verification error for {}/{}: {}",
                                namespace, name, e
                            );
                        }
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
        resources::ensure_pvc(client, node, &propagated_labels, ctx.dry_run).await?;
        Ok(())
    })
    .await?;
    info!("PVC ensured for {}/{}", namespace, name);

    // 2. Handle VSL Fetching for Validators
    let mut quorum_override: Option<crate::controller::vsl::QuorumSet> = None;
    if node.spec.node_type == NodeType::Validator {
        if let Some(config) = &node.spec.validator_config {
            if let Some(vl_source) = &config.vl_source {
                match vsl::fetch_vsl(vl_source).await {
                    Ok(quorum) => {
                        quorum_override = Some(quorum);
                    }
                    Err(e) => {
                        warn!("Failed to fetch VSL for {}/{}: {}", namespace, name, e);
                        publish_stellar_event(
                            client,
                            &ctx.event_reporter,
                            node,
                            EventType::Warning,
                            "VSLFetchFailed",
                            "VSLFetch",
                            &format!("Failed to fetch VSL from {vl_source}: {e}"),
                        )
                        .await?;
                    }
                }
            }
        }
    }

    // 3. Create/update the ConfigMap for node configuration
    apply_or_emit(ctx, node, ActionType::Update, "ConfigMap", async {
        resources::ensure_config_map(
            client,
            node,
            quorum_override.clone(),
            ctx.enable_mtls,
            ctx.dry_run,
        )
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

    let workload_existed_before = workload_resource_exists(client, node)
        .await
        .unwrap_or(false);

    // 5. Create/update the Deployment/StatefulSet based on node type
    let workload_result = apply_or_emit(
        ctx,
        node,
        ActionType::Update,
        "Workload (Deployment/StatefulSet)",
        async {
            match node.spec.node_type {
                NodeType::Validator => {
                    // Resolve the KMS/ESO/CSI seed injection spec before building the StatefulSet.
                    // Creates any required ExternalSecret CR and returns a lightweight descriptor
                    // of how to wire the seed into the pod. No secret values are ever read.
                    let seed_injection = if let Some(validator_config) = &node.spec.validator_config {
                        if let Some(_source) = validator_config.resolve_seed_source() {
                            match kms_secret::reconcile_seed_secret(client, node).await {
                                Ok(spec) => Some(spec),
                                Err(e) => {
                                    warn!(
                                        "Seed secret reconciliation failed for {}/{}: {}. \
                                         Falling back to legacy seed_secret_ref behaviour.",
                                        namespace, name, e
                                    );
                                    None
                                }
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    resources::ensure_statefulset(
                        client,
                        node,
                        ctx.enable_mtls,
                        seed_injection.as_ref(),
                        &propagated_labels,
                        ctx.dry_run,
                    )
                    .await?;
                    kms_secret::reconcile_vault_secret_rotation(
                        client,
                        node,
                        seed_injection.as_ref(),
                    )
                    .await?;
                    super::forensic_snapshot::reconcile_forensic_snapshot(client, node).await?;
                }
                NodeType::Horizon | NodeType::SorobanRpc => {
                    // Handle Canary Deployment
                    if let Some(cfg) = node.spec.strategy.canary() {
                        // Determine if we are in a canary state
                        let current_version = get_current_deployment_version(client, node).await?;

                        // Check if we already have an active canary
                        let mut is_canary_active = node
                            .status
                            .as_ref()
                            .and_then(|status| status.canary_version.as_ref())
                            .is_some();

                        if !is_canary_active {
                            if let Some(cv) = &current_version {
                                if cv != &node.spec.version {
                                    // 1. Start Canary: We have a version mismatch, start canary
                                    info!(
                                        "Canary version mismatch: spec={} current={}. Starting canary.",
                                        node.spec.version, cv
                                    );
                                    let now = chrono::Utc::now().to_rfc3339();

                                    // Update status to indicate canary has started
                                    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
                                    let patch = serde_json::json!({
                                        "status": {
                                            "canaryVersion": node.spec.version,
                                            "canaryStartTime": now,
                                            "phase": "Canary"
                                        }
                                    });
                                    api.patch_status(
                                        &name,
                                        &PatchParams::apply("stellar-operator"),
                                        &Patch::Merge(&patch),
                                    ).await?;

                                    is_canary_active = true;

                                    // We need to fetch the updated node with the new status
                                    // but we can proceed with creating canary resources for now
                                }
                            }
                        }

                        if is_canary_active {
                            // 2. Monitor Canary: we are in the middle of a rollout
                            resources::ensure_canary_deployment(client, node, ctx.enable_mtls, ctx.dry_run).await?;
                            resources::ensure_canary_service(client, node, ctx.enable_mtls, ctx.dry_run).await?;

                            let mut stable_node = node.clone();
                            // Recover the stable version from the existing deployment if possible
                            if let Some(cv) = &current_version {
                                stable_node.spec.version = cv.clone();
                            }
                            resources::ensure_deployment(client, &stable_node, ctx.enable_mtls, &propagated_labels, ctx.dry_run).await?;

                            // Check if the canary interval has elapsed
                            if let Some(status) = &node.status {
                                if let Some(start_time_str) = &status.canary_start_time {
                                    if let Ok(start_time) = chrono::DateTime::parse_from_rfc3339(start_time_str) {
                                        let now = chrono::Utc::now();
                                        let elapsed_secs = now.signed_duration_since(start_time).num_seconds();

                                        if elapsed_secs >= cfg.check_interval_seconds as i64 {
                                            // 3. Evaluate Canary: interval elapsed, check health
                                            info!(
                                                "Canary check interval elapsed ({} >= {}). Evaluating canary health.",
                                                elapsed_secs, cfg.check_interval_seconds
                                            );

                                            let canary_health = check_canary_health(client, node).await?;

                                            let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
                                            if canary_health.healthy {
                                                // 4a. Promote Canary
                                                info!("Canary {}/{} is healthy. Promoting to stable.", namespace, name);
                                                resources::ensure_deployment(client, node, ctx.enable_mtls, &propagated_labels, ctx.dry_run).await?;
                                                resources::delete_canary_resources(client, node, ctx.dry_run).await?;

                                                let patch = serde_json::json!({
                                                    "status": {
                                                        "canaryVersion": null,
                                                        "canaryStartTime": null,
                                                        "phase": "Running"
                                                    }
                                                });
                                                api.patch_status(
                                                    &name,
                                                    &PatchParams::apply("stellar-operator"),
                                                    &Patch::Merge(&patch),
                                                ).await?;
                                            } else {
                                                // 4b. Rollback Canary
                                                warn!("Canary {}/{} is unhealthy. Rolling back.", namespace, name);
                                                resources::delete_canary_resources(client, node, ctx.dry_run).await?;

                                                // Clean up canary status, emitting failure message
                                                let message = format!("Canary rollback triggered due to failed health check: {}", canary_health.message);
                                                let patch = serde_json::json!({
                                                    "status": {
                                                        "canaryVersion": null,
                                                        "canaryStartTime": null,
                                                        "phase": "Failed",
                                                        "message": message
                                                    }
                                                });
                                                api.patch_status(
                                                    &name,
                                                    &PatchParams::apply("stellar-operator"),
                                                    &Patch::Merge(&patch),
                                                ).await?;

                                                // Create a k8s event for the rollback
                                                let _ = remediation::emit_remediation_event(
                                                    client,
                                                    &ctx.event_reporter,
                                                    node,
                                                    remediation::RemediationLevel::Restart, // Not exactly a restart but conceptually similar action
                                                    &message,
                                                ).await;
                                            }
                                        } else {
                                            debug!(
                                                "Canary interval not yet elapsed: {} < {} seconds",
                                                elapsed_secs, cfg.check_interval_seconds
                                            );
                                        }
                                    }
                                }
                            }
                        } else {
                            // No canary active, regular deployment ensure
                            resources::ensure_deployment(client, node, ctx.enable_mtls, &propagated_labels, ctx.dry_run).await?;
                            resources::delete_canary_resources(client, node, ctx.dry_run).await?;
                        }
                    } else {
                        // RPC nodes use Deployment
                        resources::ensure_deployment(client, node, ctx.enable_mtls, &propagated_labels, ctx.dry_run).await?;
                        info!("Deployment ensured for RPC node {}/{}", namespace, name);

                        // Clean up canary resources if they exist
                        resources::delete_canary_resources(client, node, ctx.dry_run).await?;
                    }
                }
            }
            Ok(())
        },
    )
    .await;

    // 5.3: Handle workload failure — emit LabelPropagationFailed warning event and patch status
    match workload_result {
        Ok(()) => {}
        Err(workload_err) => {
            if let Err(e) = publish_stellar_event(
                client,
                &ctx.event_reporter,
                node,
                EventType::Warning,
                "LabelPropagationFailed",
                "LabelPropagation",
                &format!("Label propagation failed for workload: {workload_err}"),
            )
            .await
            {
                warn!("Failed to publish LabelPropagationFailed event: {}", e);
            }
            {
                let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
                let patch = serde_json::json!({
                    "status": {
                        "labelPropagationStatus": "Failed"
                    }
                });
                if let Err(e) = api
                    .patch_status(
                        &name,
                        &PatchParams::apply("stellar-operator"),
                        &Patch::Merge(&patch),
                    )
                    .await
                {
                    warn!("Failed to patch labelPropagationStatus to Failed: {}", e);
                }
            }
            return Err(workload_err);
        }
    }

    // Workload block succeeded — continue with observability

    if !ctx.dry_run {
        let workload_exists_after = workload_resource_exists(client, node).await.unwrap_or(true);
        if !workload_existed_before && workload_exists_after {
            let recorder = recorder_for(client, &ctx.event_reporter, node);
            if let Err(e) = publish_object_event(
                &recorder,
                EventType::Normal,
                "SuccessfulReconciliation",
                "Created",
                "Managed workload and related Kubernetes resources were created for this StellarNode.",
            )
            .await
            {
                warn!("Failed to publish SuccessfulReconciliation event: {e}");
            }
        }
    }

    // 5.2 / 5.4: Emit LabelsPropagated event and patch labelPropagationStatus to Synced
    if let Err(e) = publish_stellar_event(
        client,
        &ctx.event_reporter,
        node,
        EventType::Normal,
        "LabelsPropagated",
        "LabelPropagation",
        "Labels propagated to child resources",
    )
    .await
    {
        warn!("Failed to publish LabelsPropagated event: {}", e);
    }

    // 5.2: Emit LabelRemoved event when a generation change indicates labels may have been removed
    {
        let current_gen = node.metadata.generation.unwrap_or(0);
        let observed_gen = node
            .status
            .as_ref()
            .and_then(|s| s.observed_generation)
            .unwrap_or(0);
        if current_gen > observed_gen {
            if let Err(e) = publish_stellar_event(
                client,
                &ctx.event_reporter,
                node,
                EventType::Normal,
                "LabelRemoved",
                "LabelPropagation",
                "Stale labels removed from child resources",
            )
            .await
            {
                warn!("Failed to publish LabelRemoved event: {}", e);
            }
        }
    }

    {
        let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
        let patch = serde_json::json!({
            "status": {
                "labelPropagationStatus": "Synced"
            }
        });
        if let Err(e) = api
            .patch_status(
                &name,
                &PatchParams::apply("stellar-operator"),
                &Patch::Merge(&patch),
            )
            .await
        {
            warn!("Failed to patch labelPropagationStatus: {}", e);
        }
    }

    // 5a. MetalLB / LoadBalancer
    apply_or_emit(
        ctx,
        node,
        ActionType::Update,
        "MetalLB configuration",
        async {
            // TODO: Load balancer and global discovery fields not yet implemented in StellarNodeSpec
            // resources::ensure_metallb_config(client, node).await?;
            // resources::ensure_load_balancer_service(client, node).await?;
            Ok(())
        },
    )
    .await?;

    // 5b. Read-Only Replica Pools
    apply_or_emit(
        ctx,
        node,
        ActionType::Update,
        "Read-Only Replica Pool",
        async {
            crate::controller::read_pool::ensure_read_pool(client, node, ctx.enable_mtls).await?;
            crate::controller::traffic::reconcile_traffic_routing(client, node).await?;
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
                resources::ensure_hpa(client, node, ctx.dry_run).await?;
            }

            // VPA Integration
            match &node.spec.vpa_config {
                Some(vpa_cfg) => {
                    vpa_controller::ensure_vpa(client, node, vpa_cfg).await?;
                }
                None => {
                    // Clean up VPA if vpaConfig was removed from the spec
                    vpa_controller::delete_vpa(client, node).await?;
                }
            }

            resources::ensure_pdb(client, node, ctx.dry_run).await?;
            resources::ensure_alerting(client, node, ctx.dry_run).await?;
            resources::ensure_network_policy(client, node, ctx.dry_run).await?;
            Ok(())
        },
    )
    .await?;

    // 6a. CSI VolumeSnapshot schedule (Validator only)
    if node.spec.node_type == NodeType::Validator {
        if let Some(ref snapshot_config) = node.spec.snapshot_schedule {
            if let Err(e) = super::snapshot::reconcile_snapshot(client, node, snapshot_config).await
            {
                warn!(
                    "Snapshot reconciliation failed for {}/{}: {}",
                    namespace, name, e
                );
            }
        }
    }

    // 7. Perform health check to determine if node is ready
    //
    // Measure reduction in API polling overhead: Reactive Status check
    // If the DB trigger updated the status very recently (e.g. < 15 seconds ago), we can skip the health check API poll
    let mut skipped_poll = false;
    let mut recent_health = None;
    if let Some(ref status) = node.status {
        if let Some(updated_at_str) = &status.ledger_updated_at {
            if let Ok(updated_at) = chrono::DateTime::parse_from_rfc3339(updated_at_str) {
                let age = chrono::Utc::now()
                    .signed_duration_since(updated_at.with_timezone(&chrono::Utc))
                    .num_seconds();
                if age < 15 {
                    info!("Skipping health polling for {}/{}, DB trigger recently updated status {}s ago", namespace, name, age);
                    crate::controller::metrics::inc_api_polls_avoided(&namespace, &name);
                    skipped_poll = true;
                    // Assume node is healthy, use the reactively set ledger sequence
                    recent_health = Some(health::HealthCheckResult::synced(status.ledger_sequence));
                }
            }
        }
    }

    let health_result = if skipped_poll {
        recent_health.unwrap()
    } else {
        health::check_node_health(client, node, ctx.mtls_config.as_ref()).await?
    };

    debug!(
        "Health check result for {}/{}: healthy={}, synced={}, message={}",
        namespace, name, health_result.healthy, health_result.synced, health_result.message
    );

    // 7b. CVE scanning and automated patching
    if let Some(cve_config) = &node.spec.cve_handling {
        apply_or_emit(ctx, node, ActionType::Update, "CVE Handling", async {
            cve_reconciler::reconcile_cve_patches(client, node, cve_config).await?;
            Ok(())
        })
        .await?;
    }

    // 6. Trigger peer configuration reload for validators if healthy
    if node.spec.node_type == NodeType::Validator && health_result.healthy {
        if let Err(e) = peer_discovery::trigger_peer_config_reload(client, node).await {
            warn!(
                "Failed to trigger peer config reload for {}/{}: {}",
                namespace, name, e
            );
        }
    }

    // 6.5. Quorum analysis for validators
    if node.spec.node_type == NodeType::Validator && health_result.healthy {
        if let Err(e) = perform_quorum_analysis(client, node).await {
            warn!("Quorum analysis failed for {}/{}: {}", namespace, name, e);
            // Don't fail reconciliation on quorum analysis errors
        }
    }

    // 7. Trigger config-reload if VSL was updated and pod is ready
    if let Some(_quorum) = quorum_override {
        if health_result.healthy {
            // Get pod IP to trigger reload
            let pod_api: Api<k8s_openapi::api::core::v1::Pod> =
                Api::namespaced(client.clone(), &namespace);
            let lp = kube::api::ListParams::default()
                .labels(&format!("app.kubernetes.io/instance={name}"));
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
    let prev_dr_failover = node
        .status
        .as_ref()
        .and_then(|s| s.dr_status.as_ref())
        .map(|d| d.failover_active)
        .unwrap_or(false);
    if let Some(mut dr_status) = dr::reconcile_dr(client, node).await? {
        if dr_status.failover_active && !prev_dr_failover {
            let recorder = recorder_for(client, &ctx.event_reporter, node);
            if let Err(e) = publish_object_event(
                &recorder,
                EventType::Normal,
                "NodePromotedToPrimary",
                "Failover",
                "DR failover activated; this standby node is now primary.",
            )
            .await
            {
                warn!("Failed to publish NodePromotedToPrimary event: {e}");
            }
        }
        // 8a. Check if DR drill should be executed
        if let Some(drill_config) = &node
            .spec
            .dr_config
            .as_ref()
            .and_then(|c| c.drill_schedule.clone())
        {
            if dr_drill::should_run_drill(node, drill_config) {
                match dr_drill::execute_dr_drill(client, node, drill_config, &dr_status).await {
                    Ok(drill_result) => {
                        dr_status.last_drill_time = Some(Utc::now().to_rfc3339());
                        dr_status.last_drill_result = Some(drill_result);
                        info!("DR drill completed for {}", node.name_any());
                    }
                    Err(e) => {
                        warn!("DR drill failed for {}: {}", node.name_any(), e);
                    }
                }
            }
        }

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
                            &ctx.event_reporter,
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

    let prev_ready_reason = node.status.as_ref().and_then(|s| {
        conditions::find_condition(&s.conditions, conditions::CONDITION_TYPE_READY)
            .map(|c| c.reason.clone())
    });
    let sync_lag_begun = health_result.healthy
        && !health_result.synced
        && prev_ready_reason.as_deref() != Some("NodeSyncing");
    if sync_lag_begun {
        let recorder = recorder_for(client, &ctx.event_reporter, node);
        if let Err(e) = publish_object_event(
            &recorder,
            EventType::Warning,
            "SyncLagDetected",
            "Syncing",
            &health_result.message,
        )
        .await
        {
            warn!("Failed to publish SyncLagDetected event: {e}");
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
            let hardware_generation = hardware_generation_for_metrics(client, node).await;
            metrics::set_ledger_sequence(
                &namespace,
                &name,
                &node.spec.node_type.to_string(),
                node.spec.network_passphrase(),
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
                    node.spec.network_passphrase(),
                    lag.max(0),
                );
            }
        }
    }

    // 10b. Update node sync status metric
    #[cfg(feature = "metrics")]
    {
        let hardware_generation = hardware_generation_for_metrics(client, node).await;
        metrics::set_node_sync_status(
            &namespace,
            &name,
            &node.spec.node_type.to_string(),
            node.spec.network.passphrase(),
            &hardware_generation,
            phase,
        );
        
        // 10c. Update node up status based on pod readiness
        metrics::set_node_up(
            &namespace,
            &name,
            &node.spec.node_type.to_string(),
            node.spec.network.passphrase(),
            &hardware_generation,
            health_result.healthy,
        );
    }

    // 11. OCI snapshot push/pull Jobs
    if let Some(oci_cfg) = &node.spec.oci_snapshot {
        if oci_cfg.enabled {
            let ledger_seq = node
                .status
                .as_ref()
                .and_then(|s| s.ledger_sequence)
                .unwrap_or(0);

            // Push: trigger when node is healthy, synced, and we have a ledger number.
            if oci_cfg.push && health_result.healthy && health_result.synced && ledger_seq > 0 {
                if let Err(e) =
                    oci_snapshot::ensure_snapshot_push_job(client, node, oci_cfg, ledger_seq).await
                {
                    warn!(
                        "Failed to create OCI snapshot push Job for {}/{}: {}",
                        namespace, name, e
                    );
                    publish_stellar_event(
                        client,
                        &ctx.event_reporter,
                        node,
                        EventType::Warning,
                        "OciSnapshotPushFailed",
                        "Snapshot",
                        &format!("Could not create snapshot push Job: {e}"),
                    )
                    .await
                    .ok();
                }
            }

            // Pull: trigger on bootstrap when the node has never synced (ledger_seq == 0).
            // This extracts a prior snapshot so the node doesn't need a full catchup.
            if oci_cfg.pull && ledger_seq == 0 {
                if let Err(e) =
                    oci_snapshot::ensure_snapshot_pull_job(client, node, oci_cfg, 0).await
                {
                    warn!(
                        "Failed to create OCI snapshot pull Job for {}/{}: {}",
                        namespace, name, e
                    );
                    publish_stellar_event(
                        client,
                        &ctx.event_reporter,
                        node,
                        EventType::Warning,
                        "OciSnapshotPullFailed",
                        "Snapshot",
                        &format!("Could not create snapshot pull Job: {e}"),
                    )
                    .await
                    .ok();
                }
            }
        }
    }

    // 12. Service Mesh Configuration (Istio/Linkerd)
    if node.spec.service_mesh.is_some() {
        apply_or_emit(
            ctx,
            node,
            ActionType::Update,
            "Service Mesh (Istio/Linkerd)",
            async {
                service_mesh::ensure_peer_authentication(client, node).await?;
                service_mesh::ensure_destination_rule(client, node).await?;
                service_mesh::ensure_virtual_service(client, node).await?;
                service_mesh::ensure_request_authentication(client, node).await?;
                Ok(())
            },
        )
        .await?;
    }

    // Cost estimation: annotate and export metric (non-fatal).
    {
        let cost = super::cost::estimate_monthly_cost(node);
        if let Err(e) = super::cost::annotate_node_cost(client, node, cost).await {
            warn!(
                "Failed to annotate node cost for {}/{}: {:?}",
                namespace, name, e
            );
        }
        #[cfg(feature = "metrics")]
        super::cost::report_cost_metric(&namespace, &name, &node.spec.node_type.to_string(), cost);
    }

    // 13. Stamp audit annotations for the permanent reconcile trail.
    {
        use super::audit::actions;
        let action = match node.spec.node_type {
            crate::crd::NodeType::Validator => actions::UPDATED_STATEFULSET,
            crate::crd::NodeType::Horizon | crate::crd::NodeType::SorobanRpc => {
                actions::UPDATED_DEPLOYMENT
            }
        };
        super::audit::patch_audit_annotations(client, node, action).await;
    }

    // 14. Update status to Running with ready replica count
    // Use configured requeue interval for healthy reconciliation
    let requeue_interval = ctx.operator_config.reconciler.requeue_interval;
    Ok(Action::requeue(Duration::from_secs(if phase == "Ready" {
        requeue_interval
    } else {
        // Use shorter interval for non-ready phases
        requeue_interval / 4
    })))
}

/// Clean up resources when the StellarNode is deleted
#[instrument(skip(client, node, ctx), fields(name = %node.name_any(), namespace = node.namespace()))]
pub(crate) async fn cleanup_stellar_node(
    client: &Client,
    node: &StellarNode,
    ctx: &ControllerState,
) -> Result<Action> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!("Cleaning up StellarNode: {}/{}", namespace, name);

    let recorder = recorder_for(client, &ctx.event_reporter, node);
    if let Err(e) = publish_object_event(
        &recorder,
        EventType::Normal,
        "FinalizerCleanupStarted",
        "Finalize",
        "Finalizer cleanup started; removing managed Kubernetes resources for this StellarNode.",
    )
    .await
    {
        warn!("Failed to publish FinalizerCleanupStarted event: {e}");
    }

    // Delete resources in reverse order of creation

    // 0a. Delete Managed Database Resources
    apply_or_emit(ctx, node, ActionType::Delete, "Managed Database", async {
        if let Err(e) = resources::delete_cnpg_resources(client, node, ctx.dry_run).await {
            warn!("Failed to delete CNPG resources: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 0. Delete Alerting
    apply_or_emit(ctx, node, ActionType::Delete, "Alerting", async {
        if let Err(e) = resources::delete_alerting(client, node, ctx.dry_run).await {
            warn!("Failed to delete alerting: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 0b. Delete VPA (if vpaConfig was configured)
    apply_or_emit(ctx, node, ActionType::Delete, "VPA", async {
        if let Err(e) = vpa_controller::delete_vpa(client, node).await {
            warn!("Failed to delete VPA: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 1. Delete HPA (if autoscaling was configured)
    apply_or_emit(ctx, node, ActionType::Delete, "HPA", async {
        if let Err(e) = resources::delete_hpa(client, node, ctx.dry_run).await {
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
        if let Err(e) = resources::delete_ingress(client, node, ctx.dry_run).await {
            warn!("Failed to delete Ingress: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 3a. Delete NetworkPolicy
    apply_or_emit(ctx, node, ActionType::Delete, "NetworkPolicy", async {
        if let Err(e) = resources::delete_network_policy(client, node, ctx.dry_run).await {
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

    // 3c. Delete Service Mesh Resources (Istio/Linkerd)
    apply_or_emit(ctx, node, ActionType::Delete, "Service Mesh", async {
        if let Err(e) = service_mesh::delete_service_mesh_resources(client, node).await {
            warn!("Failed to delete service mesh resources: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 3d. Delete PDB
    apply_or_emit(ctx, node, ActionType::Delete, "PDB", async {
        if let Err(e) = resources::delete_pdb(client, node, ctx.dry_run).await {
            warn!("Failed to delete PodDisruptionBudget: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 4. Delete Service
    apply_or_emit(ctx, node, ActionType::Delete, "Service", async {
        if let Err(e) = resources::delete_service(client, node, ctx.dry_run).await {
            warn!("Failed to delete Service: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 5. Delete Deployment/StatefulSet
    apply_or_emit(ctx, node, ActionType::Delete, "Workload", async {
        if let Err(e) = resources::delete_workload(client, node, ctx.dry_run).await {
            warn!("Failed to delete workload: {:?}", e);
        }
        Ok(())
    })
    .await?;

    // 6. Delete ConfigMap
    apply_or_emit(ctx, node, ActionType::Delete, "ConfigMap", async {
        if let Err(e) = resources::delete_config_map(client, node, ctx.dry_run).await {
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
            if let Err(e) = resources::delete_pvc(client, node, ctx.dry_run).await {
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
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
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
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
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
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
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
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
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
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
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
pub(crate) fn apply_phase_conditions(
    conditions: &mut Vec<Condition>,
    phase: &str,
    message: Option<&str>,
) {
    match phase {
        "Ready" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_TRUE,
                "AllSubresourcesHealthy",
                message.unwrap_or("All sub-resources are healthy and operational"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_FALSE,
                "ReconcileComplete",
                "Reconciliation completed successfully",
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_FALSE,
                "NoIssues",
                "No degradation detected",
            );
        }
        "Creating" | "Pending" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Creating",
                message.unwrap_or("Resources are being created"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_TRUE,
                "Creating",
                message.unwrap_or("Creating resources"),
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        "Syncing" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Syncing",
                message.unwrap_or("Node is syncing with the network"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_TRUE,
                "Syncing",
                message.unwrap_or("Syncing data"),
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        "Running" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_TRUE,
                "ResourcesCreated",
                message.unwrap_or("Resources created successfully"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_FALSE,
                "Complete",
                "Resource creation complete",
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        "Degraded" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Degraded",
                message.unwrap_or("Node is experiencing issues"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_TRUE,
                "IssuesDetected",
                message.unwrap_or("Node is degraded"),
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_PROGRESSING);
        }
        "Failed" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Failed",
                message.unwrap_or("Node operation failed"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_TRUE,
                "Failed",
                message.unwrap_or("Operation failed"),
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_PROGRESSING);
        }
        "Remediating" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Remediating",
                message.unwrap_or("Auto-remediation in progress"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_PROGRESSING,
                conditions::CONDITION_STATUS_TRUE,
                "Remediating",
                message.unwrap_or("Remediation in progress"),
            );
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_DEGRADED,
                conditions::CONDITION_STATUS_TRUE,
                "Remediating",
                "Node required remediation",
            );
        }
        "Suspended" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Suspended",
                message.unwrap_or("Node is suspended"),
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_PROGRESSING);
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        "Maintenance" => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_FALSE,
                "Maintenance",
                message.unwrap_or("Node is in maintenance mode"),
            );
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_PROGRESSING);
            conditions::remove_condition(conditions, conditions::CONDITION_TYPE_DEGRADED);
        }
        _ => {
            conditions::set_condition(
                conditions,
                conditions::CONDITION_TYPE_READY,
                conditions::CONDITION_STATUS_UNKNOWN,
                "Unknown",
                message.unwrap_or("Status unknown"),
            );
        }
    }
}

#[allow(deprecated)]
#[instrument(skip(client, node, message), fields(name = %node.name_any(), namespace = node.namespace(), phase))]
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

    apply_phase_conditions(&mut conditions, phase, message);

    // Set observed generation on all conditions
    if let Some(gen) = observed_generation {
        for condition in &mut conditions {
            condition.observed_generation = Some(gen);
        }
    }

    let read_pool_endpoint = if node.spec.read_replica_config.is_some() {
        Some(crate::controller::read_pool::read_pool_endpoint(node))
    } else {
        None
    };

    let mut status_patch = serde_json::json!({
        "phase": phase,
        "observedGeneration": observed_generation,
        "replicas": if node.spec.suspended { 0 } else { node.spec.replicas },
        "readyReplicas": ready_replicas,
        "conditions": conditions,
        "readPoolEndpoint": read_pool_endpoint,
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
/// Run the hourly archive integrity check for a validator node.
///
/// Fetches `stellar-history.json` from each configured archive, compares the reported
/// ledger sequence to the node's current ledger, and:
/// - Sets / clears the `ArchiveIntegrityDegraded` condition on the node's status.
/// - Updates the `stellar_archive_ledger_lag` Prometheus gauge so alert rules can fire.
///
/// The function is intentionally fire-and-forget on individual per-URL errors so that a
/// single unreachable archive does not block the rest of reconciliation.
#[instrument(skip(client, node, archive_urls), fields(name = %node.name_any(), namespace = node.namespace()))]
async fn run_archive_integrity_check(
    client: &Client,
    reporter: &Reporter,
    node: &StellarNode,
    archive_urls: &[String],
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let node_ledger = node
        .status
        .as_ref()
        .and_then(|s| s.ledger_sequence)
        .unwrap_or(0);

    // If the node has not yet reported a ledger we can't compute meaningful lag values.
    // Skip until a ledger becomes available.
    if node_ledger == 0 {
        debug!(
            "Skipping archive integrity check for {}/{}: node ledger not yet available",
            namespace, name
        );
        return Ok(());
    }

    info!(
        "Running periodic archive integrity check for {}/{} (node_ledger={})",
        namespace, name, node_ledger
    );

    let results = check_archive_integrity(archive_urls, node_ledger, None).await;

    // Determine the overall worst-case lag across all archives.
    let degraded_archives: Vec<_> = results.iter().filter(|r| !r.is_healthy()).collect();
    let any_degraded = !degraded_archives.is_empty();
    let max_lag = results.iter().filter_map(|r| r.lag).max().unwrap_or(0);

    // Update Prometheus metric with the maximum observed lag.
    #[cfg(feature = "metrics")]
    let hardware_generation = hardware_generation_for_metrics(client, node).await;
    #[cfg(feature = "metrics")]
    metrics::set_archive_ledger_lag(
        &namespace,
        &name,
        &node.spec.node_type.to_string(),
        node.spec.network_passphrase(),
        max_lag as i64,
    );

    // Patch the Degraded condition on the node status.
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let mut conds = node
        .status
        .as_ref()
        .map(|s| s.conditions.clone())
        .unwrap_or_default();

    if any_degraded {
        let messages: Vec<String> = degraded_archives.iter().map(|r| r.summary()).collect();
        let message = messages.join("; ");
        warn!(
            "Archive integrity degraded for {}/{}: {}",
            namespace, name, message
        );
        publish_stellar_event(
            client,
            reporter,
            node,
            EventType::Warning,
            "ArchiveIntegrityDegraded",
            "ArchiveIntegrity",
            &format!("History archive(s) are lagging (max lag={max_lag}): {message}"),
        )
        .await?;
        conditions::set_condition(
            &mut conds,
            "ArchiveIntegrityDegraded",
            conditions::CONDITION_STATUS_TRUE,
            "ArchiveLagging",
            &format!(
                "Archive lag exceeds threshold of {ARCHIVE_LAG_THRESHOLD} ledgers. Max lag={max_lag}. {message}"
            ),
        );
    } else {
        // All archives healthy: clear (or keep cleared) the Degraded sub-condition.
        conditions::set_condition(
            &mut conds,
            "ArchiveIntegrityDegraded",
            conditions::CONDITION_STATUS_FALSE,
            "ArchiveInSync",
            &format!(
                "All {} archive(s) are within {} ledgers of the node",
                results.len(),
                ARCHIVE_LAG_THRESHOLD
            ),
        );
    }

    let patch = serde_json::json!({ "status": { "conditions": conds } });
    api.patch_status(
        &name,
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

#[instrument(skip(client, node, result), fields(name = %node.name_any(), namespace = node.namespace()))]
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
#[instrument(skip(client, node, message, health), fields(name = %node.name_any(), namespace = node.namespace()))]
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

/// Run the archive checkpoint verification check
async fn run_archive_checkpoint_verification(
    client: &Client,
    reporter: &EventReporter,
    node: &StellarNode,
    urls: &[String],
    config: &crate::crd::ArchiveIntegrityConfig,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    info!(
        "Running archive checkpoint integrity check for {}/{}",
        namespace, name
    );

    let mut results = Vec::new();
    for url in urls {
        match check_archive_integrity_random(
            url,
            config.check_percentage,
            config.max_checkpoints,
            Duration::from_secs(30),
        )
        .await
        {
            Ok(res) => results.push(res),
            Err(e) => {
                warn!("Archive integrity check failed for {}: {}", url, e);
                results.push(ArchiveIntegrityCheckResult {
                    url: url.clone(),
                    healthy: false,
                    checkpoints_verified: 0,
                    message: format!("Check failed: {e}"),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    let all_healthy = results.iter().all(|r| r.healthy);
    let summary = if all_healthy {
        format!(
            "Archive integrity verified: {} archives healthy",
            results.len()
        )
    } else {
        let failed = results.iter().filter(|r| !r.healthy).count();
        format!("Archive integrity corruption detected: {}/{} archives corrupted", failed, results.len())
    };

    // Update metrics
    let node_type = format!("{:?}", node.spec.node_type);
    let network = format!("{:?}", node.spec.network);
    let hardware = node.spec.hardware_generation.clone();
    metrics::set_archive_integrity_status(&namespace, &name, &node_type, &network, &hardware, all_healthy);

    // Update status conditions
    let mut status = node.status.clone().unwrap_or_default();
    if all_healthy {
        conditions::set_condition(
            &mut status.conditions,
            "ArchiveIntegrityCheck",
            conditions::CONDITION_STATUS_TRUE,
            "IntegrityVerified",
            &summary,
        );
        conditions::remove_condition(&mut status.conditions, "ArchiveIntegrityCorrupted");
    } else {
        conditions::set_condition(
            &mut status.conditions,
            "ArchiveIntegrityCheck",
            conditions::CONDITION_STATUS_FALSE,
            "IntegrityCheckFailed",
            &summary,
        );
        conditions::set_condition(
            &mut status.conditions,
            "ArchiveIntegrityCorrupted",
            conditions::CONDITION_STATUS_TRUE,
            "CorruptionDetected",
            &summary,
        );

        // Emit Fatal Event for corruption
        publish_stellar_event(
            client,
            reporter,
            node,
            EventType::Warning,
            "ArchiveIntegrityCorruption",
            "ArchiveIntegrity",
            &format!("FATAL: Corruption detected in history archives!\n\nDetails:\n{}", 
                results.iter()
                    .filter(|r| !r.healthy)
                    .map(|r| format!("- {}: {}", r.url, r.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        )
        .await?;
    }

    // Set observed generation
    if let Some(gen) = node.metadata.generation {
        for condition in &mut status.conditions {
            if condition.type_ == "ArchiveIntegrityCheck" || condition.type_ == "ArchiveIntegrityCorrupted" {
                condition.observed_generation = Some(gen);
            }
        }
    }

    let patch = serde_json::json!({ "status": status });
    api.patch_status(
        &name,
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

/// Helper to parse duration string (e.g. "1h", "6h", "24h")
fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.ends_with('h') {
        let hours = s[..s.len() - 1].parse::<u64>().map_err(|_| Error::ConfigError(format!("Invalid duration: {s}")))?;
        Ok(Duration::from_secs(hours * 3600))
    } else if s.ends_with('m') {
        let mins = s[..s.len() - 1].parse::<u64>().map_err(|_| Error::ConfigError(format!("Invalid duration: {s}")))?;
        Ok(Duration::from_secs(mins * 60))
    } else if s.ends_with('s') {
        let secs = s[..s.len() - 1].parse::<u64>().map_err(|_| Error::ConfigError(format!("Invalid duration: {s}")))?;
        Ok(Duration::from_secs(secs))
    } else {
        Err(Error::ConfigError(format!("Unsupported duration format: {s}")))
    }
}

/// Helper to get the latest ledger from the Stellar network
async fn get_latest_network_ledger(network: &crate::crd::StellarNetwork) -> Result<u64> {
    let url = match network {
        crate::crd::StellarNetwork::Mainnet => "https://horizon.stellar.org",
        crate::crd::StellarNetwork::Testnet => "https://horizon-testnet.stellar.org",
        crate::crd::StellarNetwork::Futurenet => "https://horizon-futurenet.stellar.org",
        crate::crd::StellarNetwork::Custom => {
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
#[instrument(skip(client, node, dr_status), fields(name = %node.name_any(), namespace = node.namespace()))]
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
pub(crate) fn error_policy(
    node: Arc<StellarNode>,
    error: &Error,
    ctx: Arc<ControllerState>,
) -> Action {
    let node_name = node.name_any();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let reconcile_id = ctx.next_reconcile_id();

    let node_name_for_span = node_name.clone();
    let namespace_for_span = namespace.clone();
    let resource_version = node
        .metadata
        .resource_version
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let _error_span = info_span!(
        "reconcile_error",
        node_name = %node_name_for_span,
        namespace = %namespace_for_span,
        reconcile_id = %reconcile_id,
        resource_version = %resource_version
    );
    let _enter = _error_span.enter();

    error!("Reconciliation error for {}: {:?}", node_name, error);

    // Get retry count from annotations (default to 0)
    let retry_count = node
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("stellar.org/error-retry-count"))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    // Calculate backoff based on error type and retry count
    let retry_duration = if error.is_retriable() {
        // Use exponential backoff for retriable errors
        ctx.operator_config
            .reconciler
            .calculate_backoff(retry_count)
    } else {
        // Use fixed interval for non-retriable errors
        Duration::from_secs(ctx.operator_config.reconciler.requeue_interval)
    };

    debug!(
        "Requeuing {} after {:?} (retry_count: {}, retriable: {})",
        node.name_any(),
        retry_duration,
        retry_count,
        error.is_retriable()
    );

    Action::requeue(retry_duration)
}

/// Perform quorum analysis for validator nodes
#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
async fn perform_quorum_analysis(client: &Client, node: &StellarNode) -> Result<()> {
    use super::quorum::QuorumAnalyzer;

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    // Get pod IPs for all validator pods
    let pod_api: Api<k8s_openapi::api::core::v1::Pod> = Api::namespaced(client.clone(), &namespace);
    let lp = kube::api::ListParams::default().labels(&format!("app.kubernetes.io/instance={name}"));

    let pods = pod_api.list(&lp).await.map_err(Error::KubeError)?;
    let pod_ips: Vec<String> = pods
        .items
        .iter()
        .filter_map(|pod| pod.status.as_ref()?.pod_ip.clone())
        .collect();

    if pod_ips.is_empty() {
        debug!(
            "No pod IPs found for quorum analysis of {}/{}",
            namespace, name
        );
        return Ok(());
    }

    // Create analyzer and run analysis with timeout
    let mut analyzer = QuorumAnalyzer::new(Duration::from_secs(10), 100);

    let analysis_future = analyzer.analyze_quorum(pod_ips);
    let result = tokio::time::timeout(Duration::from_secs(30), analysis_future)
        .await
        .map_err(|_| Error::ConfigError("Quorum analysis timeout".to_string()))?
        .map_err(|e| Error::ConfigError(format!("Quorum analysis failed: {e}")))?;

    // Update metrics
    #[cfg(feature = "metrics")]
    {
        let node_type = node.spec.node_type.to_string();
        let hardware_generation = hardware_generation_for_metrics(client, node).await;
        let network = match &node.spec.network {
            crate::crd::StellarNetwork::Mainnet => "mainnet",
            crate::crd::StellarNetwork::Testnet => "testnet",
            crate::crd::StellarNetwork::Futurenet => "futurenet",
            crate::crd::StellarNetwork::Custom => "custom",
        };

        metrics::set_quorum_critical_nodes(
            &namespace,
            &name,
            &node_type,
            network,
            &hardware_generation,
            result.critical_nodes.len() as i64,
        );
        metrics::set_quorum_min_overlap(
            &namespace,
            &name,
            &node_type,
            network,
            &hardware_generation,
            result.min_overlap as i64,
        );
        metrics::set_quorum_fragility_score(
            &namespace,
            &name,
            &node_type,
            network,
            &hardware_generation,
            result.fragility_score,
        );
    }

    // Update status
    analyzer
        .update_node_status(client, node, &result)
        .await
        .map_err(|e| Error::ConfigError(format!("Failed to update status: {e}")))?;

    info!(
        "Quorum analysis complete for {}/{}: fragility={:.3}, critical_nodes={}, min_overlap={}",
        namespace,
        name,
        result.fragility_score,
        result.critical_nodes.len(),
        result.min_overlap
    );

    Ok(())
}

#[cfg(feature = "metrics")]
async fn hardware_generation_for_metrics(client: &Client, node: &StellarNode) -> String {
    match infra::resolve_stellar_node_infra(client, node).await {
        Ok(summary) => summary.hardware_generation_label(),
        Err(err) => {
            warn!(
                "Failed to resolve hardware generation for metrics on {}/{}: {:?}",
                node.namespace().unwrap_or_else(|| "default".to_string()),
                node.name_any(),
                err
            );
            "unknown".to_string()
        }
    }
}
