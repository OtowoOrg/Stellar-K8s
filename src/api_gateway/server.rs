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
//! API gateway server: Axum-based HTTP server wiring all gateway components.

use crate::api_gateway::{
    analytics::AnalyticsStore,
    auth::ApiKeyStore,
    config::GatewayConfig,
    router::{MatchResult, Router},
    transform::{transform_request, transform_response},
    versioning::{check_version, deprecation_headers, VersionStatus},
};
use crate::error::{ApiErrorCode, ErrorResponse};
use crate::telemetry::CorrelationId;
use axum::{
    extract::{Request, State},
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Json, Router as AxumRouter,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Build a structured `ErrorResponse` JSON body (issue #1393), mirroring the
/// envelope `rest_api` handlers use so gateway and operator errors are
/// indistinguishable in shape to API consumers. The correlation ID (set by
/// `telemetry::http_trace_middleware`, which wraps the whole gateway router)
/// is echoed into the body here and is also present as an `X-Correlation-Id`
/// response header added by that same middleware.
fn error_response(
    status: StatusCode,
    code: ApiErrorCode,
    message: impl Into<String>,
    correlation_id: Option<&CorrelationId>,
) -> Response {
    (
        status,
        Json(ErrorResponse::structured(
            code,
            &message.into(),
            correlation_id.map(|c| c.to_string()),
        )),
    )
        .into_response()
}

// ── Rate limiter ──────────────────────────────────────────────────────────────

/// Simple token-bucket rate limiter per API key.
#[derive(Default)]
struct RateLimiter {
    /// key_id → (tokens, last_refill_secs)
    buckets: RwLock<HashMap<String, (f64, u64)>>,
}

impl RateLimiter {
    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Returns `true` if the request is allowed.
    async fn check(&self, key_id: &str, rps: f64, burst: f64) -> bool {
        let now = Self::now_secs();
        let mut buckets = self.buckets.write().await;
        let entry = buckets.entry(key_id.to_string()).or_insert((burst, now));
        let elapsed = now.saturating_sub(entry.1) as f64;
        entry.0 = (entry.0 + elapsed * rps).min(burst);
        entry.1 = now;
        if entry.0 >= 1.0 {
            entry.0 -= 1.0;
            true
        } else {
            false
        }
    }
}

// ── Shared gateway state ──────────────────────────────────────────────────────

#[derive(Clone)]
struct GatewayState {
    router: Arc<Router>,
    key_store: ApiKeyStore,
    analytics: AnalyticsStore,
    rate_limiter: Arc<RateLimiter>,
    config: Arc<GatewayConfig>,
    http_client: reqwest::Client,
}

// ── ApiGateway ────────────────────────────────────────────────────────────────

/// The API gateway.
pub struct ApiGateway {
    config: GatewayConfig,
    key_store: ApiKeyStore,
}

impl ApiGateway {
    pub fn new(config: GatewayConfig) -> Self {
        Self {
            config,
            key_store: ApiKeyStore::new(),
        }
    }

    /// Access the key store to pre-populate API keys.
    pub fn key_store(&self) -> &ApiKeyStore {
        &self.key_store
    }

    /// Build and return the Axum router (useful for embedding in an existing server).
    pub fn into_router(self) -> AxumRouter {
        let state = GatewayState {
            router: Arc::new(Router::new(
                self.config.routes.clone(),
                self.config.versioning.clone(),
            )),
            key_store: self.key_store,
            analytics: AnalyticsStore::new(50_000),
            rate_limiter: Arc::new(RateLimiter::default()),
            config: Arc::new(self.config),
            http_client: reqwest::Client::new(),
        };

        AxumRouter::new()
            .route("/{*path}", any(handle_request))
            .route("/", any(handle_request))
            // Management endpoints
            .route("/_gateway/keys", axum::routing::get(list_keys))
            .route("/_gateway/analytics", axum::routing::get(get_analytics))
            .layer(axum::middleware::from_fn(
                crate::telemetry::http_trace_middleware,
            ))
            .with_state(state)
    }

    /// Start the gateway on the configured bind address.
    pub async fn serve(self) -> Result<(), crate::error::Error> {
        let addr = self.config.bind_addr.clone();
        let app = self.into_router();
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| crate::error::Error::ConfigError(e.to_string()))?;
        info!(addr, "API gateway listening");
        axum::serve(listener, app)
            .await
            .map_err(|e| crate::error::Error::ConfigError(e.to_string()))
    }
}

// ── Request handler ───────────────────────────────────────────────────────────

async fn handle_request(State(state): State<GatewayState>, req: Request) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    // Set by `telemetry::http_trace_middleware`, which wraps this whole
    // router (issue #1393); used to populate `ErrorResponse::correlation_id`.
    let correlation_id = req.extensions().get::<CorrelationId>().cloned();

    // Extract headers as a plain map for routing predicates
    let headers: HashMap<String, String> = req
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_string(), v.to_string()))
        })
        .collect();

    // API key authentication
    let raw_key = headers
        .get("x-api-key")
        .or_else(|| headers.get("authorization"))
        .cloned()
        .unwrap_or_default();
    let raw_key = raw_key.trim_start_matches("Bearer ").to_string();

    let api_key = state.key_store.authenticate(&raw_key).await;
    if api_key.is_none() && !raw_key.is_empty() {
        return error_response(
            StatusCode::UNAUTHORIZED,
            ApiErrorCode::ErrUnauthorized,
            "Invalid API key",
            correlation_id.as_ref(),
        );
    }
    let key_id = api_key.as_ref().map(|k| k.id.clone());

    // Rate limiting
    if let Some(key) = &api_key {
        let rps = state
            .config
            .rate_limiting
            .key_overrides
            .get(&key.id)
            .copied()
            .unwrap_or(state.config.rate_limiting.default_rps) as f64;
        let burst = state.config.rate_limiting.default_burst as f64;
        if !state.rate_limiter.check(&key.id, rps, burst).await {
            warn!(key_id = %key.id, "rate limit exceeded");
            return error_response(
                StatusCode::TOO_MANY_REQUESTS,
                ApiErrorCode::ErrRateLimited,
                "Rate limit exceeded",
                correlation_id.as_ref(),
            );
        }
    }

    // Route matching
    let match_result = state.router.match_route(&path, &method, &headers);
    let route_match = match match_result {
        MatchResult::NotFound => {
            return error_response(
                StatusCode::NOT_FOUND,
                ApiErrorCode::ErrNotFound,
                "No matching route",
                correlation_id.as_ref(),
            );
        }
        MatchResult::Sunset(version) => {
            return error_response(
                StatusCode::GONE,
                ApiErrorCode::ErrGone,
                format!("API version {version} has been sunset"),
                correlation_id.as_ref(),
            );
        }
        MatchResult::Matched(m) => m,
    };

    let route = route_match.route;

    // Version deprecation headers
    let version_status = check_version(&route.version, &state.config.versioning);
    let mut extra_headers: Vec<(String, String)> = vec![];
    if let VersionStatus::Deprecated { sunset_date } = &version_status {
        extra_headers.extend(deprecation_headers(
            route.sunset_date.as_deref().or(sunset_date.as_deref()),
            &state.config.versioning.current_version,
        ));
    }

    // Read request body
    let (_parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ApiErrorCode::ErrBadRequest,
                "Failed to read body",
                correlation_id.as_ref(),
            )
        }
    };

    // Protocol transformation (client → upstream)
    let client_protocol = &crate::api_gateway::config::Protocol::Rest; // inferred from Content-Type in production
    let normalized = match transform_request(&body_bytes, client_protocol, &route.protocol) {
        Ok(n) => n,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                ApiErrorCode::ErrBadRequest,
                e.to_string(),
                correlation_id.as_ref(),
            );
        }
    };

    // Proxy to upstream
    let upstream_url = format!("{}{}", route.upstream, route_match.remaining_path);
    let mut headers = reqwest::header::HeaderMap::new();
    crate::telemetry::inject_trace_headers(&mut headers);
    if let Ok(value) = reqwest::header::HeaderValue::from_str(&normalized.content_type) {
        headers.insert(reqwest::header::CONTENT_TYPE, value);
    }
    let upstream_resp = state
        .http_client
        .request(
            reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
            &upstream_url,
        )
        .headers(headers)
        .body(serde_json::to_vec(&normalized.body).unwrap_or_default())
        .send()
        .await;

    let (status_code, resp_body) = match upstream_resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.bytes().await.unwrap_or_default();
            (status, body.to_vec())
        }
        Err(e) => {
            warn!(upstream = %upstream_url, error = %e, "upstream request failed");
            // Graceful degradation (#1393): the gateway keeps no response
            // cache, so there is no stale/last-known-good body to fall back
            // to here — this is a genuine full outage for this route, not a
            // partial result, so `degraded` stays `false`. Still report it
            // through the shared structured envelope (rather than a bare
            // string) so `ERR_SERVICE_UNAVAILABLE` is machine-readable and
            // consistent with every other gateway/rest_api error.
            return error_response(
                StatusCode::BAD_GATEWAY,
                ApiErrorCode::ErrServiceUnavailable,
                format!("Upstream request failed: {e}"),
                correlation_id.as_ref(),
            );
        }
    };

    // Protocol transformation (upstream → client)
    let normalized_resp =
        match transform_response(&resp_body, &route.protocol, client_protocol, status_code) {
            Ok(r) => r,
            Err(e) => {
                // Graceful degradation (#1393): unlike the upstream-request
                // failure above, we *did* get a response body back from
                // upstream — only the reshape into the client's expected
                // protocol failed. Discarding a response we actually
                // received would be strictly worse than handing it back
                // as-is, so surface it as a degraded (not hard-failed)
                // result with the raw upstream body attached.
                warn!(
                    upstream = %upstream_url,
                    error = %e,
                    "response transform failed; returning degraded raw upstream body"
                );
                return (
                    StatusCode::from_u16(status_code).unwrap_or(StatusCode::BAD_GATEWAY),
                    Json(ErrorResponse::degraded(
                        ApiErrorCode::ErrPartialDegradation,
                        &format!("Failed to transform upstream response: {e}"),
                        serde_json::json!({
                            "upstreamStatus": status_code,
                            "rawUpstreamBody": String::from_utf8_lossy(&resp_body),
                        }),
                        correlation_id.as_ref().map(|c| c.to_string()),
                    )),
                )
                    .into_response();
            }
        };

    // Analytics
    if state.config.analytics_enabled {
        state
            .analytics
            .record(
                &route.id,
                &method,
                &path,
                status_code,
                start.elapsed(),
                key_id,
                &route.version,
            )
            .await;
    }

    // Build response
    let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (
        status,
        serde_json::to_vec(&normalized_resp.body).unwrap_or_default(),
    )
        .into_response();

    for (k, v) in &extra_headers {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            response.headers_mut().insert(name, val);
        }
    }

    response
}

// ── Management handlers ───────────────────────────────────────────────────────

async fn list_keys(State(state): State<GatewayState>) -> impl IntoResponse {
    let keys = state.key_store.list().await;
    axum::Json(keys)
}

async fn get_analytics(State(state): State<GatewayState>) -> impl IntoResponse {
    let stats = state.analytics.stats_snapshot().await;
    axum::Json(stats)
}
