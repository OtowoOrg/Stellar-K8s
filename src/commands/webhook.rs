use crate::cli::{LogFormat, WebhookArgs};
use stellar_k8s::log_scrub::ScrubLayer;
use stellar_k8s::Error;
use tracing::{info, info_span, warn, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[cfg(feature = "admission-webhook")]
pub async fn run_webhook(args: WebhookArgs) -> Result<(), Error> {
    use stellar_k8s::webhook::{runtime::WasmRuntime, server::WebhookServer};

    // Initialize tracing
    let env_filter = EnvFilter::builder()
        .with_default_directive(args.log_level.parse().unwrap_or(Level::INFO.into()))
        .from_env_lossy();

    let scrub_layer = ScrubLayer::new();

    match args.log_format {
        LogFormat::Json => {
            let fmt_layer = fmt::layer().json().with_target(true);
            tracing_subscriber::registry()
                .with(env_filter)
                .with(scrub_layer)
                .with(fmt_layer)
                .init();
        }
        LogFormat::Pretty => {
            let fmt_layer = fmt::layer().pretty().with_target(true);
            tracing_subscriber::registry()
                .with(env_filter)
                .with(scrub_layer)
                .with(fmt_layer)
                .init();
        }
    }

    let namespace = std::env::var("OPERATOR_NAMESPACE").unwrap_or_else(|_| "default".to_string());

    let root_span =
        info_span!("operator", node_name = "-", namespace = %namespace, reconcile_id = "-");
    let _root_enter = root_span.enter();

    info!(
        "Starting Webhook Server v{} on {}",
        env!("CARGO_PKG_VERSION"),
        args.bind
    );

    // Parse bind address
    let addr: std::net::SocketAddr = args
        .bind
        .parse()
        .map_err(|e| Error::ConfigError(format!("Invalid bind address: {e}")))?;

    // Initialize Wasm runtime
    let runtime = WasmRuntime::new()
        .map_err(|e| Error::ConfigError(format!("Failed to initialize Wasm runtime: {e}")))?;

    // Create webhook server
    let mut server = WebhookServer::new(runtime);

    // Configure TLS if provided
    if let (Some(cert_path), Some(key_path)) = (args.cert_path, args.key_path) {
        info!("Configuring TLS with cert: {cert_path}, key: {key_path}");
        server = server.with_tls(cert_path, key_path);
    } else {
        warn!("Running webhook server without TLS (not recommended for production)");
    }

    // Start the server
    info!("Webhook server listening on {addr}");
    server
        .start(addr)
        .await
        .map_err(|e| Error::ConfigError(format!("Webhook server error: {e}")))?;

    Ok(())
}

#[cfg(not(feature = "admission-webhook"))]
pub async fn run_webhook(_args: WebhookArgs) -> Result<(), Error> {
    Err(Error::ConfigError(
        "Webhook feature not enabled. Rebuild with --features admission-webhook".to_string(),
    ))
}
