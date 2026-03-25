use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use clap::{Parser, Subcommand};
use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, ObjectMeta, Patch, PatchParams, PostParams};
use stellar_k8s::{controller, crd::StellarNode, preflight, Error};
use tracing::{debug, info, warn, Level};
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
    /// Run the admission webhook server
    Webhook(WebhookArgs),
    /// Show version and build information
    Version,
    /// Show cluster information
    Info(InfoArgs),
    /// Local simulator (kind/k3s + operator + demo validators)
    Simulator(SimulatorCli),
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

    /// Run preflight checks and exit without starting the operator
    #[arg(long, env = "PREFLIGHT_ONLY")]
    preflight_only: bool,
}

#[derive(Parser, Debug)]
struct InfoArgs {
    /// Operator namespace
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    namespace: String,
}

#[derive(clap::Subcommand, Debug)]
enum SimulatorCmd {
    /// Create cluster, install operator manifests, print health hints
    Up(SimulatorUpArgs),
}

#[derive(Parser, Debug)]
struct SimulatorCli {
    #[command(subcommand)]
    command: SimulatorCmd,
}

#[derive(Parser, Debug)]
struct SimulatorUpArgs {
    /// kind cluster name (ignored for k3s)
    #[arg(long, default_value = "stellar-sim")]
    cluster_name: String,

    /// Namespace for operator and demo nodes
    #[arg(long, default_value = "stellar-system")]
    namespace: String,

    /// Prefer k3s when both kind and k3s are available
    #[arg(long, default_value_t = false)]
    use_k3s: bool,
}

#[derive(Parser, Debug)]
struct WebhookArgs {
    /// Bind address for the webhook server
    #[arg(long, env = "WEBHOOK_BIND", default_value = "0.0.0.0:8443")]
    bind: String,

    /// TLS certificate path
    #[arg(long, env = "WEBHOOK_CERT_PATH")]
    cert_path: Option<String>,

    /// TLS key path
    #[arg(long, env = "WEBHOOK_KEY_PATH")]
    key_path: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    log_level: String,
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
        Commands::Webhook(webhook_args) => {
            return run_webhook(webhook_args).await;
        }
        Commands::Simulator(cli) => {
            return run_simulator(cli).await;
        }
    }
}

async fn run_simulator(cli: SimulatorCli) -> Result<(), Error> {
    match cli.command {
        SimulatorCmd::Up(args) => simulator_up(args).await,
    }
}

async fn simulator_up(args: SimulatorUpArgs) -> Result<(), Error> {
    use std::process::Command;

    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let validators = repo_root.join("examples/simulator/three-validators.yaml");
    let csi_sample = repo_root.join("config/samples/test-stellarnode.yaml");

    let have_kind = Command::new("kind")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let have_k3s = Command::new("k3s")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if args.use_k3s && have_k3s {
        println!("Using k3s: ensure the cluster is running and kubeconfig is configured.");
    } else if have_kind {
        println!("Creating kind cluster '{}' ...", args.cluster_name);
        let status = Command::new("kind")
            .args([
                "create",
                "cluster",
                "--name",
                &args.cluster_name,
                "--wait",
                "120s",
            ])
            .status()
            .map_err(|e| Error::ConfigError(format!("kind failed to start: {e}")))?;
        if !status.success() {
            println!("Note: kind create failed (cluster may already exist); continuing.");
        }
    } else {
        return Err(Error::ConfigError(
            "Neither kind nor k3s found in PATH. Install kind (https://kind.sigs.k8s.io/) or k3s."
                .to_string(),
        ));
    }

    println!("Applying StellarNode CRD ...");
    let crd_path = repo_root.join("config/crd/stellarnode-crd.yaml");
    let mut kubectl_crd = Command::new("kubectl");
    kubectl_crd.args(["apply", "-f", crd_path.to_str().unwrap()]);
    run_cmd(kubectl_crd, "kubectl apply CRD")?;

    println!("Ensuring namespace {} ...", args.namespace);
    let ns_pipe = format!(
        "kubectl create namespace {} --dry-run=client -o yaml | kubectl apply -f -",
        args.namespace
    );
    let mut sh_ns = Command::new("sh");
    sh_ns.arg("-c").arg(&ns_pipe);
    run_cmd(sh_ns, "kubectl ensure namespace")?;

    println!("Building operator image stellar-operator:sim …");
    let _ = Command::new("docker")
        .args(["build", "-t", "stellar-operator:sim", "."])
        .current_dir(&repo_root)
        .status();

    if have_kind && !args.use_k3s {
        let _ = Command::new("kind")
            .args([
                "load",
                "docker-image",
                "stellar-operator:sim",
                "--name",
                &args.cluster_name,
            ])
            .status();
    }

    println!("Applying simulator operator Deployment (dev-only RBAC) …");
    let op_yaml = temp_operator_yaml(&args.namespace)?;
    let mut kubectl_op = Command::new("kubectl");
    kubectl_op.args(["apply", "-f", &op_yaml]);
    run_cmd(kubectl_op, "kubectl apply operator")?;

    println!("Applying demo workloads …");
    if validators.exists() {
        let mut kubectl_val = Command::new("kubectl");
        kubectl_val.args(["apply", "-f", validators.to_str().unwrap()]);
        run_cmd(kubectl_val, "kubectl apply validators")?;
    } else if csi_sample.exists() {
        println!("Using {}", csi_sample.display());
        let mut kubectl_sample = Command::new("kubectl");
        kubectl_sample.args(["apply", "-f", csi_sample.to_str().unwrap()]);
        run_cmd(kubectl_sample, "kubectl apply sample")?;
    } else {
        println!("No examples/simulator/three-validators.yaml — skipping demo StellarNodes.");
    }

    println!("\n=== stellar simulator up — summary ===");
    println!(
        "  StellarNodes: kubectl get stellarnode -n {}",
        args.namespace
    );
    println!("  Services:     kubectl get svc -n {}", args.namespace);
    let _ = Command::new("kubectl")
        .args(["get", "stellarnode,svc", "-n", &args.namespace])
        .status();
    Ok(())
}

fn run_cmd(mut c: std::process::Command, ctx: &str) -> Result<(), Error> {
    let st = c
        .status()
        .map_err(|e| Error::ConfigError(format!("{ctx}: {e}")))?;
    if !st.success() {
        return Err(Error::ConfigError(format!("{ctx}: exit {:?}", st.code())));
    }
    Ok(())
}

fn temp_operator_yaml(namespace: &str) -> Result<String, Error> {
    use std::io::Write;
    let dir = std::env::temp_dir();
    let path = dir.join("stellar-operator-sim.yaml");
    let mut f = std::fs::File::create(&path).map_err(|e| Error::ConfigError(e.to_string()))?;
    write!(
        f,
        r#"apiVersion: v1
kind: ServiceAccount
metadata:
  name: stellar-operator
  namespace: {namespace}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: stellar-operator-sim
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: cluster-admin
subjects:
  - kind: ServiceAccount
    name: stellar-operator
    namespace: {namespace}
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: stellar-operator
  namespace: {namespace}
  labels:
    app: stellar-operator
spec:
  replicas: 1
  selector:
    matchLabels:
      app: stellar-operator
  template:
    metadata:
      labels:
        app: stellar-operator
    spec:
      serviceAccountName: stellar-operator
      containers:
        - name: operator
          image: stellar-operator:sim
          imagePullPolicy: Never
          command: ["stellar-operator", "run", "--namespace", "{namespace}"]
          env:
            - name: RUST_LOG
              value: info
            - name: OPERATOR_NAMESPACE
              value: "{namespace}"
"#
    )
    .map_err(|e| Error::ConfigError(e.to_string()))?;
    Ok(path.to_string_lossy().to_string())
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

#[cfg(feature = "admission-webhook")]
async fn run_webhook(args: WebhookArgs) -> Result<(), Error> {
    use stellar_k8s::webhook::{runtime::WasmRuntime, server::WebhookServer};

    // Initialize tracing
    let env_filter = EnvFilter::builder()
        .with_default_directive(args.log_level.parse().unwrap_or(Level::INFO.into()))
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_target(true))
        .init();

    info!(
        "Starting Webhook Server v{} on {}",
        env!("CARGO_PKG_VERSION"),
        args.bind
    );

    // Parse bind address
    let addr: std::net::SocketAddr = args
        .bind
        .parse()
        .map_err(|e| Error::ConfigError(format!("Invalid bind address: {e}")))?;

    // Initialize Wasm runtime
    let runtime = WasmRuntime::new()
        .map_err(|e| Error::ConfigError(format!("Failed to initialize Wasm runtime: {e}")))?;

    // Create webhook server
    let mut server = WebhookServer::new(runtime);

    // Configure TLS if provided
    if let (Some(cert_path), Some(key_path)) = (args.cert_path, args.key_path) {
        info!("Configuring TLS with cert: {cert_path}, key: {key_path}");
        server = server.with_tls(cert_path, key_path);
    } else {
        warn!("Running webhook server without TLS (not recommended for production)");
    }

    // Start the server
    info!("Webhook server listening on {addr}");
    server
        .start(addr)
        .await
        .map_err(|e| Error::ConfigError(format!("Webhook server error: {e}")))?;

    Ok(())
}

#[cfg(not(feature = "admission-webhook"))]
async fn run_webhook(_args: WebhookArgs) -> Result<(), Error> {
    Err(Error::ConfigError(
        "Webhook feature not enabled. Rebuild with --features admission-webhook".to_string(),
    ))
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
    let operator_config = stellar_k8s::controller::OperatorConfig::load();
    let state = Arc::new(controller::ControllerState {
        client: client.clone(),
        enable_mtls: args.enable_mtls,
        operator_namespace: args.namespace.clone(),
        mtls_config: mtls_config.clone(),
        dry_run: args.dry_run,
        is_leader: Arc::clone(&is_leader),
        operator_config: Arc::new(operator_config),
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

    // Run the main controller loop.
    // The kube-rs controller already listens for OS signals via .shutdown_on_signal(),
    // so this select! adds explicit lease release before the process exits.
    let shutdown_state = state.clone();
    let shutdown_client = client.clone();
    let shutdown_namespace = args.namespace.clone();
    let shutdown_is_leader = Arc::clone(&is_leader);
    let shutdown_identity = holder_identity.clone();

    let result = tokio::select! {
        res = controller::run_controller(state) => {
            res
        }
        _ = wait_for_shutdown_signal() => {
            info!("Shutdown signal received – draining reconciliations and releasing leader lease...");
            // Mark as non-leader so the renewal loop stops promoting us.
            shutdown_is_leader.store(false, std::sync::atomic::Ordering::Relaxed);
            drop(shutdown_state); // release controller state references
            // Release the Kubernetes Lease so a peer can take over immediately.
            release_leader_lease(&shutdown_client, &shutdown_namespace, &shutdown_identity).await;
            // The controller future's .shutdown_on_signal() will have already
            // stopped processing new work by this point; return cleanly.
            Ok(())
        }
    };

    // Flush any remaining traces
    stellar_k8s::telemetry::shutdown_telemetry();

    result
}

/// Wait for SIGTERM or SIGINT (Ctrl-C).
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

/// Clear holderIdentity on the leader Lease so peers can promote immediately.
/// Errors are logged but not propagated – we are already shutting down.
async fn release_leader_lease(client: &kube::Client, namespace: &str, identity: &str) {
    use k8s_openapi::api::coordination::v1::Lease;
    use kube::api::{Api, Patch, PatchParams};

    let leases: Api<Lease> = Api::namespaced(client.clone(), namespace);
    let existing = match leases.get(LEASE_NAME).await {
        Ok(l) => l,
        Err(e) => {
            warn!("Could not fetch lease for release: {:?}", e);
            return;
        }
    };
    let currently_held_by = existing
        .spec
        .as_ref()
        .and_then(|s| s.holder_identity.as_deref())
        .unwrap_or("");
    if currently_held_by != identity {
        debug!("Lease is held by {currently_held_by:?}, skipping release");
        return;
    }
    let patch = serde_json::json!({ "spec": { "holderIdentity": null } });
    match leases
        .patch(LEASE_NAME, &PatchParams::default(), &Patch::Merge(&patch))
        .await
    {
        Ok(_) => info!("Released leader lease {LEASE_NAME}"),
        Err(e) => warn!("Failed to release leader lease: {:?}", e),
    }
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
