//! Stellar-K8s Operator Entry Point
//!
//! Starts the Kubernetes controller and optional REST API server.

use std::sync::Arc;

use stellar_k8s::{controller, Error};
use tracing::{info, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Error> {
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
        .map_err(|e| Error::KubeError(e))?;

    info!("Connected to Kubernetes cluster");

    // Get the operator's namespace from env
    let namespace = std::env::var("POD_NAMESPACE").unwrap_or_else(|_| "default".to_string());
    info!("Operating namespace: {}", namespace);

    // Create shared controller state
    let state = Arc::new(controller::ControllerState {
        client: client.clone(),
    });

    // Start the REST API server (always running if feature enabled)
    #[cfg(feature = "rest-api")]
    {
        let api_state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = stellar_k8s::rest_api::run_server(api_state).await {
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
