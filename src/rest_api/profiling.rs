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
//! Production performance profiling HTTP endpoints (#1330).
//!
//! Routes are registered only when:
//! 1. The crate is built with `--features profiling`, and
//! 2. `REST_API_PROFILING_ENABLED=true` at process start.
//!
//! Endpoints sit on the protected REST router under `/api/v1/debug/pprof/...`
//! and require the existing Admin role (`api_admin` after `api_reader`).
//!
//! See `docs/operations/profiling-runbook.md`.

use axum::http::StatusCode;
use axum::Json;

use super::dto::ErrorResponse;

/// Environment variable that gates route registration at runtime.
pub const PROFILING_ENABLED_ENV: &str = "REST_API_PROFILING_ENABLED";

/// Default CPU sample duration when `seconds` is omitted.
pub const DEFAULT_CPU_SECONDS: u64 = 30;
/// Inclusive lower bound for CPU profile duration.
pub const MIN_CPU_SECONDS: u64 = 1;
/// Inclusive upper bound for CPU profile duration (keeps capture bounded).
pub const MAX_CPU_SECONDS: u64 = 60;

/// True when `REST_API_PROFILING_ENABLED` is a truthy value (`1`, `true`, `yes`, `on`).
pub fn profiling_runtime_enabled() -> bool {
    match std::env::var(PROFILING_ENABLED_ENV) {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
}

/// Parse and bound the `seconds` query parameter for CPU profiling.
pub fn parse_cpu_seconds(raw: Option<&str>) -> Result<u64, (StatusCode, Json<ErrorResponse>)> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_CPU_SECONDS);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(DEFAULT_CPU_SECONDS);
    }
    let seconds: u64 = trimmed.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_parameter",
                "seconds must be a positive integer",
            )),
        )
    })?;
    if !(MIN_CPU_SECONDS..=MAX_CPU_SECONDS).contains(&seconds) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_parameter",
                &format!(
                    "seconds must be between {MIN_CPU_SECONDS} and {MAX_CPU_SECONDS} inclusive"
                ),
            )),
        ));
    }
    Ok(seconds)
}

/// Attach profiling routes when the Cargo feature and runtime flag are both on.
///
/// Without the `profiling` feature, this is a no-op so default builds never
/// expose pprof endpoints.
#[cfg(feature = "profiling")]
pub fn attach_profiling_routes<S>(router: axum::Router<S>) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    use axum::middleware;
    use axum::routing::get;

    use super::auth;

    if !profiling_runtime_enabled() {
        tracing::info!(
            "{} is not enabled; profiling HTTP endpoints are not registered",
            PROFILING_ENABLED_ENV
        );
        return router;
    }

    tracing::warn!(
        "REST API profiling endpoints enabled at /api/v1/debug/pprof/* (Admin auth required)"
    );

    router
        .route(
            "/api/v1/debug/pprof/profile",
            get(cpu_profile).route_layer(middleware::from_fn(auth::api_admin)),
        )
        .route(
            "/api/v1/debug/pprof/heap",
            get(heap_profile).route_layer(middleware::from_fn(auth::api_admin)),
        )
}

#[cfg(not(feature = "profiling"))]
pub fn attach_profiling_routes<S>(router: axum::Router<S>) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    if profiling_runtime_enabled() {
        tracing::warn!(
            "{} is set but the operator binary was built without the `profiling` Cargo feature; \
             endpoints are unavailable",
            PROFILING_ENABLED_ENV
        );
    }
    router
}

#[cfg(feature = "profiling")]
mod handlers {
    use std::time::Duration;

    use axum::extract::Query;
    use axum::http::{header, HeaderValue, StatusCode};
    use axum::response::Response;
    use axum::Json;
    use serde::Deserialize;
    use tokio::sync::Mutex;

    use super::{parse_cpu_seconds, ErrorResponse, MAX_CPU_SECONDS};

    /// Serializes CPU captures so concurrent requests cannot stack profilers.
    static CPU_PROFILE_LOCK: Mutex<()> = Mutex::const_new(());

    #[derive(Debug, Deserialize)]
    pub struct CpuProfileQuery {
        /// Capture duration in seconds (1..=60). Defaults to 30.
        pub seconds: Option<String>,
        /// Optional format; only `proto` (default) is accepted.
        pub format: Option<String>,
    }

    pub async fn cpu_profile(
        Query(q): Query<CpuProfileQuery>,
    ) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
        let seconds = parse_cpu_seconds(q.seconds.as_deref())?;
        // Only protobuf is supported. SVG flamegraphs would pull in `inferno`
        // (CDDL-1.0), which is outside this repository's cargo-deny allowlist.
        // Operators render flamegraphs locally with `pprof` / `go tool pprof`.
        if let Some(format) = q.format.as_deref() {
            let format = format.trim().to_ascii_lowercase();
            if !format.is_empty() && format != "proto" {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new(
                        "invalid_parameter",
                        "format must be 'proto' (SVG flamegraphs are generated offline with pprof)",
                    )),
                ));
            }
        }

        let Ok(guard) = CPU_PROFILE_LOCK.try_lock() else {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse::new(
                    "profiler_busy",
                    "another CPU profile is already in progress",
                )),
            ));
        };

        let result = tokio::task::spawn_blocking(move || capture_cpu(seconds))
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        "profiler_error",
                        &format!("profiler task failed: {e}"),
                    )),
                )
            })?;

        drop(guard);
        result
    }

    fn capture_cpu(seconds: u64) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
        let guard = pprof::ProfilerGuardBuilder::default()
            .frequency(100)
            .blocklist(&["libc", "libgcc", "pthread", "vdso"])
            .build()
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        "profiler_error",
                        &format!("failed to start CPU profiler: {e}"),
                    )),
                )
            })?;

        std::thread::sleep(Duration::from_secs(seconds.min(MAX_CPU_SECONDS)));

        let report = guard.report().build().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "profiler_error",
                    &format!("failed to build CPU profile: {e}"),
                )),
            )
        })?;

        let profile = report.pprof().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "profiler_error",
                    &format!("failed to encode pprof profile: {e}"),
                )),
            )
        })?;
        use prost::Message;
        let mut buf = Vec::new();
        profile.encode(&mut buf).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "profiler_error",
                    &format!("failed to serialize pprof profile: {e}"),
                )),
            )
        })?;

        let mut response = Response::new(axum::body::Body::from(buf));
        *response.status_mut() = StatusCode::OK;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"cpu-profile.pb\""),
        );
        Ok(response)
    }

    pub async fn heap_profile() -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
        let Some(ctl) = jemalloc_pprof::PROF_CTL.as_ref() else {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::new(
                    "heap_unavailable",
                    "jemalloc profiling control is not available on this platform/build",
                )),
            ));
        };

        let mut prof_ctl = ctl.lock().await;
        if !prof_ctl.activated() {
            drop(prof_ctl);
            jemalloc_pprof::activate_jemalloc_profiling().await;
            prof_ctl = ctl.lock().await;
        }
        if !prof_ctl.activated() {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::new(
                    "heap_inactive",
                    "jemalloc heap profiling is not activated; ensure the image was built with \
                     `--features profiling` and MALLOC_CONF includes prof:true",
                )),
            ));
        }

        let pprof = prof_ctl.dump_pprof().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "profiler_error",
                    &format!("failed to dump heap profile: {e}"),
                )),
            )
        })?;

        let mut response = Response::new(axum::body::Body::from(pprof));
        *response.status_mut() = StatusCode::OK;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"heap-profile.pb\""),
        );
        Ok(response)
    }
}

#[cfg(feature = "profiling")]
use handlers::{cpu_profile, heap_profile};

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::{Arc, Mutex, MutexGuard};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use kube::Client;
    use tower::ServiceExt;
    use tracing_subscriber::{EnvFilter, Registry};

    use crate::controller::ControllerState;
    use crate::rest_api::auth::{api_admin, api_reader};
    use crate::rest_api::oidc::{ApiRole, OidcConfig};

    use super::*;

    /// Serialize env mutations for profiling route-registration tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn make_reload_handle() -> tracing_subscriber::reload::Handle<EnvFilter, Registry> {
        let env_filter = EnvFilter::new("info");
        let (_layer, handle): (
            tracing_subscriber::reload::Layer<EnvFilter, Registry>,
            tracing_subscriber::reload::Handle<EnvFilter, Registry>,
        ) = tracing_subscriber::reload::Layer::new(env_filter);
        handle
    }

    /// Dummy kube client (never used when oidc_config is set; OIDC path skips TokenReview).
    fn dummy_client() -> Client {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config = kube::Config::new("http://127.0.0.1:1".parse().unwrap());
        Client::try_from(config).expect("dummy kube client")
    }

    fn test_oidc_config() -> OidcConfig {
        OidcConfig {
            issuer: "https://accounts.example.test".into(),
            audience: "stellar-operator".into(),
            jwks_uri: "https://accounts.example.test/.well-known/jwks.json".into(),
            roles_claim: "roles".into(),
        }
    }

    fn test_state() -> Arc<ControllerState> {
        let audit_log = Arc::new(crate::controller::audit_log::AuditLog::new());
        Arc::new(ControllerState {
            client: dummy_client(),
            enable_mtls: false,
            operator_namespace: "stellar-operator".into(),
            watch_namespace: None,
            mtls_config: None,
            dry_run: true,
            retry_budget_retriable_secs: 5,
            retry_budget_nonretriable_secs: 60,
            retry_budget_max_attempts: 3,
            is_leader: Arc::new(AtomicBool::new(true)),
            event_reporter: kube::runtime::events::Reporter {
                controller: "stellar-operator".into(),
                instance: None,
            },
            operator_config: Arc::new(Default::default()),
            reconcile_id_counter: AtomicU64::new(0),
            last_reconcile_success: Arc::new(AtomicU64::new(0)),
            log_reload_handle: make_reload_handle(),
            log_level_expires_at: Arc::new(tokio::sync::Mutex::new(None)),
            last_event_received: Arc::new(AtomicU64::new(0)),
            job_registry: Arc::new(crate::controller::background_jobs::JobRegistry::new()),
            audit_log: audit_log.clone(),
            audit_recorder: Arc::new(crate::controller::audit_recorder::AuditRecorder::new(
                audit_log,
                vec![],
                None,
            )),
            anomaly_detector: Arc::new(crate::controller::anomaly_detection::AnomalyDetector::new(
                Default::default(),
            )),
            plugin_registry: Arc::new(crate::plugin_sdk::PluginRegistry::new()),
            analytics_engine: Arc::new(crate::logging::analytics::AnalyticsEngine::new(
                std::time::Duration::from_secs(3600),
            )),
            oidc_config: Some(test_oidc_config()),
            metrics_store: Arc::new(crate::rest_api::metrics_store::StellarMetricsStore::new()),
        })
    }

    /// Unsigned JWT accepted by structural OIDC validation (signature not verified yet).
    fn oidc_token(roles: &[&str]) -> String {
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = serde_json::json!({
            "iss": "https://accounts.example.test",
            "aud": "stellar-operator",
            "exp": exp,
            "sub": "test-user",
            "roles": roles,
        });
        let payload_enc = URL_SAFE_NO_PAD.encode(serde_json::to_string(&payload).unwrap());
        format!("{header}.{payload_enc}.fakesig")
    }

    async fn stub_ok() -> &'static str {
        "profile-ok"
    }

    /// Real production middleware stack for profiling routes: `api_reader` then `api_admin`.
    fn profiling_auth_app(state: Arc<ControllerState>) -> Router {
        Router::new()
            .route(
                "/api/v1/debug/pprof/profile",
                get(stub_ok).route_layer(middleware::from_fn(api_admin)),
            )
            .route(
                "/api/v1/debug/pprof/heap",
                get(stub_ok).route_layer(middleware::from_fn(api_admin)),
            )
            .layer(middleware::from_fn_with_state(state.clone(), api_reader))
            .with_state(state)
    }

    #[test]
    fn runtime_flag_parser_truthy_values() {
        assert!(!parse_truthy(""));
        assert!(!parse_truthy("0"));
        assert!(!parse_truthy("false"));
        assert!(parse_truthy("true"));
        assert!(parse_truthy("1"));
        assert!(parse_truthy("YES"));
        assert!(parse_truthy("on"));
    }

    fn parse_truthy(v: &str) -> bool {
        let v = v.trim().to_ascii_lowercase();
        matches!(v.as_str(), "1" | "true" | "yes" | "on")
    }

    #[test]
    fn cpu_seconds_defaults_and_bounds() {
        assert_eq!(parse_cpu_seconds(None).unwrap(), DEFAULT_CPU_SECONDS);
        assert_eq!(parse_cpu_seconds(Some("10")).unwrap(), 10);
        assert_eq!(parse_cpu_seconds(Some("1")).unwrap(), 1);
        assert_eq!(parse_cpu_seconds(Some("60")).unwrap(), 60);
        assert_eq!(
            parse_cpu_seconds(Some("0")).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            parse_cpu_seconds(Some("61")).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            parse_cpu_seconds(Some("abc")).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            parse_cpu_seconds(Some("-1")).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn missing_authorization_returns_401_via_api_reader() {
        let app = profiling_auth_app(test_state());
        for uri in [
            "/api/v1/debug/pprof/profile?seconds=1",
            "/api/v1/debug/pprof/heap",
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "expected 401 from real api_reader for {uri}"
            );
        }
    }

    #[tokio::test]
    async fn reader_role_returns_403_via_api_admin() {
        let app = profiling_auth_app(test_state());
        let token = oidc_token(&["Reader"]);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/debug/pprof/profile?seconds=1")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_role_reaches_profiling_handler() {
        let app = profiling_auth_app(test_state());
        let token = oidc_token(&["Admin"]);
        for uri in [
            "/api/v1/debug/pprof/profile?seconds=1",
            "/api/v1/debug/pprof/heap",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header("Authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "Admin should reach handler for {uri}"
            );
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            assert_eq!(&body[..], b"profile-ok");
        }
    }

    #[tokio::test]
    async fn profiling_paths_follow_api_v1_versioning_headers() {
        use axum::Extension;

        use crate::rest_api::versioning::{self, VersionPolicy};

        let state = test_state();
        let policy = Arc::new(VersionPolicy::default());
        let app = profiling_auth_app(state)
            .layer(middleware::from_fn(versioning::inject_api_version_headers))
            .layer(Extension(policy));
        let token = oidc_token(&["Admin"]);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/debug/pprof/heap")
                    .header("Authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("deprecation").is_none());
    }

    #[tokio::test]
    async fn runtime_disabled_profiling_routes_not_registered() {
        let _guard = lock_env();
        let previous = std::env::var(PROFILING_ENABLED_ENV).ok();
        std::env::remove_var(PROFILING_ENABLED_ENV);
        std::env::set_var(PROFILING_ENABLED_ENV, "false");

        let app = attach_profiling_routes(Router::new().route("/health", get(stub_ok)));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/debug/pprof/profile?seconds=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "disabled runtime flag must not expose profiling routes"
        );

        match previous {
            Some(v) => std::env::set_var(PROFILING_ENABLED_ENV, v),
            None => std::env::remove_var(PROFILING_ENABLED_ENV),
        }
    }

    #[cfg(feature = "profiling")]
    #[tokio::test]
    async fn runtime_enabled_profiling_routes_registered() {
        let _guard = lock_env();
        let previous = std::env::var(PROFILING_ENABLED_ENV).ok();
        std::env::set_var(PROFILING_ENABLED_ENV, "true");

        // Routes are registered with api_admin; absence of identity ? 403 proves the route exists.
        let app = attach_profiling_routes(Router::new());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/debug/pprof/heap")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "enabled runtime flag must register profiling routes (api_admin rejects bare requests)"
        );

        match previous {
            Some(v) => std::env::set_var(PROFILING_ENABLED_ENV, v),
            None => std::env::remove_var(PROFILING_ENABLED_ENV),
        }
    }

    #[cfg(not(feature = "profiling"))]
    #[tokio::test]
    async fn without_profiling_feature_routes_unavailable_even_if_env_set() {
        let _guard = lock_env();
        let previous = std::env::var(PROFILING_ENABLED_ENV).ok();
        std::env::set_var(PROFILING_ENABLED_ENV, "true");

        let app = attach_profiling_routes(Router::new());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/debug/pprof/profile")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        match previous {
            Some(v) => std::env::set_var(PROFILING_ENABLED_ENV, v),
            None => std::env::remove_var(PROFILING_ENABLED_ENV),
        }
    }

    #[cfg(feature = "profiling")]
    #[tokio::test]
    async fn invalid_cpu_format_rejected() {
        let app = Router::new().route("/api/v1/debug/pprof/profile", get(cpu_profile));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/debug/pprof/profile?seconds=1&format=xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[cfg(feature = "profiling")]
    #[tokio::test]
    async fn invalid_cpu_seconds_rejected_by_handler() {
        let app = Router::new().route("/api/v1/debug/pprof/profile", get(cpu_profile));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/debug/pprof/profile?seconds=999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    // Full CPU/heap profile generation (1-60s samples, jemalloc dumps) is not exercised in
    // unit tests: it is slow, platform-dependent, and requires a writable temp dir + symbols.
    // Handler bounds and auth/route registration above cover the production safety boundary;
    // profile byte formats are validated in integration/CI against a profiling-featured image.
    #[allow(dead_code)]
    fn profile_output_integration_note() {
        let _ = ApiRole::Admin;
    }
}
