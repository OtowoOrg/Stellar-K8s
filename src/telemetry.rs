//! OpenTelemetry initialization and utilities
//!
//! Provides functions to set up distributed tracing with OTLP export.

use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::resource::Resource;
use opentelemetry_sdk::runtime;
use opentelemetry_sdk::trace::{Config, Sampler};
use std::env;
use tracing_subscriber::{registry::LookupSpan, Layer};

/// Initialize OpenTelemetry tracer and tracing subscriber
pub fn init_telemetry<S>(_subscriber: &S) -> Box<dyn Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    // Set global propagator for context propagation
    global::set_text_map_propagator(TraceContextPropagator::new());

    // Get OTLP endpoint from environment or use default
    let otlp_endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4317".to_string());

    let resource = Resource::new(vec![
        KeyValue::new("service.name", "stellar-operator"),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
    ]);

    // Configure OTLP exporter
    // Note: We use grpc as default but it can be changed to http/protobuf if needed
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&otlp_endpoint);

    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(
            Config::default()
                .with_resource(resource)
                .with_sampler(Sampler::AlwaysOn),
        )
        .install_batch(runtime::Tokio)
        .expect("Failed to initialize OpenTelemetry tracer");

    // Create tracing layer
    tracing_opentelemetry::layer().with_tracer(tracer).boxed()
}

/// Shutdown OpenTelemetry tracer
pub fn shutdown_telemetry() {
    global::shutdown_tracer_provider();
}
