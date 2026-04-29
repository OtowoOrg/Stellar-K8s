//! Axum HTTP server for the REST API
//!
//! Supports mTLS with optional graceful certificate reload: when the TLS config
//! is provided as a shared RustlsConfig, the rotation task can call
//! `reload_from_config` to adopt new certificates without dropping connections.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{middleware, routing::get, Router};
use axum_server::tls_rustls::RustlsConfig;
use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use rustls::ServerConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::controller::ControllerState;
use crate::{Error, Result};

use super::audit_handlers;
use super::auth;
use super::custom_metrics;
use super::dashboard_handlers;
use super::handlers;
use super::health_summary;
use super::job_handlers;
use super::scp_topology;

/// Build a rustls ServerConfig from PEM data (cert, key, CA for client verification).
/// Used for initial server setup and after certificate rotation to reload without restart.
pub fn build_tls_server_config(
    cert_pem: &[u8],
    key_pem: &[u8],
    ca_pem: &[u8],
) -> Result<Arc<ServerConfig>> {
    let certs = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::ConfigError(format!("Failed to parse certificates: {e}")))?;

    let key = PrivateKeyDer::from_pem_slice(key_pem)
        .map_err(|e| Error::ConfigError(format!("Failed to parse private key: {e}")))?;

    let mut roots = RootCertStore::empty();
    for cert_res in CertificateDer::pem_slice_iter(ca_pem) {
        let cert =
            cert_res.map_err(|e| Error::ConfigError(format!("Failed to parse CA cert: {e}")))?;
        roots
            .add(cert)
            .map_err(|e| Error::ConfigError(format!("Failed to add CA cert: {e}")))?;
    }

    let client_verifier = WebPkiClientVerifier::builder(roots.into())
        .build()
        .map_err(|e| Error::ConfigError(format!("Failed to create client verifier: {e}")))?;

    let server_config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(certs, key)
        .map_err(|e| Error::ConfigError(format!("Failed to create server config: {e}")))?;

    Ok(Arc::new(server_config))
}

/// Metrics endpoint handler
#[cfg(feature = "metrics")]
async fn metrics_handler() -> String {
    use prometheus_client::encoding::text::encode;
    let mut buffer = String::new();
    encode(&mut buffer, &crate::controller::metrics::REGISTRY).unwrap();
    buffer
}

/// Dashboard UI handler - serves the HTML dashboard
async fn dashboard_ui() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("dashboard_ui.html"))
}

/// Run the REST API server.
///
/// When `rustls_config` is `Some`, the server runs with mTLS. The same config can be
/// shared with a certificate rotation task: after rotating the Secret, build a new
/// `ServerConfig` and call `reload_from_config` on the RustlsConfig to adopt the new
/// certificate without dropping active connections.
#[tracing::instrument(
    skip(state),
    fields(node_name = "-", namespace = "-", reconcile_id = "-")
)]
pub async fn run_server(
    state: Arc<ControllerState>,
    rustls_config: Option<RustlsConfig>,
) -> Result<()> {
    let public = Router::new()
        .route("/health", get(handlers::health))
        .route("/healthz", get(handlers::healthz))
        .route("/readyz", get(handlers::readyz))
        .route("/livez", get(handlers::livez));

    let mut protected = Router::new()
        .route("/leader", get(handlers::leader_status))
        .route("/api/v1/nodes", get(handlers::list_nodes))
        .route("/api/v1/nodes/:namespace/:name", get(handlers::get_node))
        // Health summary API (Issue #552)
        .route("/v1/health/summary", get(health_summary::get_health_summary))
        .route("/v1/health/nodes", get(health_summary::get_node_health_status))
        .route("/v1/health/incidents", get(health_summary::get_health_incidents))
        // Log level dynamic control (GET is reader, POST is admin)
        .route(
            "/config/log-level",
            get(handlers::get_log_level),
        )
        .route(
            "/config/log-level",
            axum::routing::post(handlers::set_log_level)
                .route_layer(middleware::from_fn(auth::api_admin)),
        )
        // Compliance report
        .route("/api/v1/compliance/report", get(handlers::compliance_report))
        // Dashboard routes
        .route("/", get(dashboard_ui))
        .route("/api/v1/dashboard/overview", get(dashboard_handlers::dashboard_overview))
        .route(
            "/api/v1/dashboard/nodes/:namespace/:name/logs",
            get(dashboard_handlers::get_node_logs),
        )
        .route(
            "/api/v1/dashboard/nodes/:namespace/:name/conditions",
            get(dashboard_handlers::get_node_conditions),
        )
        .route(
            "/api/v1/dashboard/nodes/:namespace/:name/metrics",
            get(dashboard_handlers::get_node_metrics),
        )
        .route(
            "/api/v1/dashboard/nodes/:namespace/:name/actions",
            axum::routing::post(dashboard_handlers::execute_node_action)
                .route_layer(middleware::from_fn(auth::api_admin)),
        )
        // Operator logs
        .route("/api/v1/dashboard/operator/logs", get(dashboard_handlers::get_operator_logs))
        // SCP topology endpoints (REST snapshot + WebSocket stream)
        .route("/api/v1/quorum/topology", get(scp_topology::get_topology))
        .route("/api/v1/quorum/topology/stream", get(scp_topology::topology_ws))
        // Documentation search API
        .route("/api/v1/docs/search-index", get(handlers::get_search_index))
        // Background job monitoring dashboard
        .route("/api/v1/jobs", get(job_handlers::list_jobs))
        .route("/api/v1/jobs/stats", get(job_handlers::job_stats))
        // Audit log
        .route("/api/v1/audit-log", get(audit_handlers::list_audit_log))
        .route("/api/v1/audit-log/search", get(audit_handlers::search_audit_log))
        .route("/api/v1/audit-log/stream", get(audit_handlers::audit_log_stream))
        .route("/api/v1/audit-log/anomalies", get(audit_handlers::list_audit_anomalies))
        // Custom metrics API
        // Custom metrics API (Kubernetes custom.metrics.k8s.io/v1beta2)
        // Discovery endpoint — required by the HPA aggregation layer
        .route(
            "/apis/custom.metrics.k8s.io/v1beta2",
            get(custom_metrics::get_metrics_discovery),
        )
        .route(
            "/apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/pods/:name/:metric",
            get(custom_metrics::get_pod_metric),
        )
        .route(
            "/apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/stellarnodes.stellar.org/:name/:metric",
            get(custom_metrics::get_stellar_node_metric),
        )
        .layer(middleware::from_fn_with_state(state.clone(), auth::api_reader))
        // Horizon-specific convenience endpoint
        .route(
            "/apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/horizons.stellar.org/:name/:metric",
            get(custom_metrics::get_horizon_metric),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let mut app = public.merge(protected);

    #[cfg(feature = "metrics")]
    {
        app = app.route("/metrics", get(metrics_handler));
    }

    // Default to 9090 to match Prometheus scrape conventions and project docs.
    let port: u16 = std::env::var("REST_API_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9090);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    if let Some(tls_config) = rustls_config {
        info!("REST API server listening on {} with mTLS", addr);
        let listener = std::net::TcpListener::bind(addr)?;
        axum_server::from_tcp_rustls(listener, tls_config)
            .serve(app.into_make_service())
            .await
            .map_err(|e| Error::ConfigError(format!("Server error: {e}")))?;
    } else {
        info!("REST API server listening on {} (insecure)", addr);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| Error::ConfigError(format!("Failed to bind to {addr}: {e}")))?;

        axum::serve(listener, app)
            .await
            .map_err(|e| Error::ConfigError(format!("Server error: {e}")))?;
    }

    Ok(())
}
