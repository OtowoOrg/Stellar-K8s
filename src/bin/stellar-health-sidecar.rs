use anyhow::{Context, Result};
use std::env;
use std::sync::Arc;
use stellar_k8s::controller::health_check_sidecar::{
    create_router, sync_monitor_loop, HealthCheckState,
};
use tokio::sync::RwLock;
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

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
    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("stellar-health-sidecar v{}: Error: Failed to bind to {}: {}", env!("CARGO_PKG_VERSION"), bind_addr, e);
            std::process::exit(1);
        }
    };

    info!("Health check sidecar listening on {}", bind_addr);

    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("stellar-health-sidecar v{}: Error: Failed to start HTTP server: {}", env!("CARGO_PKG_VERSION"), e);
        std::process::exit(1);
    }
}
