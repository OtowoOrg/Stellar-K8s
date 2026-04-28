#![allow(missing_docs)]
//! Stellar-K8s: Cloud-Native Kubernetes Operator for Stellar Infrastructure
//!
//! This crate provides a Kubernetes operator for managing Stellar Core,
//! Horizon, and Soroban RPC nodes on Kubernetes clusters.
//!
//! # Overview
//!
//! Stellar-K8s extends Kubernetes with a `StellarNode` Custom Resource Definition (CRD),
//! enabling declarative management of Stellar infrastructure. The operator reconciles
//! the desired state of Validator, Horizon, and Soroban RPC nodes with the actual
//! cluster state.
//!
//! # Key Features
//!
//! - **Type-Safe CRD**: Strongly-typed Rust definitions for StellarNode specifications
//! - **Reconciliation Loop**: Automatic state management with leader election
//! - **Health Monitoring**: Built-in health checks for Horizon sync and Soroban RPC
//! - **Archive Management**: History archive integrity checks and pruning
//! - **Disaster Recovery**: Automated backup and restore capabilities
//! - **Service Mesh Integration**: Istio and other service mesh support
//! - **Metrics & Observability**: Prometheus metrics and distributed tracing
//! - **REST API**: Optional HTTP API for external integrations
//! - **Admission Webhooks**: WASM-based custom validation plugins
//!
//! # Modules
//!
//! - [`crd`] - Custom Resource Definition types and validation
//! - [`controller`] - Main reconciliation loop and resource management
//! - [`error`] - Centralized error types
//! - [`rest_api`] - Optional HTTP API server (requires `rest-api` feature)
//! - [`webhook`] - Optional admission webhook server (requires `admission-webhook` feature)
//! - [`backup`] - Backup and restore functionality
//! - [`scheduler`] - Pod scheduling and placement logic
//! - [`telemetry`] - Observability and tracing
//! - [`preflight`] - Pre-flight checks and validation
//! - [`infra`] - Infrastructure utilities
//! - [`search`] - Search and discovery utilities
//! - [`carbon_aware`] - Carbon-aware scheduling
//! - [`runbook`] - Troubleshooting runbook generation
//! - [`incident`] - Incident report generation
//! - [`byzantine`] - Byzantine fault detection and analysis
//! - [`log_scrub`] - PII and sensitive data scrubbing for logs
//! - [`version_check`] - Background version checking against GitHub
//!
//! # Example: Creating a Validator Node
//!
//! ```yaml
//! apiVersion: stellar.org/v1alpha1
//! kind: StellarNode
//! metadata:
//!   name: my-validator
//!   namespace: stellar
//! spec:
//!   nodeType: Validator
//!   network: Testnet
//!   version: "v21.0.0"
//!   storage:
//!     storageClass: "standard"
//!     size: "100Gi"
//!   validatorConfig:
//!     seedSecretRef: "my-validator-seed"
//!     enableHistoryArchive: true
//! ```

pub mod backup;
pub mod benchmark_compare;
pub mod byzantine;
pub mod carbon_aware;
pub mod controller;
pub mod crd;
pub mod error;
pub mod fork_detector;
pub mod incident;
pub mod infra;
pub mod log_scrub;
pub mod preflight;
pub mod runbook;
pub mod scheduler;
pub mod search;
pub mod telemetry;
pub mod version_check;

#[cfg(feature = "rest-api")]
pub mod rest_api;

#[cfg(feature = "admission-webhook")]
pub mod webhook;

pub use crate::error::{Error, Result};

/// Configuration for mutual TLS (mTLS) between operator and REST API clients.
///
/// When mTLS is enabled, the operator provisions a CA and server certificate,
/// and the REST API requires client certificates signed by that CA.
///
/// # Fields
///
/// - `cert_pem`: Server certificate in PEM format
/// - `key_pem`: Server private key in PEM format
/// - `ca_pem`: CA certificate for client verification in PEM format
#[derive(Clone, Debug)]
pub struct MtlsConfig {
    /// Server certificate in PEM format
    pub cert_pem: Vec<u8>,
    /// Server private key in PEM format
    pub key_pem: Vec<u8>,
    /// CA certificate for client verification in PEM format
    pub ca_pem: Vec<u8>,
}
