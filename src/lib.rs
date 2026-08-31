#![allow(missing_docs)]
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
#![allow(non_snake_case)]
#![allow(rustdoc::broken_intra_doc_links)]
#![allow(rustdoc::private_intra_doc_links)]
#![allow(rustdoc::bare_urls)]
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

// When the `profiling` feature is enabled, use jemalloc so heap profiles can be
// exported via jemalloc_pprof (see docs/operations/profiling-runbook.md).
#[cfg(feature = "profiling")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// Enable jemalloc profiling machinery; sampling stays inactive until
// `activate_jemalloc_profiling` (heap endpoint) or MALLOC_CONF overrides this.
#[cfg(feature = "profiling")]
#[allow(non_upper_case_globals)]
#[export_name = "malloc_conf"]
static MALLOC_CONF: &[u8] = b"prof:true,prof_active:false,lg_prof_sample:19\0";

pub mod api_gateway;
pub mod backup;
pub mod benchmark_compare;
pub mod bootstrap_verify;
pub mod byzantine;
pub mod canary_deployment;
pub mod capacity_planning;
pub mod carbon_aware;
pub mod cli;
pub mod commands;
pub mod compliance;
pub mod compliance_scan;
pub mod config_mgmt;
pub mod consensus_monitoring;
pub mod controller;
pub mod cost_optimization;
pub mod crd;
pub mod data_pipeline;
pub mod db_management;
pub mod db_migrations;
pub mod deployment_strategy;
pub mod error;
pub mod error_budget;
pub mod event_processing;
pub mod feature_flags;
pub mod federation;
pub mod fork_detector;
pub mod incident;
pub mod infra;
pub mod load_balancer;
pub mod load_modeling;
pub mod log_aggregation;
pub mod log_scrub;
pub mod logging;
pub mod message_queue;
pub mod network_observability;
pub mod plugin_sdk;
pub mod preflight;
pub mod profiling;
pub mod runbook;
pub mod scheduler;
pub mod schema_registry;
pub mod sdk;
pub mod search;
pub mod security;
pub mod telemetry;
pub mod version_check;
pub mod websocket_streaming;

#[cfg(feature = "rest-api")]
pub mod rest_api;

#[cfg(feature = "admission-webhook")]
pub mod webhook;

pub mod middleware;

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
