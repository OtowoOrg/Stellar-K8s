use clap::{Parser, Subcommand};
use std::sync::Arc;
use stellar_k8s::{controller, crd::StellarNode, Error};
use tracing::{info, Level};
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
    /// Run in dry-run mode (calculate changes without applying them)
    #[arg(long, env = "DRY_RUN")]
    dry_run: bool,
}

#[derive(Parser, Debug)]
struct InfoArgs {
    /// Operator namespace
    #[arg(long, env = "OPERATOR_NAMESPACE", default_value = "default")]
    namespace: String,
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

        // Ensure CA and Server Cert exist
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

        // Fetch the secret to get the PEM data
        let secrets: kube::Api<k8s_openapi::api::core::v1::Secret> =
            kube::Api::namespaced(client_clone, &namespace);
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
    let _namespace = std::env::var("POD_NAMESPACE").unwrap_or_else(|_| "default".to_string());
    let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| {
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown-host".to_string())
    });

    info!("Leader election using holder ID: {}", hostname);

    // TODO: Re-enable leader election once kube-leader-election version is aligned
    // let lease_name = "stellar-operator-leader";
    // let lock = LeaseLock::new(...);

    // Create shared controller state
    let state = Arc::new(controller::ControllerState {
        client: client.clone(),
        enable_mtls: args.enable_mtls,
        operator_namespace: args.namespace.clone(),
        mtls_config: mtls_config.clone(),
        dry_run: args.dry_run,
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

    // Start the REST API server
    // Start the REST API server (always running if feature enabled)
    #[cfg(feature = "rest-api")]
    {
        let api_state = state.clone();

        tokio::spawn(async move {
            if let Err(e) = stellar_k8s::rest_api::run_server(api_state, mtls_config).await {
                tracing::error!("REST API server error: {:?}", e);
            }
        });
    }

    // Run the main controller loop
    let result = controller::run_controller(state).await;

    // Flush any remaining traces
    stellar_k8s::telemetry::shutdown_telemetry();

    result
}
