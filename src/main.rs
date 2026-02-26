use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use clap::{Parser, Subcommand};
use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, ObjectMeta, Patch, PatchParams, PostParams};
use stellar_k8s::{controller, crd::StellarNode, Error};
use tracing::{info, warn, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the operator
    Run(RunArgs),
    /// Show version and build information
    Version,
    /// Show cluster information
    Info(InfoArgs),
}

#[derive(Parser, Debug)]
struct RunArgs {
    /// Enable mTLS for the REST API
    #[arg(long, env = "ENABLE_MTLS")]
    enable_mtls: bool,

    /// Operator namespace
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    namespace: String,

    /// Run in dry-run mode (calculate changes without applying them)
    #[arg(long, env = "DRY_RUN")]
    dry_run: bool,

    /// Run the latency-aware scheduler instead of the operator
    #[arg(long, env = "RUN_SCHEDULER")]
    scheduler: bool,

    /// Custom scheduler name (used when --scheduler is set)
    #[arg(long, env = "SCHEDULER_NAME", default_value = "stellar-scheduler")]
    scheduler_name: String,
}

#[derive(Parser, Debug)]
struct InfoArgs {
    /// Operator namespace
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    namespace: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Args::parse();

    match args.command {
        Commands::Version => {
            println!("Stellar-K8s Operator v{}", env!("CARGO_PKG_VERSION"));
            println!("Build Date: {}", env!("BUILD_DATE"));
            println!("Git SHA: {}", env!("GIT_SHA"));
            println!("Rust Version: {}", env!("RUST_VERSION"));
            return Ok(());
        }
        Commands::Info(info_args) => {
            return run_info(info_args).await;
        }
        Commands::Run(run_args) => {
            return run_operator(run_args).await;
        }
    }
}

async fn run_info(args: InfoArgs) -> Result<(), Error> {
    // Initialize Kubernetes client
    let client = kube::Client::try_default()
        .await
        .map_err(Error::KubeError)?;

    let api: kube::Api<StellarNode> = kube::Api::namespaced(client, &args.namespace);
    let nodes = api
        .list(&Default::default())
        .await
        .map_err(Error::KubeError)?;

    println!("Managed Stellar Nodes: {}", nodes.items.len());
    Ok(())
}

async fn run_operator(args: RunArgs) -> Result<(), Error> {
    // Initialize tracing with OpenTelemetry
    let env_filter = EnvFilter::builder()
        .with_default_directive(Level::INFO.into())
        .from_env_lossy();

    let fmt_layer = fmt::layer().with_target(true);

    // Register the subscriber with both stdout logging and OpenTelemetry tracing
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    // Only enable OTEL if an endpoint is provided or via a flag
    let otel_enabled = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok();

    if otel_enabled {
        let otel_layer = stellar_k8s::telemetry::init_telemetry(&registry);
        registry.with(otel_layer).init();
        info!("OpenTelemetry tracing initialized");
    } else {
        registry.init();
        info!("OpenTelemetry tracing disabled (OTEL_EXPORTER_OTLP_ENDPOINT not set)");
    }

    info!(
        "Starting Stellar-K8s Operator v{}",
        env!("CARGO_PKG_VERSION")
    );

    // Initialize Kubernetes client
    let client = kube::Client::try_default()
        .await
        .map_err(Error::KubeError)?;

    info!("Connected to Kubernetes cluster");

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

        let secrets: kube::Api<k8s_openapi::api::core::v1::Secret> =
            kube::Api::namespaced(client_clone.clone(), &namespace);
        let secret = secrets
            .get(controller::mtls::SERVER_CERT_SECRET_NAME)
            .await
            .map_err(Error::KubeError)?;
        let data = secret
            .data
            .ok_or_else(|| Error::ConfigError("Secret has no data".to_string()))?;

        let cert_pem = data
            .get("tls.crt")
            .ok_or_else(|| Error::ConfigError("Missing tls.crt".to_string()))?
            .0
            .clone();
        let key_pem = data
            .get("tls.key")
            .ok_or_else(|| Error::ConfigError("Missing tls.key".to_string()))?
            .0
            .clone();
        let ca_pem = data
            .get("ca.crt")
            .ok_or_else(|| Error::ConfigError("Missing ca.crt".to_string()))?
            .0
            .clone();

        Some(stellar_k8s::MtlsConfig {
            cert_pem,
            key_pem,
            ca_pem,
        })
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

        tokio::spawn(async move {
            run_leader_election(lease_client, &lease_ns, &identity, is_leader_bg).await;
        });
    }

    // Create shared controller state
    let state = Arc::new(controller::ControllerState {
        client: client.clone(),
        enable_mtls: args.enable_mtls,
        operator_namespace: args.namespace.clone(),
        mtls_config: mtls_config.clone(),
        dry_run: args.dry_run,
        is_leader: Arc::clone(&is_leader),
    });

    // Start the peer discovery manager
    let peer_discovery_client = client.clone();
    let peer_discovery_config = controller::PeerDiscoveryConfig::default();
    tokio::spawn(async move {
        let manager =
            controller::PeerDiscoveryManager::new(peer_discovery_client, peer_discovery_config);
        if let Err(e) = manager.run().await {
            tracing::error!("Peer discovery manager error: {:?}", e);
        }
    });

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

        tokio::spawn(async move {
            if let Err(e) = stellar_k8s::rest_api::run_server(api_state, server_tls).await {
                tracing::error!("REST API server error: {:?}", e);
            }
        });

        // Certificate rotation: when mTLS is enabled, periodically check and rotate
        // server cert if within threshold, then graceful reload of TLS config
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

            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600)); // check hourly
                interval.tick().await; // first tick completes immediately
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
                            // Rotation performed: fetch new secret and reload TLS
                            let secrets: kube::Api<k8s_openapi::api::core::v1::Secret> =
                                kube::Api::namespaced(rotation_client.clone(), &rotation_namespace);
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
                                            info!(
                                                "TLS server config reloaded with new certificate"
                                            );
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to build TLS config after rotation: {:?}",
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        Ok(false) => {}
                        Err(e) => {
                            tracing::error!("Certificate rotation check failed: {:?}", e);
                        }
                    }
                }
            });
        }
    }

    // Run the main controller loop
    let result = controller::run_controller(state).await;

    // Flush any remaining traces
    stellar_k8s::telemetry::shutdown_telemetry();

    result
}

const LEASE_NAME: &str = "stellar-operator-leader";
const LEASE_DURATION_SECS: i32 = 15;
const RENEW_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

async fn run_leader_election(
    client: kube::Client,
    namespace: &str,
    identity: &str,
    is_leader: Arc<AtomicBool>,
) {
    let leases: Api<Lease> = Api::namespaced(client, namespace);

    loop {
        match try_acquire_or_renew(&leases, identity).await {
            Ok(true) => {
                if !is_leader.load(Ordering::Relaxed) {
                    info!("Acquired leadership for lease {}", LEASE_NAME);
                }
                is_leader.store(true, Ordering::Relaxed);
                tokio::time::sleep(RENEW_INTERVAL).await;
            }
            Ok(false) => {
                if is_leader.load(Ordering::Relaxed) {
                    warn!("Lost leadership for lease {}", LEASE_NAME);
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

async fn try_acquire_or_renew(leases: &Api<Lease>, identity: &str) -> Result<bool, kube::Error> {
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
                info!(
                    "Lease held by {:?} has expired, taking over",
                    current_holder
                );
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
                    namespace: Some(
                        leases
                            .resource_url()
                            .split('/')
                            .nth(5)
                            .unwrap_or("default")
                            .to_string(),
                    ),
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
            info!("Created lease {} with holder {}", LEASE_NAME, identity);
            Ok(true)
        }
        Err(e) => Err(e),
    }
}
