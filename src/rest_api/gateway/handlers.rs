//! Gateway API Handlers
//!
//! HTTP handlers for the API Gateway endpoints

use std::sync::Arc;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, Method, StatusCode},
    response::{Html, IntoResponse, Response, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::controller::ControllerState;
use crate::rest_api::dto::ErrorResponse;

use super::{
    analytics::{ApiHealth, TimeWindowMetrics},
    auth::AuthConfig,
    openapi::OpenApiGenerator,
    plugin::PluginSettings,
    ratelimit::{QuotaConfig, RateLimitInfo},
    router::RouterConfig,
    GatewayConfig, GatewayState,
};

/// Gateway state wrapper for axum
pub struct GatewayStateWrapper(pub Arc<GatewayState>);

/// Initialize gateway routes
pub fn gateway_routes<S>(state: Arc<GatewayState>) -> Router<S> {
    Router::new()
        // Health and info
        .route("/health", get(health))
        .route("/healthz", get(healthz))
        
        // Gateway management
        .route("/api/v1/gateway/config", get(get_gateway_config))
        .route("/api/v1/gateway/config", post(update_gateway_config))
        
        // Analytics endpoints
        .route("/api/v1/gateway/analytics/metrics", get(get_metrics))
        .route("/api/v1/gateway/analytics/clients", get(get_client_usage))
        .route("/api/v1/gateway/analytics/recent", get(get_recent_calls))
        
        // Health endpoint
        .route("/api/v1/gateway/health", get(get_health))
        
        // Rate limiting
        .route("/api/v1/gateway/ratelimit", get(get_rate_limit_info))
        .route("/api/v1/gateway/ratelimit", post(set_rate_limit))
        
        // Quota management
        .route("/api/v1/gateway/quota", get(get_quota))
        .route("/api/v1/gateway/quota", post(set_quota))
        
        // Plugin management
        .route("/api/v1/gateway/plugins", get(list_plugins))
        .route("/api/v1/gateway/plugins/:name/enable", post(enable_plugin))
        .route("/api/v1/gateway/plugins/:name/disable", post(disable_plugin))
        
        // OpenAPI documentation
        .route("/api/v1/gateway/openapi.json", get(get_openapi_spec))
        
        // Developer portal
        .route("/docs", get(developer_portal))
        .route("/docs/", get(developer_portal))
        
        // Router management
        .route("/api/v1/gateway/routes", get(list_routes))
        .route("/api/v1/gateway/routes", post(add_route))
        .route("/api/v1/gateway/routes/:path", delete(remove_route))
        
        .with_state(state)
}

// Health check
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "api-gateway",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn healthz() -> StatusCode {
    StatusCode::OK
}

// Gateway configuration
#[derive(Deserialize)]
struct ConfigQuery {
    #[serde(default)]
    section: Option<String>,
}

async fn get_gateway_config(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ConfigQuery>,
) -> impl IntoResponse {
    let config = GatewayConfig {
        auth: AuthConfig {
            jwt_secret: None, // Never expose secrets
            ..state.auth.clone().into()
        },
        rate_limit: RateLimitConfig::default(),
        router: RouterConfig::default(),
        plugins: vec![],
    };

    match query.section.as_deref() {
        Some("auth") => Json(config.auth),
        Some("ratelimit") => Json(config.rate_limit),
        Some("router") => Json(config.router),
        _ => Json(config),
    }
}

async fn update_gateway_config(
    State(_state): State<Arc<GatewayState>>,
    Json(_config): Json<GatewayConfig>,
) -> impl IntoResponse {
    // In production, validate and apply configuration
    (StatusCode::OK, Json(serde_json::json!({ "status": "updated" })))
}

// Analytics handlers
#[derive(Deserialize)]
struct MetricsQuery {
    #[serde(default = "default_window")]
    window_seconds: u64,
}

fn default_window() -> u64 {
    60
}

async fn get_metrics(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<MetricsQuery>,
) -> impl IntoResponse {
    let window = tokio::time::Duration::from_secs(query.window_seconds);
    let metrics = state.analytics.read().await.get_window_metrics(window);
    Json(metrics)
}

async fn get_client_usage(
    State(state): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    let usage = state.analytics.read().await.get_client_usage();
    Json(usage)
}

#[derive(Deserialize)]
struct RecentQuery {
    #[serde(default = "default_recent")]
    limit: usize,
}

fn default_recent() -> usize {
    100
}

async fn get_recent_calls(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<RecentQuery>,
) -> impl IntoResponse {
    let calls = state.analytics.read().await.get_recent_calls(query.limit);
    Json(calls)
}

async fn get_health(
    State(state): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    let health = state.analytics.read().await.get_health();
    Json(health)
}

// Rate limiting handlers
async fn get_rate_limit_info(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<MetricsQuery>,
) -> impl IntoResponse {
    let info = state.rate_limiter.get_limit_info("default").await;
    Json(info)
}

#[derive(Deserialize)]
struct RateLimitSetRequest {
    client_id: String,
    requests_per_minute: Option<u32>,
}

async fn set_rate_limit(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<RateLimitSetRequest>,
) -> impl IntoResponse {
    if let Some(rpm) = req.requests_per_minute {
        let config = super::ratelimit::RateLimitConfig {
            requests_per_minute: rpm,
            requests_per_hour: rpm * 60,
            requests_per_day: rpm * 60 * 24,
            burst_size: rpm / 5,
            ..Default::default()
        };
        state.rate_limiter.set_custom_limit(&req.client_id, config).await;
    }
    (StatusCode::OK, Json(serde_json::json!({ "status": "updated" })))
}

// Quota handlers
async fn get_quota(
    State(state): State<Arc<GatewayState>>,
    Path(client_id): Path<String>,
) -> impl IntoResponse {
    let quota = state.quota_manager.get_quota(&client_id).await;
    Json(quota)
}

async fn set_quota(
    State(state): State<Arc<GatewayState>>,
    Json(config): Json<QuotaConfig>,
) -> impl IntoResponse {
    state.quota_manager.set_quota(&config.client_id, config).await;
    (StatusCode::OK, Json(serde_json::json!({ "status": "updated" })))
}

// Plugin handlers
async fn list_plugins(
    State(state): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    let plugins = state.plugin_manager.list_plugins().await;
    Json(plugins)
}

async fn enable_plugin(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    state.plugin_manager.set_enabled(&name, true).await;
    (StatusCode::OK, Json(serde_json::json!({ "status": "enabled" })))
}

async fn disable_plugin(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    state.plugin_manager.set_enabled(&name, false).await;
    (StatusCode::OK, Json(serde_json::json!({ "status": "disabled" })))
}

// OpenAPI handler
async fn get_openapi_spec(
    State(_state): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    let routes = super::openapi::get_default_routes();
    let doc = OpenApiGenerator::new("Stellar Operator API", "1.0.0")
        .description("Kubernetes Operator API for Stellar Infrastructure")
        .add_server("https://api.stellar-operator.svc.cluster.local", Some("Kubernetes cluster".to_string()))
        .add_server("https://localhost:9090", Some("Local development".to_string()))
        .routes(routes)
        .generate();
    
    (StatusCode::OK, Json(doc))
}

// Developer portal
async fn developer_portal() -> impl IntoResponse {
    Html(super::developer_portal::DEVELOPER_PORTAL_HTML)
}

// Route management
async fn list_routes(
    State(state): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    let routes = state.router.get_routes().await;
    Json(routes)
}

#[derive(Deserialize)]
struct AddRouteRequest {
    path: String,
    target: String,
    #[serde(default)]
    methods: Vec<String>,
}

async fn add_route(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<AddRouteRequest>,
) -> impl IntoResponse {
    use super::router::{RouteRule, RouteTarget};
    
    let target = RouteTarget::Internal(req.target);
    let rule = RouteRule::path(&req.path, target);
    state.router.add_route(rule).await;
    
    (StatusCode::OK, Json(serde_json::json!({ "status": "added" })))
}

async fn remove_route(
    State(state): State<Arc<GatewayState>>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    state.router.remove_route(&path).await;
    (StatusCode::OK, Json(serde_json::json!({ "status": "removed" })))
}