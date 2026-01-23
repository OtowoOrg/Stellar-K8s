//! Axum HTTP server for the REST API

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::get, Router};
use axum_server::tls_rustls::RustlsConfig;
use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::controller::ControllerState;
use crate::{Error, MtlsConfig, Result};

use super::handlers;
use super::custom_metrics;

/// Metrics endpoint handler
async fn metrics_handler() -> String {
    use prometheus_client::encoding::text::encode;
    let mut buffer = String::new();
    encode(&mut buffer, &crate::controller::metrics::REGISTRY).unwrap();
    buffer
}

/// Run the REST API server
pub async fn run_server(
    state: Arc<ControllerState>,
    mtls_config: Option<MtlsConfig>,
) -> Result<()> {
    let app = Router::new()
        .route("/health", get(handlers::health))
        .route("/metrics", get(metrics_handler))
        .route("/api/v1/nodes", get(handlers::list_nodes))
        .route("/api/v1/nodes/:namespace/:name", get(handlers::get_node))
        .route("/apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/pods/:name/:metric", get(custom_metrics::get_pod_metric))
        .route("/apis/custom.metrics.k8s.io/v1beta2/namespaces/:namespace/stellarnodes.stellar.org/:name/:metric", get(custom_metrics::get_stellar_node_metric))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));

    if let Some(config) = mtls_config {
        info!("REST API server listening on {} with mTLS", addr);

        // Load certificates
        let mut cert_reader = std::io::BufReader::new(&config.cert_pem[..]);
        let certs = rustls_pemfile::certs(&mut cert_reader)
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| Error::ConfigError(format!("Failed to parse certificates: {}", e)))?;

        // Load private key
        let mut key_reader = std::io::BufReader::new(&config.key_pem[..]);
        let key = rustls_pemfile::private_key(&mut key_reader)
            .map_err(|e| Error::ConfigError(format!("Failed to parse private key: {}", e)))?
            .ok_or_else(|| Error::ConfigError("No private key found in PEM".to_string()))?;

        // Load CA for client verification
        let mut roots = RootCertStore::empty();
        let mut ca_reader = std::io::BufReader::new(&config.ca_pem[..]);
        for cert in rustls_pemfile::certs(&mut ca_reader) {
            roots
                .add(cert.map_err(|e| Error::ConfigError(e.to_string()))?)
                .map_err(|e| Error::ConfigError(format!("Failed to add CA cert: {}", e)))?;
        }

        let client_verifier = WebPkiClientVerifier::builder(roots.into())
            .build()
            .map_err(|e| Error::ConfigError(format!("Failed to create client verifier: {}", e)))?;

        // Create rustls ServerConfig
        let server_config = rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(certs, key)
            .map_err(|e| Error::ConfigError(format!("Failed to create server config: {}", e)))?;

        let rustls_config = RustlsConfig::from_config(Arc::new(server_config));

        axum_server::bind_rustls(addr, rustls_config)
            .serve(app.into_make_service())
            .await
            .map_err(|e| Error::ConfigError(format!("Server error: {}", e)))?;
    } else {
        info!("REST API server listening on {} (insecure)", addr);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| Error::ConfigError(format!("Failed to bind to {}: {}", addr, e)))?;

        axum::serve(listener, app)
            .await
            .map_err(|e| Error::ConfigError(format!("Server error: {}", e)))?;
    }

    Ok(())
}
