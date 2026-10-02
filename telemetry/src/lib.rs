//! Stellar Telemetry — PromQL Metrics Exporter and Proxy for Stellar Infrastructure
//!
//! This crate provides:
//! - An asynchronous log parser and Prometheus metrics exporter for monitoring
//!   Soroban smart contract CPU and memory consumption.
//! - An intelligent rate-limiter proxy that sits between Prometheus and the
//!   Stellar Core `/metrics` endpoint, protecting high-throughput nodes from
//!   CPU spikes caused by frequent scrapes.
//!
//! # Quick start — gas exporter
//!
//! ```no_run
//! use stellar_telemetry::exporter::run_exporter;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     run_exporter("0.0.0.0:9100").await
//! }
//! ```
//!
//! # Quick start — metrics proxy
//!
//! ```no_run
//! use stellar_telemetry::proxy::run_from_env;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     run_from_env().await
//! }
//! ```
//!
//! # Modules
//!
//! - [`parser`]  — Zero-copy async log parser for Soroban RPC invocation streams.
//! - [`exporter`] — Prometheus metrics exporter with an HTTP `/metrics` endpoint.
//! - [`proxy`]   — Intelligent rate-limiter proxy for the Stellar Core metrics endpoint.

pub mod exporter;
pub mod parser;
pub mod proxy;
