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
//! Intelligent Prometheus Metrics Rate-Limiter Proxy.
//!
//! This module provides a lightweight HTTP proxy that sits between Prometheus
//! and the Stellar Core `/metrics` endpoint.  It protects high-throughput
//! Stellar nodes from CPU spikes caused by frequent metric scrapes by:
//!
//! - **Rate-limiting** full upstream scrapes to a configurable minimum interval
//!   (default 1 s).  Requests arriving sooner receive a cached response.
//! - **Filtering** metric series and label names according to rules loaded from
//!   a Kubernetes ConfigMap.
//! - **Caching** slow-changing metric families (node version, uptime strings)
//!   by TTL while always passing high-frequency metrics (SCP rounds, CPU) fresh.
//!
//! # Module layout
//!
//! | Module | Responsibility |
//! |--------|---------------|
//! | [`config`] | `ProxyConfig` YAML schema + ConfigMap loading |
//! | [`filter`] | Prometheus text parser, label-drop, series-filter |
//! | [`limiter`] | Rate-limiting + per-family TTL cache |
//! | [`server`] | Axum HTTP server wiring everything together |
//!
//! # Quick start
//!
//! ```no_run
//! use stellar_telemetry::proxy::server::run_from_env;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     run_from_env().await
//! }
//! ```

pub mod config;
pub mod filter;
pub mod limiter;
pub mod server;

// Re-export the most commonly used types for convenience.
pub use config::ProxyConfig;
pub use filter::MetricsFilter;
pub use limiter::{CacheStats, RateLimiter, ScrapeDecision};
pub use server::{build_router, run, run_from_env, ProxyState};
