//! Axum HTTP server for the REST API
//!
//! Supports mTLS with optional graceful certificate reload: when the TLS config
//! is provided as a shared RustlsConfig, the rotation task can call
//! `reload_from_config` to adopt new certificates without dropping connections.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::get, Router};
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

use super::custom_metrics;
use super::handlers;

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

/// Run the REST API server.
///
/// When `rustls_config` is `Some`, the server runs with mTLS. The same config can be
/// shared with a certificate rotation task: after rotating the Secret, build a new
/// `ServerConfig` and call `reload_from_config` on the RustlsConfig to adopt the new
/// certificate without dropping active connections.
pub async fn run_server(
    state: Arc<ControllerState>,
    rustls_config: Option<RustlsConfig>,
) -> Result<()> {
    let mut app = Router::new()
        .route("/health", get(handlers::health))
        .route("/leader", get(handlers::leader_status))
        .route("/api/v1/nodes", get(handlers::list_nodes))
        .route("/api/v1/nodes/:namespace/:name", get(handlers::get_node))
        .route(
            "/apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/pods/:name/:metric",
            get(custom_metrics::get_pod_metric),
        )
        .route(
            "/apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/stellarnodes.stellar.org/:name/:metric",
            get(custom_metrics::get_stellar_node_metric),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

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
