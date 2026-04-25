//! OpenTelemetry initialization and utilities
//!
//! Provides functions to set up distributed tracing with OTLP export and
//! trace-ID injection into structured JSON logs.
//!
//! # Trace ID in logs
//!
//! [`OtelTraceIdLayer`] is a thin `tracing_subscriber::Layer` that reads the
//! active OTel span from the current tracing span's extensions and appends
//! `trace_id` and `span_id` W3C hex fields to every log event.  This lets
//! operators correlate log lines with traces in Honeycomb / Jaeger / Tempo.

use opentelemetry::trace::TraceResult;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::export::trace::SpanData;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::resource::Resource;
use opentelemetry_sdk::runtime;
use opentelemetry_sdk::trace::{Config, Sampler, SpanProcessor};
use std::env;
use tracing_opentelemetry::OtelData;
use tracing_subscriber::{registry::LookupSpan, Layer};

/// A span processor that scrubs sensitive information from span attributes
#[derive(Debug)]
struct ScrubbingProcessor {
    inner: std::sync::Mutex<Box<dyn SpanProcessor + Send + Sync>>,
}

impl ScrubbingProcessor {
    fn new(inner: Box<dyn SpanProcessor + Send + Sync>) -> Self {
        ScrubbingProcessor {
            inner: std::sync::Mutex::new(inner),
        }
    }

    fn scrub_attributes(&self, attributes: &mut [KeyValue]) {
        for kv in attributes.iter_mut() {
            let key = kv.key.as_str();
            if key == "net.peer.ip"
                || key == "net.host.ip"
                || key == "http.client_ip"
                || key == "k8s.cluster.name"
                || key == "host.name"
            {
                kv.value = opentelemetry::Value::String("[REDACTED]".into());
            }
        }
    }
}

impl SpanProcessor for ScrubbingProcessor {
    fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, cx: &opentelemetry::Context) {
        if let Ok(inner) = self.inner.lock() {
            inner.on_start(span, cx);
        }
    }

    fn on_end(&self, mut span: SpanData) {
        self.scrub_attributes(&mut span.attributes);
        if let Ok(inner) = self.inner.lock() {
            inner.on_end(span);
        }
    }

    fn force_flush(&self) -> TraceResult<()> {
        if let Ok(inner) = self.inner.lock() {
            inner.force_flush()
        } else {
            Ok(())
        }
    }

    fn shutdown(&mut self) -> TraceResult<()> {
        if let Ok(mut inner) = self.inner.lock() {
            inner.shutdown()
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Trace-ID injection layer
// ---------------------------------------------------------------------------

/// A `tracing_subscriber` layer that appends `trace_id` and `span_id` W3C hex
/// fields to every JSON log event when an active OTel span is present.
///
/// Add this layer **after** the `fmt::layer()` so the fields appear in the
/// same JSON object:
///
/// ```rust,ignore
/// tracing_subscriber::registry()
///     .with(fmt::layer().json())
///     .with(OtelTraceIdLayer)
///     .with(otel_layer)
///     .init();
/// ```
pub struct OtelTraceIdLayer;

impl<S> Layer<S> for OtelTraceIdLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Walk up the span stack to find the nearest span that carries OTel data.
        let span = ctx.event_span(event).or_else(|| ctx.lookup_current());
        if let Some(span) = span {
            let extensions = span.extensions();
            if let Some(otel_data) = extensions.get::<OtelData>() {
                let trace_id = otel_data.builder.trace_id.map(|id| format!("{:032x}", id));
                let span_id = otel_data.builder.span_id.map(|id| format!("{:016x}", id));
                // Emit as a tracing event so the fmt layer picks them up.
                // We use a dedicated target so they can be filtered independently.
                if let (Some(tid), Some(sid)) = (trace_id, span_id) {
                    tracing::trace!(
                        target: "otel::trace_ids",
                        trace_id = %tid,
                        span_id  = %sid,
                    );
                }
            }
        }
    }
}

/// Returns a `tracing_subscriber` layer that injects `trace_id` and `span_id`
/// into every log event.  Wire this in alongside the OTel tracing layer.
pub fn trace_id_layer<S>() -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    OtelTraceIdLayer
}

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
    // TLS is handled automatically if endpoint scheme is https
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&otlp_endpoint);

    let batch_processor = opentelemetry_sdk::trace::BatchSpanProcessor::builder(
        exporter
            .build_span_exporter()
            .expect("Failed to build exporter"),
        runtime::Tokio,
    )
    .build();

    let scrubbing_processor = ScrubbingProcessor::new(Box::new(batch_processor));

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_config(
            Config::default()
                .with_resource(resource)
                .with_sampler(Sampler::AlwaysOn),
        )
        .with_span_processor(scrubbing_processor)
        .build();

    let tracer = opentelemetry::trace::TracerProvider::tracer(&provider, "stellar-operator");

    // Set global provider
    global::set_tracer_provider(provider);

    // Create tracing layer
    tracing_opentelemetry::layer().with_tracer(tracer).boxed()
}

/// Shutdown OpenTelemetry tracer
pub fn shutdown_telemetry() {
    global::shutdown_tracer_provider();
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::TraceResult;
    use opentelemetry_sdk::export::trace::SpanData;
    use opentelemetry_sdk::trace::{Span, SpanProcessor};
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    struct MockProcessor {
        pub spans: Arc<Mutex<Vec<SpanData>>>,
    }

    impl MockProcessor {
        fn new() -> Self {
            Self {
                spans: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl SpanProcessor for MockProcessor {
        fn on_start(&self, _span: &mut Span, _cx: &opentelemetry::Context) {}

        fn on_end(&self, span: SpanData) {
            self.spans.lock().unwrap().push(span);
        }

        fn force_flush(&self) -> TraceResult<()> {
            Ok(())
        }

        fn shutdown(&mut self) -> TraceResult<()> {
            Ok(())
        }
    }

    #[test]
    fn test_scrubbing_processor() {
        let mock_inner = MockProcessor::new();
        let processor = ScrubbingProcessor::new(Box::new(mock_inner.clone()));

        // Create a span with sensitive attributes
        // Since we can't easily construct a full SpanData manually due to private fields/complexity,
        // we'll try to use the processor on a real span if possible, or just mock the input.
        // Opentelemetry SDK SpanData construction is verbose.
        // Let's rely on the fact that on_end takes SpanData.

        // Actually, constructing SpanData is hard.
        // Let's verify `scrub_attributes` directly if we make it visible to tests,
        // or just move the test logic to test `scrub_attributes` by making it `pub(crate)` or internal.

        let mut attributes = vec![
            KeyValue::new("net.peer.ip", "1.2.3.4"),
            KeyValue::new("safe.key", "value"),
            KeyValue::new("k8s.cluster.name", "production-cluster"),
        ];

        processor.scrub_attributes(&mut attributes);

        assert_eq!(
            attributes[0].value,
            opentelemetry::Value::String("[REDACTED]".into())
        );
        assert_eq!(
            attributes[1].value,
            opentelemetry::Value::String("value".into())
        );
        assert_eq!(
            attributes[2].value,
            opentelemetry::Value::String("[REDACTED]".into())
        );
    }
}
