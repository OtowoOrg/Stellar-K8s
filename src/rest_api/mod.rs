//! REST API module for external integrations
//!
//! Provides an HTTP API for querying and managing StellarNodes.
//!
//! # Overview
//!
//! The REST API enables external systems to:
//! - Query node status and health
//! - List all StellarNode resources
//! - Access Prometheus metrics
//! - View the interactive dashboard
//! - Dynamically adjust log levels
//!
//! # Features
//!
//! - **mTLS Support**: Optional mutual TLS for secure client authentication
//! - **RBAC Integration**: Kubernetes RBAC-based authorization
//! - **Health Probes**: Kubernetes-compatible liveness and readiness probes
//! - **Metrics**: Prometheus metrics endpoint
//! - **Dashboard**: Interactive web UI for cluster monitoring
//! - **Custom Metrics**: Kubernetes custom metrics API support
//!
//! # Endpoints
//!
//! - `GET /health` - Basic health check
//! - `GET /healthz` - Kubernetes health probe
//! - `GET /readyz` - Kubernetes readiness probe
//! - `GET /livez` - Kubernetes liveness probe
//! - `GET /leader` - Leader election status
//! - `GET /api/v1/nodes` - List all StellarNodes
//! - `GET /api/v1/nodes/:namespace/:name` - Get specific StellarNode
//! - `GET /metrics` - Prometheus metrics
//! - `GET /` - Interactive dashboard
//! - `POST /config/log-level` - Adjust log level dynamically
//!
//! # Example: Querying Nodes
//!
//! ```bash
//! # List all nodes
//! curl https://operator:9090/api/v1/nodes \
//!   --cert client.crt --key client.key --cacert ca.crt
//!
//! # Get specific node
//! curl https://operator:9090/api/v1/nodes/stellar/my-validator \
//!   --cert client.crt --key client.key --cacert ca.crt
//! ```

mod auth;
mod custom_metrics;
mod dashboard_dto;
mod dashboard_handlers;
mod dto;
mod handlers;
mod oidc;
mod server;
mod sustainability;

pub use auth::{check_rbac_permission, k8s_rbac_auth};
pub use oidc::{oidc_auth, require_admin, require_reader, ApiRole, OidcConfig};
pub use server::{build_tls_server_config, run_server};
