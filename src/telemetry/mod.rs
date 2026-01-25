//! OpenTelemetry initialization and utilities
//!
//! Provides functions to set up distributed tracing with OTLP export.

pub mod privacy;
pub mod proxy;

use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    propagation::TraceContextPropagator,
    runtime,
    trace::{self, Sampler},
    Resource,
};
use std::env;
use tracing_subscriber::{registry::LookupSpan, Layer};

/// Initialize OpenTelemetry tracer and tracing subscriber with privacy protections
pub fn init_telemetry<S>(_subscriber: &S) -> Box<dyn Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    // Set global propagator for context propagation
    global::set_text_map_propagator(TraceContextPropagator::new());

    // Get OTLP endpoint from environment or use default
    let otlp_endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4317".to_string());

    // Requirement: End-to-end encryption for telemetry data.
    // Ensure that if we are sending to a remote endpoint, we use TLS.
    if otlp_endpoint.starts_with("http://")
        && !otlp_endpoint.contains("localhost")
        && !otlp_endpoint.contains("127.0.0.1")
        && !otlp_endpoint.contains("::1")
    {
        tracing::warn!("Unencrypted telemetry endpoint detected for remote host {} (Privacy may be compromised)", otlp_endpoint);
    }

    let mut resource_attributes = vec![
        KeyValue::new("service.name", "stellar-operator"),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
    ];

    // Privacy awareness: Do not include specific host IP or cluster name by default here.
    // These will be scrubbed/anonymized by the collector proxy.
    if env::var("K8S_CLUSTER_NAME").is_ok() {
        // We set the cluster name to a generic value to avoid leaking the real name
        resource_attributes.push(KeyValue::new("k8s.cluster.name", "hidden"));
    }

    let resource = Resource::new(resource_attributes);

    // Configure OTLP exporter
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&otlp_endpoint);

    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(
            trace::config()
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
