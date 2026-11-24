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

//! `stellar-audit-sink` — out-of-process Kubernetes audit webhook sink (#1958).
//!
//! Run it in its own pod/namespace with its own service account so a
//! compromised API server or operator cannot rewrite history. Point the
//! kube-apiserver `--audit-webhook-config-file` at `POST /audit`.

use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use stellar_k8s::audit_ledger::monitor::ChainMonitor;
use stellar_k8s::audit_ledger::sink::{http, spawn_sink, NormalizeConfig, SinkConfig};

#[derive(Parser)]
#[command(
    name = "stellar-audit-sink",
    version,
    about = "Hash-chained Kubernetes audit webhook sink"
)]
struct Args {
    #[arg(long, env = "AUDIT_SINK_LISTEN", default_value = "0.0.0.0:8443")]
    listen: SocketAddr,
    /// Append-only ledger file.
    #[arg(
        long,
        env = "AUDIT_SINK_STORE",
        default_value = "/var/lib/stellar-audit/ledger.ndjson"
    )]
    store: PathBuf,
    /// Chain identifier (usually the cluster name); part of the genesis hash.
    #[arg(long, env = "AUDIT_SINK_CHAIN_ID")]
    chain_id: String,
    #[arg(long, default_value_t = 1024)]
    queue_capacity: usize,
    #[arg(long, default_value_t = 256)]
    max_batch: usize,
    #[arg(long, default_value_t = 200_000)]
    dedupe_window: usize,
    #[arg(long, env = "AUDIT_SINK_TLS_CERT")]
    tls_cert: Option<PathBuf>,
    #[arg(long, env = "AUDIT_SINK_TLS_KEY")]
    tls_key: Option<PathBuf>,
    /// File containing the bearer token the apiserver must present.
    #[arg(long, env = "AUDIT_SINK_TOKEN_FILE")]
    bearer_token_file: Option<PathBuf>,
    /// Keep client source IPs (personal data; off by default).
    #[arg(long)]
    include_source_ips: bool,
    /// Incremental integrity scan interval (gap alert latency ≈ this + alert eval).
    #[arg(long, default_value_t = 10)]
    scan_interval_secs: u64,
    /// Run a full verification every N scans (0 disables).
    #[arg(long, default_value_t = 360)]
    full_verify_every: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();
    let a = Args::parse();

    let mut cfg = SinkConfig::new(&a.store, &a.chain_id);
    cfg.queue_capacity = a.queue_capacity;
    cfg.max_batch = a.max_batch;
    cfg.dedupe_window = a.dedupe_window;
    cfg.normalize = NormalizeConfig {
        include_source_ips: a.include_source_ips,
    };
    let (sink, _writer) = spawn_sink(cfg)?;

    let monitor = ChainMonitor::new(&a.store, sink.metrics.clone());
    let interval = Duration::from_secs(a.scan_interval_secs.max(1));
    let full_every = a.full_verify_every;
    std::thread::Builder::new()
        .name("audit-ledger-monitor".into())
        .spawn(move || monitor.run(interval, full_every))?;

    let token = match &a.bearer_token_file {
        Some(p) => Some(std::fs::read_to_string(p)?.trim().to_string()),
        None => None,
    };
    let app = http::router(sink, token);

    match (&a.tls_cert, &a.tls_key) {
        (Some(c), Some(k)) => {
            let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(c, k).await?;
            tracing::info!(listen = %a.listen, "audit sink listening (TLS)");
            axum_server::bind_rustls(a.listen, tls)
                .serve(app.into_make_service())
                .await?;
        }
        (None, None) => {
            tracing::warn!(
                "serving audit webhook WITHOUT TLS; use only behind a trusted network or mesh"
            );
            let l = tokio::net::TcpListener::bind(a.listen).await?;
            axum::serve(l, app).await?;
        }
        _ => return Err("--tls-cert and --tls-key must be given together".into()),
    }
    Ok(())
}
