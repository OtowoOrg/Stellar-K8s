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

//! Fail-closed readiness and metrics sidecar for admission webhook TLS.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use stellar_k8s::logging::{init_binary_subscriber, LogOutputFormat};
use stellar_k8s::webhook::cert_health::{validate_files, CertHealthState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{error, info, warn, Level};

#[derive(Parser, Debug)]
#[command(
    name = "stellar-cert-health",
    about = "Validate an admission webhook serving certificate and gate readiness"
)]
struct Args {
    #[arg(long, env = "CERT_PATH", default_value = "/certs/tls.crt")]
    cert_path: String,

    #[arg(long, env = "KEY_PATH", default_value = "/certs/tls.key")]
    key_path: String,

    #[arg(long, env = "CA_PATH", default_value = "/certs/ca.crt")]
    ca_path: String,

    #[arg(
        long,
        env = "CERT_DNS_NAME",
        default_value = "stellar-webhook.stellar-webhook.svc"
    )]
    dns_name: String,

    #[arg(long, env = "CERT_HEALTH_BIND", default_value = "0.0.0.0:9443")]
    bind: String,

    #[arg(long, env = "CERT_RELOAD_INTERVAL_SECS", default_value_t = 5)]
    reload_interval_secs: u64,

    #[arg(long, env = "RUST_LOG", default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let level = args.log_level.parse().unwrap_or(Level::INFO);
    init_binary_subscriber(level, LogOutputFormat::Json);

    let state = Arc::new(CertHealthState::default());
    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("failed to bind cert-health endpoint {}", args.bind))?;
    info!(bind = %args.bind, dns_name = %args.dns_name, "cert-health sidecar started");

    let validation_state = state.clone();
    let cert_path = args.cert_path.clone();
    let key_path = args.key_path.clone();
    let ca_path = args.ca_path.clone();
    let dns_name = args.dns_name.clone();
    let interval = Duration::from_secs(args.reload_interval_secs.max(1));
    tokio::spawn(async move {
        loop {
            match validate_files(&cert_path, &key_path, &ca_path, &dns_name) {
                Ok(current) => {
                    validation_state.replace(&current);
                    info!("serving certificate passed trust, identity, validity, and DNS checks");
                }
                Err(err) => {
                    validation_state.invalidate();
                    error!(error = %err, "serving certificate failed validation; readiness is failing closed");
                }
            }
            tokio::time::sleep(interval).await;
        }
    });

    let http_state = state.clone();
    loop {
        let (mut socket, _) = listener.accept().await?;
        let state = http_state.clone();
        tokio::spawn(async move {
            let mut request = [0_u8; 2048];
            let size = match socket.read(&mut request).await {
                Ok(size) => size,
                Err(err) => {
                    warn!(error = %err, "failed to read cert-health request");
                    return;
                }
            };
            let request = String::from_utf8_lossy(&request[..size]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let (status, content_type, body) = if path == "/metrics" {
                (
                    "200 OK",
                    "text/plain; version=0.0.4; charset=utf-8",
                    state.prometheus(now),
                )
            } else if state.is_valid() {
                ("200 OK", "text/plain; charset=utf-8", "ok\n".to_string())
            } else {
                (
                    "503 Service Unavailable",
                    "text/plain; charset=utf-8",
                    "certificate validation failed\n".to_string(),
                )
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            if let Err(err) = socket.write_all(response.as_bytes()).await {
                warn!(error = %err, "failed to write cert-health response");
            }
        });
    }
}
