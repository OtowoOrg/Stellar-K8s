//! Axum HTTP server for the REST API

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::get, Router};
use axum_server::tls_rustls::RustlsConfig;
use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::controller::ControllerState;
use crate::{Error, MtlsConfig, Result};

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::{
    extract::Request,
    middleware::{self, Next},
    response::Response,
};
use opentelemetry::{global, propagation::Extractor};
use tracing_opentelemetry::OpenTelemetrySpanExt;

struct HeaderExtractor<'a>(&'a HeaderMap);

impl<'a> Extractor for HeaderExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v: &HeaderValue| v.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k: &HeaderName| k.as_str()).collect()
    }
}

async fn extract_trace_context(request: Request, next: Next) -> Response {
    let parent_cx = global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(request.headers()))
    });
    tracing::Span::current().set_parent(parent_cx);
    next.run(request).await
}

use super::custom_metrics;
use super::handlers;

/// Metrics endpoint handler
#[cfg(feature = "metrics")]
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
        .layer(middleware::from_fn(extract_trace_context))
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

    if let Some(config) = mtls_config {
        info!("REST API server listening on {} with mTLS", addr);

        // Load certificates
        let certs = CertificateDer::pem_slice_iter(&config.cert_pem)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::ConfigError(format!("Failed to parse certificates: {e}")))?;

        // Load private key
        let key = PrivateKeyDer::from_pem_slice(&config.key_pem)
            .map_err(|e| Error::ConfigError(format!("Failed to parse private key: {e}")))?;

        // Load CA for client verification
        let mut roots = RootCertStore::empty();
        for cert_res in CertificateDer::pem_slice_iter(&config.ca_pem) {
            let cert = cert_res
                .map_err(|e| Error::ConfigError(format!("Failed to parse CA cert: {e}")))?;
            roots
                .add(cert)
                .map_err(|e| Error::ConfigError(format!("Failed to add CA cert: {e}")))?;
        }

        let client_verifier = WebPkiClientVerifier::builder(roots.into())
            .build()
            .map_err(|e| Error::ConfigError(format!("Failed to create client verifier: {e}")))?;

        // Create rustls ServerConfig
        let server_config = rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(certs, key)
            .map_err(|e| Error::ConfigError(format!("Failed to create server config: {e}")))?;

        let rustls_config = RustlsConfig::from_config(Arc::new(server_config));

        let listener = std::net::TcpListener::bind(addr)?;
        axum_server::from_tcp_rustls(listener, rustls_config)
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
