//! OpenTelemetry initialization and utilities
//!
//! Provides functions to set up distributed tracing with OTLP export.

use opentelemetry::trace::TraceResult;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::export::trace::SpanData;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::resource::Resource;
use opentelemetry_sdk::runtime;
use opentelemetry_sdk::trace::{Config, Sampler, SpanProcessor};
use std::env;
use std::sync::Arc;
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

    fn scrub_attributes(&self, attributes: &mut Vec<KeyValue>) {
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
        if let Ok(mut inner) = self.inner.lock() {
            inner.on_start(span, cx);
        }
    }

    fn on_end(&self, mut span: SpanData) {
        self.scrub_attributes(&mut span.attributes);
        if let Ok(mut inner) = self.inner.lock() {
            inner.on_end(span);
        }
    }

    fn force_flush(&self) -> TraceResult<()> {
        if let Ok(mut inner) = self.inner.lock() {
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
