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
//! Cross-service OpenTelemetry propagation (issue #1290).
//!
//! Builds three in-process HTTP services (A → B → C), sends one request to A,
//! and asserts a single W3C trace id plus parent/child span relationships.
//! Logs recorded during the request must contain that trace id.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use opentelemetry::trace::TraceContextExt;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use stellar_k8s::telemetry::{
    http_trace_middleware, init_capturing_tracer, inject_trace_headers, trace_id_layer,
    CapturedSpan,
};
use tower::util::ServiceExt;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::prelude::*;

fn current_ids() -> (String, String) {
    let sc = tracing::Span::current()
        .context()
        .span()
        .span_context()
        .clone();
    (
        to_hex(&sc.trace_id().to_bytes()),
        to_hex(&sc.span_id().to_bytes()),
    )
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone)]
struct Downstream(Router);

async fn service_c() -> impl IntoResponse {
    let (trace_id, span_id) = current_ids();
    tracing::info!(service = "c", trace_id = %trace_id, span_id = %span_id, "service C handled request");
    format!("c:{trace_id}")
}

async fn service_b(State(Downstream(mut next)): State<Downstream>) -> impl IntoResponse {
    let (trace_id, span_id) = current_ids();
    tracing::info!(service = "b", trace_id = %trace_id, span_id = %span_id, "service B calling C");
    let mut headers = axum::http::HeaderMap::new();
    inject_trace_headers(&mut headers);
    let mut req = Request::builder()
        .method("GET")
        .uri("/c")
        .body(Body::empty())
        .unwrap();
    *req.headers_mut() = headers;
    let response = next.oneshot(req).await.expect("call C");
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    format!("b:{trace_id}|{}", String::from_utf8_lossy(&body))
}

async fn service_a(State(Downstream(mut next)): State<Downstream>) -> impl IntoResponse {
    let (trace_id, span_id) = current_ids();
    tracing::info!(service = "a", trace_id = %trace_id, span_id = %span_id, "service A calling B");
    let mut headers = axum::http::HeaderMap::new();
    inject_trace_headers(&mut headers);
    let mut req = Request::builder()
        .method("GET")
        .uri("/b")
        .body(Body::empty())
        .unwrap();
    *req.headers_mut() = headers;
    let response = next.oneshot(req).await.expect("call B");
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    format!("a:{trace_id}|{}", String::from_utf8_lossy(&body))
}

struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;
    fn make_writer(&'a self) -> Self::Writer {
        LogBuffer(self.0.clone())
    }
}

fn traced(router: Router) -> Router {
    router.layer(axum::middleware::from_fn(http_trace_middleware))
}

#[tokio::test]
async fn trace_context_propagates_a_b_c_and_appears_in_logs() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let (otel_layer, spans) = init_capturing_tracer();
    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(LogBuffer(logs.clone()))
                .with_ansi(false),
        )
        .with(otel_layer)
        .with(trace_id_layer());
    let _guard = tracing::subscriber::set_default(subscriber);

    let svc_c = traced(Router::new().route("/c", get(service_c)));
    let svc_b = traced(
        Router::new()
            .route("/b", get(service_b))
            .with_state(Downstream(svc_c)),
    );
    let mut svc_a = traced(
        Router::new()
            .route("/a", get(service_a))
            .with_state(Downstream(svc_b)),
    );

    let request = Request::builder()
        .method("GET")
        .uri("/a")
        .body(Body::empty())
        .unwrap();
    let response = svc_a.oneshot(request).await.expect("call A");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);

    opentelemetry::global::shutdown_tracer_provider();

    let captured: Vec<CapturedSpan> = spans.lock().unwrap().clone();
    assert!(
        captured.len() >= 3,
        "expected at least 3 spans (A, B, C), got {}: {captured:?}",
        captured.len()
    );

    let http_spans: Vec<_> = captured
        .iter()
        .filter(|s| s.name.contains("http.request") || s.name.contains("GET"))
        .cloned()
        .collect();
    let http_spans = if http_spans.len() >= 3 {
        http_spans
    } else {
        captured.clone()
    };

    let trace_ids: std::collections::BTreeSet<_> =
        http_spans.iter().map(|s| s.trace_id.clone()).collect();
    assert_eq!(
        trace_ids.len(),
        1,
        "A→B→C must share one trace id, got {trace_ids:?} spans={http_spans:?}"
    );
    let trace_id = trace_ids.iter().next().unwrap().clone();
    assert_eq!(trace_id.len(), 32, "W3C trace id must be 32 hex chars");
    assert!(
        body.contains(&trace_id),
        "response should echo the trace id: {body}"
    );

    let ids: std::collections::BTreeSet<_> = http_spans.iter().map(|s| s.span_id.clone()).collect();
    let children: Vec<_> = http_spans
        .iter()
        .filter(|s| s.parent_span_id.is_some())
        .collect();
    assert!(
        !children.is_empty(),
        "expected child spans with parent ids, got {http_spans:?}"
    );
    for child in &children {
        let parent = child.parent_span_id.as_ref().unwrap();
        assert!(
            ids.contains(parent) || http_spans.iter().any(|s| s.span_id == *parent),
            "child {} parent {parent} not in trace {http_spans:?}",
            child.span_id
        );
    }

    let log_text = String::from_utf8_lossy(&logs.lock().unwrap()).into_owned();
    assert!(
        log_text.contains(&trace_id),
        "logs must contain trace_id {trace_id}: {log_text}"
    );
    for service in ["service A", "service B", "service C"] {
        assert!(
            log_text.to_lowercase().contains(&service.to_lowercase())
                || log_text.contains(match service {
                    "service A" => "service A calling B",
                    "service B" => "service B calling C",
                    _ => "service C handled request",
                }),
            "missing {service} log line in: {log_text}"
        );
    }
}

#[test]
fn w3c_headers_round_trip() {
    use opentelemetry::propagation::TextMapPropagator;
    use opentelemetry::trace::{
        SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
    };
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use stellar_k8s::telemetry::{extract_parent_context, HeaderInjector};

    stellar_k8s::telemetry::install_w3c_propagator();
    let trace_id = TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").expect("trace id");
    let span_id = SpanId::from_hex("00f067aa0ba902b7").expect("span id");
    let sc = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );
    let cx = opentelemetry::Context::new().with_remote_span_context(sc);

    let mut headers = axum::http::HeaderMap::new();
    TraceContextPropagator::new().inject_context(&cx, &mut HeaderInjector(&mut headers));
    assert!(
        headers.get("traceparent").is_some(),
        "traceparent must be injected"
    );
    assert!(
        headers.get("authorization").is_none(),
        "must not inject authorization"
    );

    let extracted = extract_parent_context(&headers);
    let got = extracted.span().span_context().clone();
    assert_eq!(got.trace_id(), trace_id);
    assert_eq!(got.span_id(), span_id);
}
