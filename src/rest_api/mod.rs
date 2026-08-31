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
//! - `GET /api/versions` - API version catalog (URL-path versioning)
//! - `GET /metrics` - Prometheus metrics
//! - `GET /` - Interactive dashboard
//! - `POST /config/log-level` - Adjust log level dynamically
//! - `GET /api/v1/debug/pprof/profile` - CPU profile (feature `profiling` + runtime flag)
//! - `GET /api/v1/debug/pprof/heap` - Heap profile (feature `profiling` + runtime flag)
//!
//! Versioning uses the URL path (`/api/vN/...`). See `docs/api/versioning.md`.
//! Profiling requires Admin auth; see `docs/operations/profiling-runbook.md`.
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

mod audit_handlers;
mod auth;
mod compliance_handlers;
pub mod custom_metrics;
mod dashboard_dto;
mod dashboard_handlers;
pub mod dto;
mod handlers;
mod health_summary;
mod horizon_cache_handlers;
mod job_handlers;
pub mod metrics_store;
mod oidc;
mod profiling;
mod resource_optimization_handlers;
mod scp_topology;
#[cfg(feature = "rest-api")]
pub mod schema_validation;
mod server;
pub mod stellar_metrics_server;
mod versioning;

pub mod gateway;

pub use auth::{check_rbac_permission, k8s_rbac_auth};
pub use health_summary::{get_health_incidents, get_health_summary, get_node_health_status};
pub use metrics_store::StellarMetricsStore;
pub use oidc::{oidc_auth, require_admin, require_reader, ApiRole, OidcConfig};
pub use server::{build_router, build_tls_server_config, run_server};
