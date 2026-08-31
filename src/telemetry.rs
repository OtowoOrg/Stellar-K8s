// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
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
            if is_sensitive_attribute(key) {
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

fn is_sensitive_attribute(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "net.peer.ip"
            | "net.host.ip"
            | "http.client_ip"
            | "k8s.cluster.name"
            | "host.name"
            | "http.request.header.authorization"
            | "http.request.header.cookie"
            | "http.request.header.set-cookie"
            | "http.request.header.x-api-key"
            | "http.request.header.x-auth-token"
            | "authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "x-auth-token"
            | "password"
            | "token"
            | "secret"
            | "private_key"
    ) || key.contains("password")
        || key.contains("authorization")
        || (key.contains("private") && key.contains("key"))
}

thread_local! {
    static TRACE_CONTEXT: std::cell::RefCell<Option<(String, String)>> =
        const { std::cell::RefCell::new(None) };
}

/// Current W3C `trace_id` / `span_id` if a span is active.
pub fn current_trace_context() -> Option<(String, String)> {
    TRACE_CONTEXT.with(|cell| cell.borrow().clone())
}

fn store_trace_context(trace_id: String, span_id: String) {
    TRACE_CONTEXT.with(|cell| {
        *cell.borrow_mut() = Some((trace_id, span_id));
    });
}

/// W3C Trace Context injector for `http::HeaderMap` / reqwest headers.
pub struct HeaderInjector<'a>(pub &'a mut http::HeaderMap);

impl opentelemetry::propagation::Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if is_sensitive_attribute(key) {
            return;
        }
        if let (Ok(name), Ok(val)) = (
            http::HeaderName::from_bytes(key.as_bytes()),
            http::HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, val);
        }
    }
}

/// W3C Trace Context extractor for incoming HTTP headers.
pub struct HeaderExtractor<'a>(pub &'a http::HeaderMap);

impl opentelemetry::propagation::Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

/// Inject the current span context into outbound HTTP headers.
pub fn inject_trace_headers(headers: &mut http::HeaderMap) {
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    let cx = tracing::Span::current().context();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut HeaderInjector(headers));
    });
}

/// Extract a parent context from incoming HTTP headers (W3C `traceparent`).
pub fn extract_parent_context(headers: &http::HeaderMap) -> opentelemetry::Context {
    global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(headers)))
}

fn sampler_from_env() -> Sampler {
    let kind =
        env::var("OTEL_TRACES_SAMPLER").unwrap_or_else(|_| "parentbased_traceidratio".into());
    let ratio = env::var("OTEL_TRACES_SAMPLER_ARG")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    match kind.as_str() {
        "always_on" => Sampler::AlwaysOn,
        "always_off" => Sampler::AlwaysOff,
        "traceidratio" => Sampler::TraceIdRatioBased(ratio),
        _ => Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(ratio))),
    }
}

fn service_name_from_env() -> String {
    env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "stellar-operator".to_string())
}

/// HTTP middleware: join or start a trace, record safe span attributes, never
/// capture Authorization / cookies / bodies.
#[cfg(any(feature = "rest-api", feature = "admission-webhook"))]
pub async fn http_trace_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use tracing::Instrument;
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    let parent = extract_parent_context(request.headers());
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let service = service_name_from_env();

    let span = tracing::info_span!(
        "http.request",
        otel.name = %format!("{method} {path}"),
        otel.kind = "server",
        http.method = %method,
        http.route = %path,
        http.status_code = tracing::field::Empty,
        service.name = %service,
    );
    span.set_parent(parent);

    async move {
        let response = next.run(request).await;
        tracing::Span::current().record("http.status_code", response.status().as_u16());
        response
    }
    .instrument(span)
    .await
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
    fn on_enter(&self, id: &tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        store_from_span(ctx.span(id));
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let span = ctx.event_span(event).or_else(|| ctx.lookup_current());
        store_from_span(span);
    }
}

fn store_from_span<S>(span: Option<tracing_subscriber::registry::SpanRef<'_, S>>)
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    let Some(span) = span else {
        return;
    };
    let extensions = span.extensions();
    if let Some(otel_data) = extensions.get::<OtelData>() {
        let trace_id = otel_data
            .builder
            .trace_id
            .map(|id| hex::encode(id.to_bytes()));
        let span_id = otel_data
            .builder
            .span_id
            .map(|id| hex::encode(id.to_bytes()));
        if let (Some(tid), Some(sid)) = (trace_id, span_id) {
            store_trace_context(tid, sid);
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
    let service_name = service_name_from_env();

    let resource = Resource::new(vec![
        KeyValue::new("service.name", service_name.clone()),
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
                .with_sampler(sampler_from_env()),
        )
        .with_span_processor(scrubbing_processor)
        .build();

    let tracer = opentelemetry::trace::TracerProvider::tracer(&provider, service_name);

    // Set global provider
    global::set_tracer_provider(provider);

    // Create tracing layer
    tracing_opentelemetry::layer().with_tracer(tracer).boxed()
}

/// Shutdown OpenTelemetry tracer
pub fn shutdown_telemetry() {
    global::shutdown_tracer_provider();
}

/// In-memory span capture used by propagation tests (no OTLP, no Jaeger).
#[derive(Clone, Debug)]
pub struct CapturedSpan {
    pub name: String,
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
}

#[derive(Debug, Clone)]
struct CapturingProcessor {
    spans: std::sync::Arc<std::sync::Mutex<Vec<CapturedSpan>>>,
}

impl SpanProcessor for CapturingProcessor {
    fn on_start(&self, _span: &mut opentelemetry_sdk::trace::Span, _cx: &opentelemetry::Context) {}

    fn on_end(&self, span: SpanData) {
        let parent = span.parent_span_id;
        let parent_span_id = if parent == opentelemetry::trace::SpanId::INVALID {
            None
        } else {
            Some(hex::encode(parent.to_bytes()))
        };
        self.spans.lock().unwrap().push(CapturedSpan {
            name: span.name.to_string(),
            trace_id: hex::encode(span.span_context.trace_id().to_bytes()),
            span_id: hex::encode(span.span_context.span_id().to_bytes()),
            parent_span_id,
        });
    }

    fn force_flush(&self) -> TraceResult<()> {
        Ok(())
    }

    fn shutdown(&mut self) -> TraceResult<()> {
        Ok(())
    }
}

/// Install a capturing tracer provider + tracing-opentelemetry layer for tests.
pub fn init_capturing_tracer<S>() -> (
    Box<dyn Layer<S> + Send + Sync>,
    std::sync::Arc<std::sync::Mutex<Vec<CapturedSpan>>>,
)
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    global::set_text_map_propagator(TraceContextPropagator::new());
    let spans = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let processor = CapturingProcessor {
        spans: spans.clone(),
    };
    let resource = Resource::new(vec![KeyValue::new("service.name", "stellar-test")]);
    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_config(
            Config::default()
                .with_resource(resource)
                .with_sampler(Sampler::AlwaysOn),
        )
        .with_span_processor(processor)
        .build();
    let tracer = opentelemetry::trace::TracerProvider::tracer(&provider, "stellar-test");
    global::set_tracer_provider(provider);
    (
        tracing_opentelemetry::layer().with_tracer(tracer).boxed(),
        spans,
    )
}

/// Ensure W3C propagation is registered without talking to an OTLP collector.
pub fn install_w3c_propagator() {
    global::set_text_map_propagator(TraceContextPropagator::new());
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
            KeyValue::new("authorization", "Bearer secret-token"),
            KeyValue::new("http.method", "GET"),
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
        assert_eq!(
            attributes[3].value,
            opentelemetry::Value::String("[REDACTED]".into())
        );
        assert_eq!(
            attributes[4].value,
            opentelemetry::Value::String("GET".into())
        );
    }

    #[test]
    fn sampler_from_env_parent_based_default() {
        std::env::remove_var("OTEL_TRACES_SAMPLER");
        std::env::remove_var("OTEL_TRACES_SAMPLER_ARG");
        match sampler_from_env() {
            Sampler::ParentBased(_) => {}
            _ => panic!("expected ParentBased sampler"),
        }
    }

    #[test]
    fn w3c_header_round_trip_does_not_write_authorization() {
        use opentelemetry::propagation::TextMapPropagator;
        use opentelemetry::trace::{
            SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
        };

        install_w3c_propagator();
        let trace_id = TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        let span_id = SpanId::from_hex("00f067aa0ba902b7").unwrap();
        let sc = SpanContext::new(
            trace_id,
            span_id,
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        );
        let cx = opentelemetry::Context::new().with_remote_span_context(sc);
        let mut headers = http::HeaderMap::new();
        TraceContextPropagator::new().inject_context(&cx, &mut HeaderInjector(&mut headers));
        assert!(headers.get("traceparent").is_some());
        assert!(headers.get("authorization").is_none());
        let extracted = extract_parent_context(&headers);
        assert_eq!(extracted.span().span_context().trace_id(), trace_id);
    }
}
