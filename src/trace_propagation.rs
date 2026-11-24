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
//! Standardized Trace Propagation Across Async Messaging Boundaries
//!
//! Carries W3C trace context (`traceparent` / `tracestate`) through message
//! headers so async request paths stitch into a single distributed trace.
//!
//! # Design
//!
//! - [`inject_context`] / [`extract_context`] work on a plain header map, so
//!   every transport only needs a thin shim that maps its own header type.
//! - Shims wrap the existing producer/consumer API instead of replacing it:
//!   [`TracedMessageQueue`] for [`crate::message_queue`] and [`kafka`] for
//!   rdkafka record headers. HTTP is already covered by
//!   [`crate::telemetry::inject_trace_headers`].
//! - Propagation never blocks delivery. A consumer that receives no context,
//!   or a malformed one, starts a new trace and reports a broken chain
//!   (warning log plus `stellar_trace_context_extractions{result!="stitched"}`
//!   with transport, queue and topic labels).
//! - `tracestate` is dropped when it would push the added headers over
//!   [`MAX_HEADER_OVERHEAD_BYTES`]; `traceparent` alone is 66 bytes.
//!
//! ## Acceptance Criteria (from #1971)
//! - End-to-end trace stitch rate above 95% for async paths (stitch rate is
//!   `stitched / all` on `stellar_trace_context_extractions`)
//! - Broken-chain events exported with queue + topic labels
//! - Added header overhead under 200 bytes ([`header_overhead`])
//! - No measurable throughput regression (header map writes only; no I/O)

use std::collections::HashMap;

use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry::Context;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing::{warn, Instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::message_queue::{Message, SharedMQ};

pub const TRACEPARENT: &str = "traceparent";
pub const TRACESTATE: &str = "tracestate";

/// Upper bound on the bytes trace propagation adds to a message.
pub const MAX_HEADER_OVERHEAD_BYTES: usize = 200;

/// Transport label for [`crate::message_queue`].
pub const TRANSPORT_QUEUE: &str = "queue";
/// Transport label for Kafka.
pub const TRANSPORT_KAFKA: &str = "kafka";

/// Why a consumer could not continue the producer's trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokenChainReason {
    Missing,
    Malformed,
}

impl BrokenChainReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            BrokenChainReason::Missing => "missing",
            BrokenChainReason::Malformed => "malformed",
        }
    }
}

/// Writes propagation headers into a message header map.
pub struct MapInjector<'a>(pub &'a mut HashMap<String, String>);

impl Injector for MapInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_ascii_lowercase(), value);
    }
}

/// Reads propagation headers from a message header map (case-insensitive).
pub struct MapExtractor<'a>(pub &'a HashMap<String, String>);

impl Extractor for MapExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .get(key)
            .or_else(|| {
                self.0
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(key))
                    .map(|(_, v)| v)
            })
            .map(String::as_str)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}

/// Injects `cx` as W3C trace context, keeping the overhead within budget.
pub fn inject_context(cx: &Context, headers: &mut HashMap<String, String>) {
    TraceContextPropagator::new().inject_context(cx, &mut MapInjector(headers));
    if header_overhead(headers) > MAX_HEADER_OVERHEAD_BYTES {
        headers.remove(TRACESTATE);
    }
}

/// Bytes taken by the propagation headers (names plus values).
pub fn header_overhead(headers: &HashMap<String, String>) -> usize {
    [TRACEPARENT, TRACESTATE]
        .iter()
        .filter_map(|k| headers.get(*k).map(|v| k.len() + v.len()))
        .sum()
}

/// Extracts the producer's trace context. On a missing or malformed
/// `traceparent` the broken chain is logged and counted, and the caller
/// should start a new trace.
pub fn extract_context(
    headers: &HashMap<String, String>,
    transport: &str,
    queue: &str,
    topic: &str,
) -> Result<Context, BrokenChainReason> {
    let extractor = MapExtractor(headers);
    let result = match extractor.get(TRACEPARENT) {
        None => Err(BrokenChainReason::Missing),
        Some(value) if !is_valid_traceparent(value) => Err(BrokenChainReason::Malformed),
        Some(_) => Ok(TraceContextPropagator::new().extract(&extractor)),
    };

    if let Err(reason) = result {
        warn!(
            transport,
            queue,
            topic,
            reason = reason.as_str(),
            "Broken trace chain: message consumed without a valid traceparent"
        );
    }
    #[cfg(feature = "metrics")]
    crate::controller::metrics::inc_trace_context_extraction(
        transport,
        queue,
        topic,
        result.as_ref().map_or_else(|r| r.as_str(), |_| "stitched"),
    );
    result
}

/// Checks a `traceparent` value against the W3C Trace Context format.
pub fn is_valid_traceparent(value: &str) -> bool {
    fn hex(s: &str, len: usize) -> bool {
        s.len() == len && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
    fn non_zero(s: &str) -> bool {
        s.bytes().any(|b| b != b'0')
    }

    let parts: Vec<&str> = value.trim().split('-').collect();
    let [version, trace_id, parent_id, flags, rest @ ..] = parts.as_slice() else {
        return false;
    };
    hex(version, 2)
        && *version != "ff"
        && (*version != "00" || rest.is_empty())
        && hex(trace_id, 32)
        && non_zero(trace_id)
        && hex(parent_id, 16)
        && non_zero(parent_id)
        && hex(flags, 2)
}

/// [`crate::message_queue`] shim: same enqueue/receive calls, with trace
/// context carried in the message headers.
#[derive(Clone)]
pub struct TracedMessageQueue {
    inner: SharedMQ,
}

impl TracedMessageQueue {
    pub fn new(inner: SharedMQ) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &SharedMQ {
        &self.inner
    }

    /// Enqueues `msg` inside a producer span whose context travels with it.
    pub async fn enqueue(&self, mut msg: Message) -> Result<(), String> {
        let span = tracing::info_span!(
            "queue.publish",
            otel.kind = "producer",
            messaging.destination = %msg.queue,
            messaging.message_id = %msg.id,
        );
        inject_context(&span.context(), &mut msg.headers);
        async { self.inner.write().await.enqueue(msg) }
            .instrument(span)
            .await
    }

    /// Receives from `queue` and returns the message with a consumer span
    /// that continues the producer's trace (or starts a new one).
    pub async fn receive(&self, queue: &str) -> Option<(Message, Span)> {
        let msg = self.inner.write().await.receive(queue)?;
        let span = tracing::info_span!(
            "queue.process",
            otel.kind = "consumer",
            messaging.destination = %queue,
            messaging.message_id = %msg.id,
        );
        let topic = msg.routing_key.as_deref().unwrap_or_default();
        if let Ok(parent) = extract_context(&msg.headers, TRANSPORT_QUEUE, queue, topic) {
            span.set_parent(parent);
        }
        Some((msg, span))
    }
}

/// rdkafka shim: wraps record headers on produce and consume.
#[cfg(feature = "kafka")]
pub mod kafka {
    use std::collections::HashMap;

    use opentelemetry::Context;
    use rdkafka::message::{Header, Headers, OwnedHeaders};
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    use super::{extract_context, inject_context, BrokenChainReason, TRANSPORT_KAFKA};

    /// Adds the current span's trace context to outgoing record headers.
    pub fn inject_headers(headers: OwnedHeaders) -> OwnedHeaders {
        let mut map = HashMap::new();
        inject_context(&tracing::Span::current().context(), &mut map);
        map.iter().fold(headers, |h, (key, value)| {
            h.insert(Header {
                key: key.as_str(),
                value: Some(value.as_str()),
            })
        })
    }

    /// Extracts the producer's trace context from a consumed record.
    pub fn extract_headers<H: Headers>(
        headers: Option<&H>,
        topic: &str,
    ) -> Result<Context, BrokenChainReason> {
        let mut map = HashMap::new();
        if let Some(headers) = headers {
            for i in 0..headers.count() {
                let header = headers.get(i);
                if let Some(value) = header.value.and_then(|v| std::str::from_utf8(v).ok()) {
                    map.insert(header.key.to_ascii_lowercase(), value.to_string());
                }
            }
        }
        extract_context(&map, TRANSPORT_KAFKA, topic, topic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_queue::{new_shared, QueueConfig};
    use opentelemetry::trace::{
        SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
    };
    use std::str::FromStr;

    const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const SPAN_ID: &str = "00f067aa0ba902b7";

    fn remote_context(state: TraceState) -> Context {
        Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from_hex(TRACE_ID).unwrap(),
            SpanId::from_hex(SPAN_ID).unwrap(),
            TraceFlags::SAMPLED,
            true,
            state,
        ))
    }

    #[test]
    fn round_trips_trace_context_through_headers() {
        let mut headers = HashMap::new();
        inject_context(&remote_context(TraceState::default()), &mut headers);
        assert_eq!(
            headers.get(TRACEPARENT).map(String::as_str),
            Some(format!("00-{TRACE_ID}-{SPAN_ID}-01").as_str())
        );

        let cx = extract_context(&headers, TRANSPORT_QUEUE, "jobs", "ledger").unwrap();
        let span = cx.span();
        let sc = span.span_context();
        assert_eq!(sc.trace_id().to_string(), TRACE_ID);
        assert_eq!(sc.span_id().to_string(), SPAN_ID);
        assert!(sc.is_remote());
    }

    #[test]
    fn header_overhead_stays_within_budget() {
        let mut headers = HashMap::new();
        let state = TraceState::from_str("vendor=abc").unwrap();
        inject_context(&remote_context(state), &mut headers);
        assert!(headers.contains_key(TRACESTATE));
        assert!(header_overhead(&headers) <= MAX_HEADER_OVERHEAD_BYTES);

        let mut headers = HashMap::new();
        let big = TraceState::from_str(&format!("vendor={}", "a".repeat(150))).unwrap();
        inject_context(&remote_context(big), &mut headers);
        assert!(!headers.contains_key(TRACESTATE));
        assert!(headers.contains_key(TRACEPARENT));
        assert!(header_overhead(&headers) <= MAX_HEADER_OVERHEAD_BYTES);
    }

    #[test]
    fn reports_missing_and_malformed_context() {
        let empty = HashMap::new();
        assert_eq!(
            extract_context(&empty, TRANSPORT_QUEUE, "jobs", "ledger").unwrap_err(),
            BrokenChainReason::Missing
        );

        let mut bad = HashMap::new();
        bad.insert("Traceparent".to_string(), "00-not-a-trace-01".to_string());
        assert_eq!(
            extract_context(&bad, TRANSPORT_QUEUE, "jobs", "ledger").unwrap_err(),
            BrokenChainReason::Malformed
        );
    }

    #[test]
    fn validates_traceparent_format() {
        assert!(is_valid_traceparent(&format!("00-{TRACE_ID}-{SPAN_ID}-01")));
        assert!(is_valid_traceparent(&format!("01-{TRACE_ID}-{SPAN_ID}-01-extra")));
        assert!(!is_valid_traceparent(&format!("00-{TRACE_ID}-{SPAN_ID}-01-extra")));
        assert!(!is_valid_traceparent(&format!("ff-{TRACE_ID}-{SPAN_ID}-01")));
        assert!(!is_valid_traceparent(&format!("00-{}-{SPAN_ID}-01", "0".repeat(32))));
        assert!(!is_valid_traceparent(&format!("00-{TRACE_ID}-{}-01", "0".repeat(16))));
        assert!(!is_valid_traceparent(&format!(
            "00-{}-{SPAN_ID}-01",
            TRACE_ID.to_uppercase()
        )));
    }

    #[tokio::test]
    async fn traced_queue_never_blocks_delivery() {
        let mq = new_shared();
        mq.write().await.create_queue(QueueConfig {
            name: "jobs".to_string(),
            ..Default::default()
        });
        let traced = TracedMessageQueue::new(mq);

        // No OpenTelemetry layer is installed, so nothing is injected; the
        // message must still be delivered with a fresh consumer span.
        traced
            .enqueue(Message::new("m1", "jobs", serde_json::json!({"n": 1})))
            .await
            .unwrap();
        let (msg, _span) = traced.receive("jobs").await.unwrap();
        assert_eq!(msg.id, "m1");
    }
}
