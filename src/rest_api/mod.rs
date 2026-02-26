//! REST API module for external integrations
//!
//! Provides an HTTP API for querying and managing StellarNodes.

mod custom_metrics;
mod dto;
mod handlers;
mod server;
mod sustainability;

pub use server::run_server;
