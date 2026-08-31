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
//! Correlation ID middleware — propagated across service boundaries
//!
//! Every inbound request gets a correlation ID:
//! - If client supplied `X-Correlation-ID` or `X-Request-ID`, reuse it
//! - Otherwise generate a `req-` prefixed ULID-like ID
//! The ID is:
//! - Stored in `request.extensions()` as `CorrelationId`
//! - Added to response headers `X-Correlation-ID`
//! - Injected into the current tracing span as `correlation_id` (see `logging::fields`)
//! Downstream HTTP calls MUST forward this header for end-to-end tracing.

use axum::{
    body::Body,
    http::{HeaderName, HeaderValue, Request, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// Typed correlation ID stored in request extensions
#[derive(Clone, Debug)]
pub struct CorrelationId(pub String);

impl CorrelationId {
    pub fn new(id: String) -> Self {
        Self(id)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn generate_id() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let ctr = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("req-{}-{:06}", ts, ctr % 1_000_000)
}

/// Extract correlation ID from request or generate a new one
pub fn extract_or_generate(req: &Request<Body>) -> String {
    const HEADER_CANDIDATES: &[&str] = &["x-correlation-id", "x-request-id", "x-trace-id"];
    for name in HEADER_CANDIDATES {
        if let Some(v) = req.headers().get(*name).and_then(|h| h.to_str().ok()) {
            let trimmed = v.trim();
            if !trimmed.is_empty() && trimmed.len() <= 128 {
                return trimmed.to_string();
            }
        }
    }
    generate_id()
}

pub async fn correlation_middleware(mut req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    let cid = extract_or_generate(&req);
    // Store for handlers
    req.extensions_mut().insert(CorrelationId(cid.clone()));
    // Run handler with tracing span enriched
    let span = tracing::Span::current();
    span.record("correlation_id", &cid.as_str());
    // Use fields constant for consistency
    tracing::trace!(correlation_id = %cid, "incoming request");

    let mut res = next.run(req).await;
    // Echo correlation ID in response
    if let Ok(val) = HeaderValue::from_str(&cid) {
        res.headers_mut().insert(
            HeaderName::from_static("x-correlation-id"),
            val,
        );
    }
    // Structured logging: include correlation_id in JSON logs via tracing field
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_id_is_unique() {
        let a = generate_id();
        let b = generate_id();
        assert_ne!(a, b);
        assert!(a.starts_with("req-"));
    }

    #[test]
    fn extract_uses_header_when_present() {
        let req = Request::builder()
            .header("X-Correlation-ID", "test-corr-123")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_or_generate(&req), "test-corr-123");
    }

    #[test]
    fn extract_generates_when_missing() {
        let req = Request::builder().body(Body::empty()).unwrap();
        let id = extract_or_generate(&req);
        assert!(id.starts_with("req-"));
    }
}
