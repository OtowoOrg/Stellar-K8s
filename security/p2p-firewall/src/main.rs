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

//! P2P Gossip Network Threat Detection Firewall — binary entry point.
//!
//! Listens on the Stellar SCP gossip port (default 11625), analyzes incoming
//! packet streams for malformed XDR payloads, flood patterns, and handshake
//! failures, then enforces IP blacklisting via Kubernetes NetworkPolicy and
//! iptables rules.

use anyhow::Result;
use clap::Parser;
use p2p_firewall::{FirewallConfig, FirewallEngine};
use tracing::info;
use tracing_subscriber::EnvFilter;

/// P2P Gossip Network Threat Detection Firewall for Stellar SCP
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Port to monitor for SCP traffic
    #[arg(long, default_value_t = 11625)]
    port: u16,

    /// Flood detection threshold: packets per second per IP before blacklisting
    #[arg(long, default_value_t = 100)]
    flood_pps_threshold: u32,

    /// How long (seconds) a blacklisted IP remains blocked
    #[arg(long, default_value_t = 300)]
    blacklist_duration_secs: u64,

    /// Kubernetes namespace for NetworkPolicy enforcement
    #[arg(long, default_value = "stellar")]
    namespace: String,

    /// Prometheus metrics bind address
    #[arg(long, default_value = "0.0.0.0:9090")]
    metrics_addr: String,

    /// Malformed-payload count per IP within the sampling window before blacklisting
    #[arg(long, default_value_t = 10)]
    malformed_threshold: u32,

    /// Failed-handshake count per IP within the sampling window before blacklisting
    #[arg(long, default_value_t = 5)]
    handshake_fail_threshold: u32,

    /// Time window (seconds) for rate counters
    #[arg(long, default_value_t = 10)]
    window_secs: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Structured JSON logging for production; RUST_LOG overrides level.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    info!(
        port = cli.port,
        namespace = %cli.namespace,
        metrics_addr = %cli.metrics_addr,
        "Starting P2P Gossip Network Threat Detection Firewall"
    );

    let config = FirewallConfig {
        scp_port: cli.port,
        flood_pps_threshold: cli.flood_pps_threshold,
        blacklist_duration_secs: cli.blacklist_duration_secs,
        namespace: cli.namespace,
        metrics_addr: cli.metrics_addr,
        malformed_threshold: cli.malformed_threshold,
        handshake_fail_threshold: cli.handshake_fail_threshold,
        window_secs: cli.window_secs,
    };

    let engine = FirewallEngine::new(config).await?;
    engine.run().await
}
