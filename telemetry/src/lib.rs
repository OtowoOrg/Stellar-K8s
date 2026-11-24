//! Stellar Telemetry — PromQL Metrics Exporter for Soroban Gas Profiling
//!
//! This crate provides an asynchronous log parser and Prometheus metrics
//! exporter for monitoring Soroban smart contract CPU and memory consumption.
//!
//! # Quick start
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
//! # Modules
//!
//! - [`parser`] — Zero-copy async log parser for Soroban RPC invocation streams.
//! - [`exporter`] — Prometheus metrics exporter with an HTTP `/metrics` endpoint.

pub mod exporter;
pub mod parser;
