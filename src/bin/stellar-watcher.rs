//! `stellar-watcher` — Byzantine Monitoring Watcher binary.
//!
//! Polls a single Stellar Core node from a geographically dispersed vantage
//! point and exports Prometheus metrics for Byzantine partition detection.
//!
//! # Usage
//!
//! ```text
//! stellar-watcher \
//!   --watcher-id watcher-us-east-1 \
//!   --cloud aws \
//!   --region us-east-1 \
//!   --network mainnet \
//!   --node-endpoint http://stellar-core.stellar-system.svc.cluster.local:11626 \
//!   --poll-interval 10 \
//!   --metrics-bind 0.0.0.0:9101
//! ```
//!
//! All flags can also be set via environment variables (see `--help`).

use anyhow::Result;
use clap::Parser;
use stellar_k8s::byzantine::types::WatcherConfig;
use stellar_k8s::byzantine::watcher::run_watcher;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser, Debug)]
#[command(
    name = "stellar-watcher",
    version,
    about = "Byzantine Monitoring Watcher — polls a Stellar Core node and exports Prometheus metrics",
    long_about = "Deploys as a sidecar or standalone container in multiple cloud regions.\n\
        Each instance independently observes the Stellar network from its vantage point\n\
        and exports ledger hash/sequence metrics. A central Prometheus instance aggregates\n\
        all watchers and fires an alert when >20% disagree on the current ledger hash.\n\n\
        EXAMPLES:\n  \
        stellar-watcher --watcher-id watcher-us-east-1 --cloud aws --region us-east-1 \\\n  \
          --network mainnet --node-endpoint http://stellar-core:11626\n\n  \
        stellar-watcher --config /etc/watcher/config.yaml"
)]
struct Args {
    /// Unique identifier for this watcher instance.
    /// Env: WATCHER_ID
    #[arg(long, env = "WATCHER_ID", default_value = "watcher-default")]
    watcher_id: String,

    /// Cloud provider label (aws / gcp / azure / on-prem / …).
    /// Env: WATCHER_CLOUD
    #[arg(long, env = "WATCHER_CLOUD", default_value = "unknown")]
    cloud: String,

    /// Geographic region label (us-east-1 / eu-west-1 / ap-south-1 / …).
    /// Env: WATCHER_REGION
    #[arg(long, env = "WATCHER_REGION", default_value = "unknown")]
    region: String,

    /// Stellar network name (mainnet / testnet / futurenet / custom).
    /// Env: WATCHER_NETWORK
    #[arg(long, env = "WATCHER_NETWORK", default_value = "mainnet")]
    network: String,

    /// HTTP endpoint of the Stellar Core node to poll.
    /// Example: http://stellar-core.stellar-system.svc.cluster.local:11626
    /// Env: WATCHER_NODE_ENDPOINT
    #[arg(
        long,
        env = "WATCHER_NODE_ENDPOINT",
        default_value = "http://localhost:11626"
    )]
    node_endpoint: String,

    /// How often to poll the Stellar Core node (seconds).
    /// Env: WATCHER_POLL_INTERVAL
    #[arg(long, env = "WATCHER_POLL_INTERVAL", default_value_t = 10)]
    poll_interval: u64,

    /// HTTP request timeout for Stellar Core polls (seconds).
    /// Env: WATCHER_REQUEST_TIMEOUT
    #[arg(long, env = "WATCHER_REQUEST_TIMEOUT", default_value_t = 5)]
    request_timeout: u64,

    /// Address to bind the Prometheus /metrics HTTP server.
    /// Env: WATCHER_METRICS_BIND
    #[arg(
        long,
        env = "WATCHER_METRICS_BIND",
        default_value = "0.0.0.0:9101"
    )]
    metrics_bind: String,

    /// Log level (trace / debug / info / warn / error).
    /// Env: RUST_LOG
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialise structured logging.
    tracing_subscriber::registry()
        .with(fmt::layer().json())
        .with(EnvFilter::new(&args.log_level))
        .init();

    info!(
        watcher_id = %args.watcher_id,
        cloud = %args.cloud,
        region = %args.region,
        network = %args.network,
        node_endpoint = %args.node_endpoint,
        poll_interval_secs = args.poll_interval,
        metrics_bind = %args.metrics_bind,
        "stellar-watcher starting"
    );

    let config = WatcherConfig {
        watcher_id: args.watcher_id,
        cloud: args.cloud,
        region: args.region,
        network: args.network,
        node_endpoint: args.node_endpoint,
        poll_interval_secs: args.poll_interval,
        request_timeout_secs: args.request_timeout,
        metrics_bind_addr: args.metrics_bind,
    };

    run_watcher(config).await
}
