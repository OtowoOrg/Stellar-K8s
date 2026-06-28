use anyhow::{Context, Result};
use std::env;
use std::sync::Arc;
use stellar_k8s::controller::health_check_sidecar::{
    create_router, sync_monitor_loop, HealthCheckState,
};
use tokio::sync::RwLock;
use stellar_k8s::logging::{init_binary_subscriber, LogOutputFormat};
use tracing::{error, info, Level};

#[tokio::main]
async fn main() -> Result<()> {
    init_binary_subscriber(Level::INFO, LogOutputFormat::Json);

    info!("Starting Stellar Health Check Sidecar");

    // Get configuration from environment
    let core_url = env::var("CORE_URL").unwrap_or_else(|_| "http://localhost:11626".to_string());
    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8081".to_string());

    info!("Core URL: {}", core_url);
    info!("Bind address: {}", bind_addr);

    // Create shared state
    let state = HealthCheckState {
        core_url: core_url.clone(),
        sync_status: Arc::new(RwLock::new(Default::default())),
    };

    // Start sync monitoring loop
    let monitor_state = state.clone();
    tokio::spawn(async move {
        sync_monitor_loop(monitor_state).await;
    });

    // Create and start the HTTP server
    let app = create_router(state);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("Failed to bind to {}", bind_addr))?;

    info!("Health check sidecar listening on {}", bind_addr);

    axum::serve(listener, app)
        .await
        .context("Failed to start HTTP server")?;

    Ok(())
}
