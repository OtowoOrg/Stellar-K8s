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
//! `stellar-fork-detector` — Fork Detection Sidecar binary.
//!
//! Monitors the local Stellar Core ledger hash and compares it in real-time
//! against multiple public anchor nodes to detect potential network forks.
//!
//! # Usage
//!
//! ```text
//! stellar-fork-detector \
//!   --local-endpoint http://localhost:11626 \
//!   --anchor https://horizon.stellar.org/ledgers?order=desc&limit=1 \
//!   --anchor https://horizon-testnet.stellar.org/ledgers?order=desc&limit=1 \
//!   --anchor https://horizon.stellar.lobstr.co/ledgers?order=desc&limit=1 \
//!   --divergence-threshold 3 \
//!   --metrics-bind 0.0.0.0:9102
//! ```

use anyhow::Result;
use clap::Parser;
use stellar_k8s::fork_detector::{run_fork_detector, ForkDetectorConfig};
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser, Debug)]
#[command(
    name = "stellar-fork-detector",
    version,
    about = "Fork Detection Sidecar — compares local ledger hash against public anchors",
    long_about = "Runs as a sidecar alongside a Stellar Core node.\n\
        Periodically fetches the latest ledger hash from the local Core and 3+ public\n\
        anchor nodes. Alerts if a divergence persists for more than N consecutive ledgers.\n\
        Exports 'stellar_fork_detector_sync_confidence' as a Prometheus metric.\n\n\
        EXAMPLES:\n  \
        stellar-fork-detector \\\n  \
          --local-endpoint http://localhost:11626 \\\n  \
          --anchor https://horizon.stellar.org/ledgers?order=desc&limit=1 \\\n  \
          --anchor https://horizon.stellar.lobstr.co/ledgers?order=desc&limit=1 \\\n  \
          --anchor https://horizon.satoshipay.io/ledgers?order=desc&limit=1"
)]
struct Args {
    /// HTTP endpoint of the local Stellar Core node.
    /// Env: FORK_DETECTOR_LOCAL_ENDPOINT
    #[arg(
        long,
        env = "FORK_DETECTOR_LOCAL_ENDPOINT",
        default_value = "http://localhost:11626"
    )]
    local_endpoint: String,

    /// Public anchor node Horizon endpoints (repeat for multiple).
    /// Env: FORK_DETECTOR_ANCHORS (comma-separated)
    #[arg(long = "anchor", env = "FORK_DETECTOR_ANCHORS", value_delimiter = ',')]
    anchors: Vec<String>,

    /// How often to poll all nodes (seconds).
    /// Env: FORK_DETECTOR_POLL_INTERVAL
    #[arg(long, env = "FORK_DETECTOR_POLL_INTERVAL", default_value_t = 5)]
    poll_interval: u64,

    /// HTTP request timeout per node (seconds).
    /// Env: FORK_DETECTOR_REQUEST_TIMEOUT
    #[arg(long, env = "FORK_DETECTOR_REQUEST_TIMEOUT", default_value_t = 5)]
    request_timeout: u64,

    /// Number of consecutive diverging ledgers before firing a fork alert.
    /// Env: FORK_DETECTOR_DIVERGENCE_THRESHOLD
    #[arg(long, env = "FORK_DETECTOR_DIVERGENCE_THRESHOLD", default_value_t = 3)]
    divergence_threshold: u64,

    /// Address to bind the Prometheus /metrics HTTP server.
    /// Env: FORK_DETECTOR_METRICS_BIND
    #[arg(
        long,
        env = "FORK_DETECTOR_METRICS_BIND",
        default_value = "0.0.0.0:9102"
    )]
    metrics_bind: String,

    /// Stellar network name (for metric labels).
    /// Env: FORK_DETECTOR_NETWORK
    #[arg(long, env = "FORK_DETECTOR_NETWORK", default_value = "mainnet")]
    network: String,

    /// Node identifier (for metric labels).
    /// Env: FORK_DETECTOR_NODE_ID
    #[arg(long, env = "FORK_DETECTOR_NODE_ID", default_value = "local")]
    node_id: String,

    /// Log level (trace / debug / info / warn / error).
    /// Env: RUST_LOG
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Same OTel wiring `stellar_k8s::logging::init_binary_subscriber` gives
    // every other sidecar (issue #1369) — kept inline here, rather than
    // switching to that helper, so the existing `--log-level` filter string
    // (which may be a full directive like "info,foo=debug", not just a bare
    // level) keeps working exactly as before.
    let use_otel = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok();
    let registry = tracing_subscriber::registry()
        .with(fmt::layer().json())
        .with(EnvFilter::new(&args.log_level));
    if use_otel {
        let otel_layer = stellar_k8s::telemetry::init_telemetry(&registry);
        let trace_id_layer = stellar_k8s::telemetry::trace_id_layer();
        registry.with(otel_layer).with(trace_id_layer).init();
    } else {
        registry.init();
    }

    // Use default anchors if none provided.
    let anchor_endpoints = if args.anchors.is_empty() {
        vec![
            "https://horizon.stellar.org/ledgers?order=desc&limit=1".to_string(),
            "https://horizon.stellar.lobstr.co/ledgers?order=desc&limit=1".to_string(),
            "https://horizon.satoshipay.io/ledgers?order=desc&limit=1".to_string(),
        ]
    } else {
        args.anchors
    };

    info!(
        node_id = %args.node_id,
        network = %args.network,
        local_endpoint = %args.local_endpoint,
        anchors = anchor_endpoints.len(),
        divergence_threshold = args.divergence_threshold,
        "stellar-fork-detector starting"
    );

    let config = ForkDetectorConfig {
        local_endpoint: args.local_endpoint,
        anchor_endpoints,
        poll_interval_secs: args.poll_interval,
        request_timeout_secs: args.request_timeout,
        divergence_threshold_ledgers: args.divergence_threshold,
        metrics_bind_addr: args.metrics_bind,
        network: args.network,
        node_id: args.node_id,
    };

    run_fork_detector(config).await
}
