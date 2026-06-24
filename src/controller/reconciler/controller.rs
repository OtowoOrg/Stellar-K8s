//! Controller loop entry point.

use super::prelude::*;
use super::error_policy::error_policy;
use super::reconcile::reconcile;
use super::state::ControllerState;

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
///         retry_budget_retriable_secs: 15,
///         retry_budget_nonretriable_secs: 60,
///         retry_budget_max_attempts: 3,
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
///         last_event_received: Arc::new(AtomicU64::new(0)),
///         job_registry: Arc::new(stellar_k8s::controller::background_jobs::JobRegistry::new()),
///         audit_log: Arc::new(stellar_k8s::controller::audit_log::AuditLog::new()),
///         audit_recorder: Arc::new(stellar_k8s::controller::AuditRecorder::new(
///             Arc::new(stellar_k8s::controller::audit_log::AuditLog::new()),
///             vec![],
///             None,
///         )),
///         anomaly_detector: Arc::new(stellar_k8s::controller::AnomalyDetector::new(
///             Default::default(),
///         )),
///         plugin_registry: Arc::new(stellar_k8s::plugin_sdk::PluginRegistry::new()),
///         oidc_config: None,
///         metrics_store: Arc::new(stellar_k8s::rest_api::metrics_store::StellarMetricsStore::new()),
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
            format!("namespace-scoped: {ns}")
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

    // Start Node Drain Orchestrator in the background
    let drain_orchestrator = Arc::new(maintenance::NodeDrainOrchestrator::new(
        client.clone(),
        state.event_reporter.clone(),
    ));
    tokio::spawn(async move {
        if let Err(e) = drain_orchestrator.run().await {
            error!("Node Drain Orchestrator stopped with error: {}", e);
        }
    });

    // Start Spot/Preemptible Drain Handler in the background.
    // NODE_NAME must be injected via the Downward API (spec.nodeName).
    if let Ok(node_name) = std::env::var("NODE_NAME") {
        let spot_handler = Arc::new(spot_drain::SpotDrainHandler::new(
            client.clone(),
            state.event_reporter.clone(),
            node_name,
        ));
        tokio::spawn(async move {
            if let Err(e) = spot_handler.run().await {
                error!("Spot Drain Handler stopped with error: {}", e);
            }
        });
    } else {
        info!("NODE_NAME env var not set – Spot Drain Handler disabled");
    }
    // Start Horizon Metrics Collector in the background
    #[cfg(feature = "rest-api")]
    {
        use super::horizon_metrics_collector::spawn_horizon_metrics_collector;
        let collector_client = client.clone();
        let collector_store = state.metrics_store.clone();
        let collector_watch_ns = state.watch_namespace.clone();
        tokio::spawn(async move {
            let _handle = spawn_horizon_metrics_collector(
                collector_store,
                30, // poll every 30 seconds
                collector_client,
                collector_watch_ns,
            );
            if let Err(e) = _handle.await {
                error!("Horizon Metrics Collector stopped with error: {:?}", e);
            }
        });
    }

    // Start Quorum Optimizer in the background
    let quorum_optimizer = Arc::new(super::quorum::QuorumOptimizer::new(
        client.clone(),
        state.event_reporter.clone(),
    ));
    tokio::spawn(async move {
        if let Err(e) = quorum_optimizer.run().await {
            error!("Quorum Optimizer stopped with error: {}", e);
        }
    });

    // Start Audit Worker if enabled
    if state.operator_config.audit.enabled {
        let audit_worker = AuditWorker::new(client.clone(), state.audit_recorder.clone());
        tokio::spawn(async move {
            if let Err(e) = audit_worker.run().await {
                error!("Audit Worker stopped with error: {}", e);
            }
        });
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
        .watches::<k8s_openapi::api::core::v1::Secret, _>(
            if let Some(ns) = &state.watch_namespace {
                Api::namespaced(client.clone(), ns)
            } else {
                Api::all(client.clone())
            },
            Config::default(),
            |secret| {
                // Trigger reconciliation for all StellarNodes that reference this secret
                // The reconciler will check if the secret version changed and trigger restarts
                vec![]
            },
        )
        .shutdown_on_signal()
        .run(|obj, ctx| reconcile(obj, ctx), error_policy, state.clone())
        .fold(BatchSummaryReport::new(50), {
            let state = state.clone();
            move |mut report, res| {
                let state = state.clone();
                async move {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    state
                        .last_event_received
                        .store(now, std::sync::atomic::Ordering::Relaxed);

                    match res {
                        Ok(obj) => {
                            let name = format!("{:?}", obj);
                            info!("Reconciled: {:?}", obj);
                            report.record_success(name);
                        }
                        Err(e) => {
                            let err_str = format!("{:?}", e);
                            error!("Reconcile error: {:?}", e);
                            report.record_failure("unknown".to_string(), err_str);
                        }
                    }
                    report
                }
            }
        })
        .await
        .emit_final_summary();

    Ok(())
}
