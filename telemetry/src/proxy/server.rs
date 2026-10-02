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
//! Axum HTTP server that wires together the rate-limiter and filter engines.
//!
//! # Endpoints
//!
//! | Method | Path            | Description |
//! |--------|-----------------|-------------|
//! | GET    | `/metrics`      | Filtered, rate-limited Prometheus metrics |
//! | GET    | `/healthz`      | Liveness probe — always `200 OK` |
//! | GET    | `/readyz`       | Readiness probe — `200` once first scrape succeeds |
//! | GET    | `/proxy-stats`  | JSON cache/rate-limiter statistics |
//!
//! # Request flow
//!
//! ```text
//!  Prometheus scrape ──► GET /metrics
//!                              │
//!                     RateLimiter::check()
//!                    ┌─────────┴──────────┐
//!              UseCached               FetchFresh
//!                    │                     │
//!              return cached        GET upstream/metrics
//!                    │                     │
//!                    │               MetricsFilter::apply()
//!                    │                     │
//!                    │           RateLimiter::store_full_response()
//!                    │                     │
//!                    └──────────┬──────────┘
//!                            200 OK
//!                     text/plain; version=0.0.4
//! ```
//!
//! # Latency
//!
//! The hot path (rate-limit hit → cached response) takes < 1 ms: one async
//! mutex acquisition, a string clone, and response serialisation.  The cold
//! path (upstream fetch) is bounded by network latency to the Stellar Core pod
//! (typically < 2 ms within the same K8s node) plus filter processing.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{error, info, instrument, warn};

use crate::proxy::config::ProxyConfig;
use crate::proxy::filter::MetricsFilter;
use crate::proxy::limiter::{RateLimiter, ScrapeDecision};

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

/// State shared across all request handlers.
pub struct ProxyState {
    /// Configuration loaded from ConfigMap.
    pub config: ProxyConfig,
    /// Pre-compiled filter engine.
    pub filter: MetricsFilter,
    /// Rate-limiter and per-family cache engine.
    pub limiter: RateLimiter,
    /// HTTP client for upstream scrapes.
    pub client: reqwest::Client,
    /// Whether the first successful upstream scrape has completed.
    pub ready: RwLock<bool>,
    /// Monotonic counter: total upstream scrapes performed.
    pub upstream_scrapes_total: std::sync::atomic::AtomicU64,
    /// Monotonic counter: total responses served from cache (rate-limit hits).
    pub cache_hits_total: std::sync::atomic::AtomicU64,
}

impl ProxyState {
    /// Build a [`ProxyState`] from a validated [`ProxyConfig`].
    ///
    /// Returns an error if regex compilation fails or the HTTP client cannot
    /// be constructed.
    pub fn new(config: ProxyConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let filter = MetricsFilter::from_config(&config)?;
        let limiter = RateLimiter::from_config(&config)?;

        let timeout = config.upstream_timeout();
        let client = reqwest::Client::builder()
            .timeout(timeout)
            // Keep connections alive across scrapes to avoid TCP handshake overhead.
            .pool_max_idle_per_host(2)
            .build()?;

        Ok(ProxyState {
            config,
            filter,
            limiter,
            client,
            ready: RwLock::new(false),
            upstream_scrapes_total: std::sync::atomic::AtomicU64::new(0),
            cache_hits_total: std::sync::atomic::AtomicU64::new(0),
        })
    }
}

type SharedState = Arc<ProxyState>;

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /metrics` — the main proxy handler.
///
/// Returns filtered, rate-limited Prometheus text in the standard exposition
/// format (`text/plain; version=0.0.4`).
#[instrument(skip(state), name = "proxy_metrics")]
async fn metrics_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let start = Instant::now();

    // Fast path: rate-limit check.
    match state.limiter.check().await {
        ScrapeDecision::UseCached(cached) => {
            state
                .cache_hits_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return prometheus_response(StatusCode::OK, cached);
        }
        ScrapeDecision::FetchFresh => {}
    }

    // Cold path: fetch from upstream.
    let raw = match fetch_upstream(&state.client, &state.config.upstream).await {
        Ok(body) => body,
        Err(e) => {
            error!(upstream = %state.config.upstream, error = %e, "upstream scrape failed");
            // Return last cached response if available to avoid gaps in Prometheus data.
            if let ScrapeDecision::UseCached(stale) = state.limiter.check().await {
                warn!("returning stale cache after upstream error");
                return prometheus_response(StatusCode::OK, stale);
            }
            return (
                StatusCode::BAD_GATEWAY,
                [("content-type", "text/plain")],
                format!("upstream scrape error: {e}"),
            )
                .into_response();
        }
    };

    state
        .upstream_scrapes_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    // Apply label-drop and series-filter rules.
    let filtered = state.filter.apply(&raw);

    // Cache the full filtered response for subsequent rate-limited scrapes.
    state.limiter.store_full_response(filtered.clone()).await;

    // Mark proxy as ready after the first successful scrape.
    {
        let mut ready = state.ready.write().await;
        if !*ready {
            info!("metrics proxy is now ready — first upstream scrape succeeded");
            *ready = true;
        }
    }

    let elapsed_ms = start.elapsed().as_millis();
    if elapsed_ms > 5 {
        warn!(
            elapsed_ms,
            "scrape latency exceeded 5 ms SLA — consider raising min_scrape_interval_ms"
        );
    }

    prometheus_response(StatusCode::OK, filtered)
}

/// `GET /healthz` — liveness probe; always returns 200.
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// `GET /readyz` — readiness probe; 200 once the first scrape has succeeded.
async fn ready_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let ready = *state.ready.read().await;
    if ready {
        (StatusCode::OK, "ready").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "waiting for first scrape").into_response()
    }
}

/// `GET /proxy-stats` — exposes rate-limiter and cache statistics as JSON.
async fn stats_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let cache_stats = state.limiter.stats().await;
    let body = serde_json::json!({
        "upstreamScrapesTotal": state.upstream_scrapes_total.load(std::sync::atomic::Ordering::Relaxed),
        "cacheHitsTotal": state.cache_hits_total.load(std::sync::atomic::Ordering::Relaxed),
        "cachedFamilies": cache_stats.cached_families,
        "hasFullResponse": cache_stats.has_full_response,
        "lastScrapeAgeMs": cache_stats.last_scrape_age_ms,
        "upstream": state.config.upstream,
        "minScrapeIntervalMs": state.config.min_scrape_interval_ms,
    });

    match serde_json::to_string(&body) {
        Ok(json) => (
            StatusCode::OK,
            [("content-type", "application/json")],
            json,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("content-type", "text/plain")],
            format!("stats serialisation error: {e}"),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Helper: upstream HTTP fetch
// ---------------------------------------------------------------------------

async fn fetch_upstream(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, reqwest::Error> {
    let response = client.get(url).send().await?;
    response.text().await
}

// ---------------------------------------------------------------------------
// Helper: build a Prometheus text response
// ---------------------------------------------------------------------------

fn prometheus_response(status: StatusCode, body: String) -> axum::response::Response {
    (
        status,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

/// Build the Axum router for the metrics proxy.
pub fn build_router(state: SharedState) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(health_handler))
        .route("/readyz", get(ready_handler))
        .route("/proxy-stats", get(stats_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Server entry points
// ---------------------------------------------------------------------------

/// Start the metrics rate-limiter proxy server.
///
/// Binds to the address specified in `config.listen_addr`, builds the Axum
/// router, and serves requests until the process exits.
pub async fn run(config: ProxyConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listen_addr = config.listen_addr.clone();
    let upstream = config.upstream.clone();
    let min_interval = config.min_scrape_interval_ms;

    let state = Arc::new(ProxyState::new(config)?);
    let app = build_router(state);

    let addr: SocketAddr = listen_addr.parse()?;
    let listener = TcpListener::bind(addr).await?;

    info!(
        %addr,
        %upstream,
        min_scrape_interval_ms = min_interval,
        "Stellar metrics rate-limiter proxy listening"
    );

    axum::serve(listener, app).await?;
    Ok(())
}

/// Convenience wrapper: load config from the default path and start the server.
pub async fn run_from_env() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = ProxyConfig::load();
    run(config).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::config::ProxyConfig;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for `oneshot`

    fn make_state(config: ProxyConfig) -> SharedState {
        Arc::new(ProxyState::new(config).expect("valid test state"))
    }

    #[tokio::test]
    async fn healthz_returns_200() {
        let state = make_state(ProxyConfig::default());
        let app = build_router(state);

        let req = Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn readyz_returns_503_before_first_scrape() {
        let state = make_state(ProxyConfig::default());
        let app = build_router(state);

        let req = Request::builder()
            .uri("/readyz")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn readyz_returns_200_after_ready_flag() {
        let state = make_state(ProxyConfig::default());
        // Manually flip the ready flag.
        *state.ready.write().await = true;

        let app = build_router(Arc::clone(&state));
        let req = Request::builder()
            .uri("/readyz")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn proxy_stats_returns_json() {
        let state = make_state(ProxyConfig::default());
        let app = build_router(state);

        let req = Request::builder()
            .uri("/proxy-stats")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("application/json"), "unexpected content-type: {ct}");
    }

    #[tokio::test]
    async fn metrics_returns_cached_on_second_call() {
        // Seed the rate-limiter cache with a fake response.
        let config = ProxyConfig {
            min_scrape_interval_ms: 60_000, // 60 s — will never expire in test
            ..Default::default()
        };
        let state = make_state(config);
        state
            .limiter
            .store_full_response("# cached response\nup 1\n".to_owned())
            .await;

        let app = build_router(state);
        let req = Request::builder()
            .uri("/metrics")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(text.contains("up 1"), "unexpected body: {text}");
    }

    #[tokio::test]
    async fn metrics_content_type_is_prometheus() {
        let config = ProxyConfig {
            min_scrape_interval_ms: 60_000,
            ..Default::default()
        };
        let state = make_state(config);
        state
            .limiter
            .store_full_response("up 1\n".to_owned())
            .await;

        let app = build_router(state);
        let req = Request::builder()
            .uri("/metrics")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.contains("text/plain"),
            "content-type should be text/plain, got: {ct}"
        );
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let state = make_state(ProxyConfig::default());
        let app = build_router(state);

        let req = Request::builder()
            .uri("/nonexistent")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn proxy_state_new_invalid_regex_returns_error() {
        let config = ProxyConfig {
            high_frequency_patterns: vec!["[bad-regex".to_string()],
            ..Default::default()
        };
        assert!(ProxyState::new(config).is_err());
    }
}
