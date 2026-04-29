use chrono::Utc;
use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, ObjectMeta, Patch, PatchParams, PostParams};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{info, info_span, warn, Instrument, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::cli::RunArgs;
#[cfg(feature = "rest-api")]
use stellar_k8s::rest_api::metrics_store::StellarMetricsStore;
use stellar_k8s::log_scrub::ScrubLayer;
use stellar_k8s::{controller, preflight, Error};

const LEASE_NAME: &str = "stellar-operator-leader";
const LEASE_DURATION_SECS: i32 = 15;
const RENEW_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

pub async fn run_operator(args: RunArgs) -> Result<(), Error> {
    // Handle --dump-config: print resolved configuration and exit.
    if args.dump_config {
        let operator_config = controller::OperatorConfig::load();
        let resolved = serde_json::json!({
            "cli": {
                "namespace": args.namespace,
                "watch_namespace": args.watch_namespace,
                "enable_mtls": args.enable_mtls,
                "dry_run": args.dry_run,
                "scheduler": args.scheduler,
                "scheduler_name": args.scheduler_name,
                "retry_budget_retriable_secs": args.retry_budget_retriable_secs,
                "retry_budget_nonretriable_secs": args.retry_budget_nonretriable_secs,
                "retry_budget_max_attempts": args.retry_budget_max_attempts,
            },
            "operator_config": operator_config,
        });
        println!(
            "{}",
            serde_yaml::to_string(&resolved)
                .unwrap_or_else(|_| serde_json::to_string_pretty(&resolved).unwrap())
        );
        return Ok(());
    }

    // Initialize tracing with OpenTelemetry
    let env_filter = EnvFilter::builder()
        .with_default_directive(Level::INFO.into())
        .from_env_lossy();

    let (env_filter, reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);

    let fmt_layer = fmt::layer().json().with_target(true);

    // Register the subscriber with both stdout logging and OpenTelemetry tracing
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(ScrubLayer::new())
        .with(fmt_layer);

    // Only enable OTEL if an endpoint is provided or via a flag
    let otel_enabled = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok();

    if otel_enabled {
        let otel_layer = stellar_k8s::telemetry::init_telemetry(&registry);
        let trace_id_layer = stellar_k8s::telemetry::trace_id_layer();
        registry.with(otel_layer).with(trace_id_layer).init();
    } else {
        registry.init();
    }

    let root_span = info_span!(
        "operator",
        node_name = "-",
        namespace = %args.namespace,
        reconcile_id = "-"
    );
    let _root_enter = root_span.enter();

    if otel_enabled {
        info!("OpenTelemetry tracing initialized");
    } else {
        info!("OpenTelemetry tracing disabled (OTEL_EXPORTER_OTLP_ENDPOINT not set)");
    }

    info!(
        "Starting Stellar-K8s Operator v{}",
        env!("CARGO_PKG_VERSION")
    );

    // Fast-fail preflight for GitHub automation dependencies when explicitly configured.
    let github_repo = args
        .github_repo
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    if let Some(repo) = github_repo {
        preflight::run_gh_label_preflight(Some(repo))?;
    } else {
        info!("Skipping GitHub preflight (GITHUB_REPOSITORY not set)");
    }

    // Initialise operator build-info metric
    #[cfg(feature = "metrics")]
    {
        controller::metrics::init_operator_info();
    }

    // Initialize Kubernetes client
    let client = kube::Client::try_default()
        .await
        .map_err(Error::KubeError)?;

    info!("Connected to Kubernetes cluster");

    // Run preflight self-checks
    let preflight_results = preflight::run_preflight_checks(&client, &args.namespace).await;
    preflight::print_diagnostic_summary(&preflight_results);

    if args.preflight_only {
        info!("--preflight-only flag set; exiting after diagnostics.");
        return preflight::evaluate_results(&preflight_results);
    }

    preflight::evaluate_results(&preflight_results)?;

    // If --scheduler flag is set, run the latency-aware scheduler instead
    if args.scheduler {
        info!(
            "Running in scheduler mode with name: {}",
            args.scheduler_name
        );
        let scheduler = stellar_k8s::scheduler::core::Scheduler::new(client, args.scheduler_name);
        return scheduler
            .run()
            .await
            .map_err(|e| Error::ConfigError(e.to_string()));
    }

    let client_clone = client.clone();
    let namespace = args.namespace.clone();

    let mtls_config = if args.enable_mtls {
        info!("Initializing mTLS for Operator...");

        controller::mtls::ensure_ca(&client_clone, &namespace).await?;
        controller::mtls::ensure_server_cert(
            &client_clone,
            &namespace,
            vec![
                "stellar-operator".to_string(),
                format!("stellar-operator.{}", namespace),
            ],
        )
        .await?;

        let secrets: Api<k8s_openapi::api::core::v1::Secret> =
            Api::namespaced(client_clone.clone(), &namespace);
        let secret = secrets
            .get(controller::mtls::SERVER_CERT_SECRET_NAME)
            .await
            .map_err(Error::KubeError)?;
        Some(controller::mtls::load_mtls_config_from_secret(&secret)?)
    } else {
        None
    };

    // Leader election configuration
    let leader_namespace =
        std::env::var("POD_NAMESPACE").unwrap_or_else(|_| args.namespace.clone());
    let holder_identity = std::env::var("HOSTNAME").unwrap_or_else(|_| {
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown-host".to_string())
    });

    info!("Leader election using holder ID: {}", holder_identity);

    let is_leader = Arc::new(AtomicBool::new(false));

    {
        let lease_client = client.clone();
        let lease_ns = leader_namespace.clone();
        let identity = holder_identity.clone();
        let is_leader_bg = Arc::clone(&is_leader);

        tokio::spawn(
            async move {
                run_leader_election(lease_client, &lease_ns, &identity, is_leader_bg).await;
            }
            .instrument(root_span.clone()),
        );
    }

    // Update leader-status and uptime metrics every 10 s
    #[cfg(feature = "metrics")]
    {
        let is_leader_metrics = Arc::clone(&is_leader);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                let leader = is_leader_metrics.load(Ordering::Relaxed);
                controller::metrics::set_leader_status(leader);
                controller::metrics::inc_uptime_seconds(10);
            }
        });
    }

    // Create shared controller state
    let operator_config = controller::OperatorConfig::load();
    #[cfg(feature = "rest-api")]
    let oidc_config = operator_config.oidc.clone();
    let audit_log = Arc::new(controller::AuditLog::new());
    let audit_sink: Option<Arc<dyn controller::audit_sink::AuditSink>> = if
        operator_config.audit.enabled
    {
        if let Some(s3_config) = &operator_config.audit.s3 {
            Some(
                Arc::new(controller::audit_sink::S3AuditSink::new(s3_config.clone()).await)
                    as Arc<dyn controller::audit_sink::AuditSink>,
            )
        } else {
            Some(Arc::new(controller::audit_sink::NoopAuditSink)
                as Arc<dyn controller::audit_sink::AuditSink>)
        }
    } else {
        None
    };
    let audit_recorder = Arc::new(controller::AuditRecorder::new(
        audit_log.clone(),
        audit_sink.clone(),
    ));
    let anomaly_detector = Arc::new(controller::AnomalyDetector::new(
        operator_config.anomaly_detection.clone(),
    ));

    let state = Arc::new(controller::ControllerState {
        client: client.clone(),
        enable_mtls: args.enable_mtls,
        operator_namespace: args.namespace.clone(),
        watch_namespace: args.watch_namespace.clone(),
        mtls_config: mtls_config.clone(),
        dry_run: args.dry_run,
        retry_budget_retriable_secs: args.retry_budget_retriable_secs,
        retry_budget_nonretriable_secs: args.retry_budget_nonretriable_secs,
        retry_budget_max_attempts: args.retry_budget_max_attempts,
        is_leader: Arc::clone(&is_leader),
        event_reporter: kube::runtime::events::Reporter {
            controller: "stellar-operator".to_string(),
            instance: None,
        },
        operator_config: Arc::new(operator_config),
        reconcile_id_counter: std::sync::atomic::AtomicU64::new(0),
        last_reconcile_success: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        log_reload_handle: reload_handle,
        log_level_expires_at: Arc::new(tokio::sync::Mutex::new(None)),
        last_event_received: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        job_registry: Arc::new(controller::JobRegistry::new()),
        audit_log: audit_log.clone(),
        audit_recorder: audit_recorder.clone(),
        anomaly_detector: anomaly_detector.clone(),
        audit_log: Arc::new(controller::AuditLog::new()),
        plugin_registry: Arc::new(stellar_k8s::plugin_sdk::PluginRegistry::new()),
        #[cfg(feature = "rest-api")]
        oidc_config,
        #[cfg(feature = "rest-api")]
        metrics_store: Arc::new(StellarMetricsStore::new()),
    });

    // Start the peer discovery manager
    let peer_discovery_client = client.clone();
    let peer_discovery_config = controller::PeerDiscoveryConfig::default();
    tokio::spawn(
        async move {
            let manager =
                controller::PeerDiscoveryManager::new(peer_discovery_client, peer_discovery_config);
            if let Err(e) = manager.run().await {
                tracing::error!("Peer discovery manager error: {:?}", e);
            }
        }
        .instrument(root_span.clone()),
    );

    // Start the feature-flag watcher
    let feature_flags = controller::feature_flags::new_shared();
    {
        let ff_client = client.clone();
        let ff_namespace = args.namespace.clone();
        let ff_flags = feature_flags.clone();
        let ff_audit_recorder = if state.operator_config.audit.enabled {
            Some(audit_recorder.clone())
        } else {
            None
        };

        tokio::spawn(async move {
            controller::watch_feature_flags(
                ff_client,
                ff_namespace,
                ff_flags,
                ff_audit_recorder,
            )
            .await;
        });
    }

    // Start anomaly detection loop
    {
        let detector_state = state.clone();
        let detector = anomaly_detector.clone();
        tokio::spawn(async move {
            controller::run_anomaly_detection(detector_state, detector).await;
        });
    }

    // Start the REST API server and optional mTLS certificate rotation
    #[cfg(feature = "rest-api")]
    {
        let api_state = state.clone();
        let rustls_config = mtls_config
            .as_ref()
            .and_then(|cfg| {
                stellar_k8s::rest_api::build_tls_server_config(
                    &cfg.cert_pem,
                    &cfg.key_pem,
                    &cfg.ca_pem,
                )
                .ok()
            })
            .map(axum_server::tls_rustls::RustlsConfig::from_config);
        let server_tls = rustls_config.clone();

        tokio::spawn(
            async move {
                if let Err(e) = stellar_k8s::rest_api::run_server(api_state, server_tls).await {
                    tracing::error!("REST API server error: {:?}", e);
                }
            }
            .instrument(root_span.clone()),
        );

        if let (true, Some(rustls_config)) = (args.enable_mtls, rustls_config) {
            let rotation_client = client.clone();
            let rotation_namespace = args.namespace.clone();
            let rotation_dns = vec![
                "stellar-operator".to_string(),
                format!("stellar-operator.{}", args.namespace),
            ];
            let rotation_threshold_days = std::env::var("CERT_ROTATION_THRESHOLD_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(controller::mtls::DEFAULT_CERT_ROTATION_THRESHOLD_DAYS);
            let is_leader_rot = Arc::clone(&is_leader);

            tokio::spawn(
                async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
                    loop {
                        interval.tick().await;
                        if !is_leader_rot.load(Ordering::Relaxed) {
                            continue;
                        }
                        match controller::mtls::maybe_rotate_server_cert(
                            &rotation_client,
                            &rotation_namespace,
                            rotation_dns.clone(),
                            rotation_threshold_days,
                        )
                        .await
                        {
                            Ok(true) => {
                                let secrets: Api<k8s_openapi::api::core::v1::Secret> =
                                    Api::namespaced(rotation_client.clone(), &rotation_namespace);
                                if let Ok(secret) =
                                    secrets.get(controller::mtls::SERVER_CERT_SECRET_NAME).await
                                {
                                    if let (Some(cert), Some(key), Some(ca)) = (
                                        secret.data.as_ref().and_then(|d| d.get("tls.crt")),
                                        secret.data.as_ref().and_then(|d| d.get("tls.key")),
                                        secret.data.as_ref().and_then(|d| d.get("ca.crt")),
                                    ) {
                                        match stellar_k8s::rest_api::build_tls_server_config(
                                            &cert.0, &key.0, &ca.0,
                                        ) {
                                            Ok(new_config) => {
                                                rustls_config.reload_from_config(new_config);
                                                info!("TLS server config reloaded");
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    "Failed to build TLS config: {:?}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(false) => {}
                            Err(e) => {
                                tracing::error!("Rotation check failed: {:?}", e);
                            }
                        }
                    }
                }
                .instrument(root_span.clone()),
            );
        }
    }

    let shutdown_state = state.clone();
    let shutdown_client = client.clone();
    let shutdown_namespace = args.namespace.clone();
    let shutdown_is_leader = Arc::clone(&is_leader);
    let shutdown_identity = holder_identity.clone();

    {
        let bench_client = client.clone();
        let bench_is_leader = Arc::clone(&is_leader);
        tokio::spawn(async move {
            if let Err(e) =
                controller::run_benchmark_controller(bench_client, bench_is_leader).await
            {
                tracing::error!("Benchmark controller error: {:?}", e);
            }
        });
    }

    {
        let snapshot_client = client.clone();
        let snapshot_reporter = kube::runtime::events::Reporter {
            controller: "stellar-operator-snapshot-worker".to_string(),
            instance: None,
        };
        tokio::spawn(async move {
            controller::run_snapshot_worker(snapshot_client, snapshot_reporter).await;
        });
        info!("Auto-snapshot worker spawned");
    }

    // Start the snapshot integrity checker background worker
    {
        use stellar_k8s::controller::snapshot_integrity::{
            SnapshotIntegrityChecker, SnapshotIntegrityConfig,
        };

        let integrity_client = client.clone();
        let integrity_config = std::env::var("SNAPSHOT_INTEGRITY_CONFIG")
            .ok()
            .and_then(|s| serde_json::from_str::<SnapshotIntegrityConfig>(&s).ok())
            .unwrap_or_default();

        if integrity_config.enabled {
            info!(
                "Starting snapshot integrity checker (schedule: {})",
                integrity_config.schedule
            );
            tokio::spawn(
                async move {
                    let checker = SnapshotIntegrityChecker::new(integrity_config, integrity_client);
                    if let Err(e) = checker.start().await {
                        tracing::error!("Snapshot integrity checker error: {:?}", e);
                    }
                }
                .instrument(root_span.clone()),
            );
        } else {
            info!("Snapshot integrity checker disabled (set SNAPSHOT_INTEGRITY_CONFIG to enable)");
        }
    }

    let result = tokio::select! {
        res = controller::run_controller(state) => {
            res
        }
        _ = wait_for_shutdown_signal() => {
            info!("Shutdown signal received");
            shutdown_is_leader.store(false, Ordering::Relaxed);
            drop(shutdown_state);
            release_leader_lease(&shutdown_client, &shutdown_namespace, &shutdown_identity).await;
            Ok(())
        }
    };

    stellar_k8s::telemetry::shutdown_telemetry();
    result
}

async fn wait_for_shutdown_signal() {
    use tokio::signal;
    #[cfg(unix)]
    {
        use signal::unix::{signal as unix_signal, SignalKind};
        let mut sigterm =
            unix_signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");
        let mut sigint =
            unix_signal(SignalKind::interrupt()).expect("Failed to register SIGINT handler");
        tokio::select! {
            _ = sigterm.recv() => { info!("Received SIGTERM"); }
            _ = sigint.recv()  => { info!("Received SIGINT");  }
        }
    }
    #[cfg(not(unix))]
    {
        signal::ctrl_c().await.expect("Failed to listen for Ctrl-C");
        info!("Received Ctrl-C");
    }
}

async fn release_leader_lease(client: &kube::Client, namespace: &str, identity: &str) {
    let leases: Api<Lease> = Api::namespaced(client.clone(), namespace);
    let existing = match leases.get(LEASE_NAME).await {
        Ok(l) => l,
        Err(e) => {
            warn!("Could not fetch lease: {:?}", e);
            return;
        }
    };
    let currently_held_by = existing
        .spec
        .as_ref()
        .and_then(|s| s.holder_identity.as_deref())
        .unwrap_or("");
    if currently_held_by != identity {
        return;
    }
    let patch = serde_json::json!({ "spec": { "holderIdentity": null } });
    let _ = leases
        .patch(LEASE_NAME, &PatchParams::default(), &Patch::Merge(&patch))
        .await;
}

async fn run_leader_election(
    client: kube::Client,
    namespace: &str,
    identity: &str,
    is_leader: Arc<AtomicBool>,
) {
    let leases: Api<Lease> = Api::namespaced(client, namespace);

    loop {
        match try_acquire_or_renew(&leases, namespace, identity).await {
            Ok(true) => {
                if !is_leader.load(Ordering::Relaxed) {
                    info!("Acquired leadership: {}", LEASE_NAME);
                }
                is_leader.store(true, Ordering::Relaxed);
                tokio::time::sleep(RENEW_INTERVAL).await;
            }
            Ok(false) => {
                if is_leader.load(Ordering::Relaxed) {
                    warn!("Lost leadership: {}", LEASE_NAME);
                }
                is_leader.store(false, Ordering::Relaxed);
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
            Err(e) => {
                warn!("Leader election error: {:?}", e);
                is_leader.store(false, Ordering::Relaxed);
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
        }
    }
}

async fn try_acquire_or_renew(
    leases: &Api<Lease>,
    namespace: &str,
    identity: &str,
) -> Result<bool, kube::Error> {
    let now = Utc::now();

    match leases.get(LEASE_NAME).await {
        Ok(existing) => {
            let spec = existing.spec.as_ref();
            let current_holder = spec.and_then(|s| s.holder_identity.as_deref());

            if current_holder == Some(identity) {
                let patch = serde_json::json!({
                    "spec": {
                        "renewTime": MicroTime(now),
                        "leaseDurationSeconds": LEASE_DURATION_SECS,
                    }
                });
                leases
                    .patch(LEASE_NAME, &PatchParams::default(), &Patch::Merge(&patch))
                    .await?;
                return Ok(true);
            }

            let expired = spec
                .and_then(|s| s.renew_time.as_ref())
                .map(|renew| {
                    let duration = spec
                        .and_then(|s| s.lease_duration_seconds)
                        .unwrap_or(LEASE_DURATION_SECS);
                    let expiry = renew.0 + chrono::Duration::seconds(duration as i64);
                    now > expiry
                })
                .unwrap_or(true);

            if expired {
                let patch = serde_json::json!({
                    "spec": {
                        "holderIdentity": identity,
                        "acquireTime": MicroTime(now),
                        "renewTime": MicroTime(now),
                        "leaseDurationSeconds": LEASE_DURATION_SECS,
                    }
                });
                leases
                    .patch(LEASE_NAME, &PatchParams::default(), &Patch::Merge(&patch))
                    .await?;
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Err(kube::Error::Api(err)) if err.code == 404 => {
            let lease = Lease {
                metadata: ObjectMeta {
                    name: Some(LEASE_NAME.to_string()),
                    namespace: Some(namespace.to_string()),
                    ..Default::default()
                },
                spec: Some(k8s_openapi::api::coordination::v1::LeaseSpec {
                    holder_identity: Some(identity.to_string()),
                    acquire_time: Some(MicroTime(now)),
                    renew_time: Some(MicroTime(now)),
                    lease_duration_seconds: Some(LEASE_DURATION_SECS),
                    ..Default::default()
                }),
            };
            leases.create(&PostParams::default(), &lease).await?;
            Ok(true)
        }
        Err(e) => Err(e),
    }
}
