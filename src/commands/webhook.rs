use crate::cli::{LogFormat, WebhookArgs};
use stellar_k8s::logging::{init_subscriber, LogOutputFormat, SubscriberConfig};
use stellar_k8s::Error;
use tracing::{info, info_span, warn, Level};

#[cfg(feature = "admission-webhook")]
pub async fn run_webhook(args: WebhookArgs) -> Result<(), Error> {
    use stellar_k8s::webhook::{runtime::WasmRuntime, server::WebhookServer};

    let log_format = match args.log_format {
        LogFormat::Json => LogOutputFormat::Json,
        LogFormat::Pretty => LogOutputFormat::Pretty,
    };
    let log_level = args.log_level.parse().unwrap_or(Level::INFO);

    init_subscriber(SubscriberConfig {
        level: log_level,
        format: log_format,
        ..Default::default()
    });

    let namespace = std::env::var("OPERATOR_NAMESPACE").unwrap_or_else(|_| "default".to_string());

    let root_span =
        info_span!("operator", node_name = "-", namespace = %namespace, reconcile_id = "-");
    let _root_enter = root_span.enter();

    info!(
        "Starting Webhook Server v{} on {}",
        env!("CARGO_PKG_VERSION"),
        args.bind
    );

    let addr: std::net::SocketAddr = args
        .bind
        .parse()
        .map_err(|e| Error::ConfigError(format!("Invalid bind address: {e}")))?;

    let runtime = WasmRuntime::new()
        .map_err(|e| Error::ConfigError(format!("Failed to initialize Wasm runtime: {e}")))?;

    let mut server = WebhookServer::new(runtime);

    if let (Some(cert_path), Some(key_path)) = (args.cert_path, args.key_path) {
        info!("Configuring TLS with cert: {cert_path}, key: {key_path}");
        server = server.with_tls(cert_path, key_path);
    } else {
        warn!("Running webhook server without TLS (not recommended for production)");
    }

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
