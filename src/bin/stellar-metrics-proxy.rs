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
//! `stellar-metrics-proxy` binary entry point.
//!
//! Lightweight HTTP proxy that sits between Prometheus and the Stellar Core
//! `/metrics` endpoint.  It enforces a scrape rate-limit, drops configured
//! labels, filters metric series, and caches slow-changing metric families —
//! reducing CPU spikes on high-throughput validator nodes.
//!
//! # Configuration
//!
//! All configuration is read from a YAML file mounted from a Kubernetes
//! ConfigMap (default path: `/etc/stellar-metrics-proxy/config.yaml`).
//! Override the path with the `METRICS_PROXY_CONFIG` environment variable.
//!
//! # Environment variables
//!
//! | Variable              | Default                               | Description |
//! |-----------------------|---------------------------------------|-------------|
//! | `METRICS_PROXY_CONFIG`| `/etc/stellar-metrics-proxy/config.yaml` | Path to ConfigMap-mounted YAML config |
//! | `RUST_LOG`            | `info`                                | Log filter (tracing-subscriber) |
//!
//! # Kubernetes deployment pattern
//!
//! ```
//!  ┌─────────────────────────────────────────────┐
//!  │  Stellar Core Pod                           │
//!  │                                             │
//!  │  ┌──────────────────┐  :11626/metrics       │
//!  │  │  stellar-core    │◄─────────────────┐    │
//!  │  └──────────────────┘                  │    │
//!  │                                        │    │
//!  │  ┌──────────────────┐  :9091/metrics   │    │
//!  │  │ metrics-proxy    │──────────────────┘    │
//!  │  │  (this binary)   │◄── Prometheus         │
//!  │  └──────────────────┘                       │
//!  └─────────────────────────────────────────────┘
//! ```

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialise structured JSON logging.  Default level: info.
    tracing_subscriber::fmt()
        .json()
        .with_target(true)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Load config from ConfigMap-mounted file (or built-in defaults).
    let config = stellar_telemetry::proxy::ProxyConfig::load();

    tracing::info!(
        upstream = %config.upstream,
        listen = %config.listen_addr,
        min_scrape_interval_ms = config.min_scrape_interval_ms,
        label_drop_rules = config.label_drop_rules.len(),
        series_filter_rules = config.series_filter_rules.len(),
        cache_rules = config.cache_rules.len(),
        "Starting stellar-metrics-proxy"
    );

    stellar_telemetry::proxy::run(config).await
}
