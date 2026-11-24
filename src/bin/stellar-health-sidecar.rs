// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
use anyhow::{Context, Result};
use std::env;
use std::sync::Arc;
use stellar_k8s::controller::health_check_sidecar::{
    create_router, sync_monitor_loop, HealthCheckState,
};
use stellar_k8s::controller::peer_connectivity::parse_known_peers;
use stellar_k8s::logging::{init_binary_subscriber, LogOutputFormat};
use tokio::sync::RwLock;
use tracing::{error, info, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    init_binary_subscriber(Level::INFO, LogOutputFormat::Json);

    info!("Starting Stellar Health Check Sidecar");

    // Get configuration from environment
    let core_url = env::var("CORE_URL").unwrap_or_else(|_| "http://localhost:11626".to_string());
    let core_version = env::var("CORE_VERSION").unwrap_or_else(|_| "v21.3.1".to_string());
    let archive_urls: Vec<String> = env::var("ARCHIVE_URLS")
        .map(|s| {
            s.split(',')
                .map(|u| u.trim().to_string())
                .filter(|u| !u.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8081".to_string());
    let known_peers_toml = env::var("KNOWN_PEERS").unwrap_or_default();

    info!("Core URL: {}", core_url);
    info!("Core Version: {}", core_version);
    info!("Archive URLs: {:?}", archive_urls);
    info!("Bind address: {}", bind_addr);

    // Create shared state
    let state = HealthCheckState {
        core_url: core_url.clone(),
        core_version,
        archive_urls,
        sync_status: Arc::new(RwLock::new(Default::default())),
        peer_connectivity: Arc::new(RwLock::new(None)),
    };

    // Start peer reachability monitoring. Probing is what turns a validator
    // that has silently dropped out of quorum into something observable.
    let peers = parse_known_peers(&known_peers_toml);
    if peers.is_empty() {
        info!("No KNOWN_PEERS configured; peer connectivity probing disabled");
    } else {
        info!("Probing {} configured peers for reachability", peers.len());
    }
    state.spawn_peer_monitor(peers);

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
