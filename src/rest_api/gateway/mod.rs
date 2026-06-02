//! API Gateway Module
//!
//! Provides unified access to operator APIs with:
//! - Plugin system for extensibility
//! - Multiple authentication methods (JWT, OAuth2, API keys)
//! - Rate limiting with quota management
//! - Request/response transformation pipeline
//! - API versioning and routing
//! - API analytics and usage tracking
//! - OpenAPI/Swagger documentation
//! - Developer portal with API explorer

pub mod analytics;
pub mod auth;
pub mod developer_portal;
pub mod handlers;
pub mod openapi;
pub mod plugin;
pub mod ratelimit;
pub mod router;
pub mod transform;

pub use analytics::{Analytics, ApiCall, GatewayMetrics, TimeWindowMetrics, ApiHealth, HealthStatus};
pub use auth::{AuthConfig, AuthMiddleware, AuthProvider, JwtAuth, OAuth2Auth, ApiKeyAuth, AuthContext, AuthError};
pub use plugin::{GatewayPlugin, PluginContext, PluginHook, PluginManager};
pub use ratelimit::{RateLimitConfig, RateLimiter, QuotaManager, QuotaConfig, QuotaTier};
pub use router::{RouteRule, RouterConfig, VersionedRouter, ApiVersion};
pub use transform::{TransformPipeline, TransformRule, BodyTransform};
pub use openapi::{OpenApiDocument, OpenApiGenerator, ApiRoute, get_default_routes};
pub use handlers::{GatewayStateWrapper, gateway_routes};
pub use developer_portal::DEVELOPER_PORTAL_HTML;

use std::sync::Arc;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use tokio::sync::RwLock;

use crate::controller::ControllerState;
use crate::rest_api::dto::ErrorResponse;

/// Gateway state shared across requests
pub struct GatewayState {
    pub auth: AuthMiddleware,
    pub router: VersionedRouter,
    pub rate_limiter: RateLimiter,
    pub quota_manager: QuotaManager,
    pub transform_pipeline: TransformPipeline,
    pub plugin_manager: PluginManager,
    pub analytics: Arc<RwLock<Analytics>>,
    pub inner: Arc<ControllerState>,
}

impl GatewayState {
    pub fn new(inner: Arc<ControllerState>) -> Self {
        Self {
            auth: AuthMiddleware::default(),
            router: VersionedRouter::new(),
            rate_limiter: RateLimiter::new(100, 60), // 100 req/min default
            quota_manager: QuotaManager::new(),
            transform_pipeline: TransformPipeline::new(),
            plugin_manager: PluginManager::new(),
            analytics: Arc::new(RwLock::new(Analytics::new())),
            inner,
        }
    }
}

/// Main gateway request handler
pub async fn gateway_handler(
    State(state): State<Arc<GatewayState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let start = std::time::Instant::now();
    let client_ip = get_client_ip(&request);
    let path = request.uri().path().to_string();
    let method = request.method().clone();

    // 1. Authentication
    let auth_result = state.auth.authenticate(&request).await;
    if let Err(e) = auth_result {
        return error_response(e.status, e.code, e.message);
    }
    let auth_context = auth_result.unwrap();

    // 2. Rate Limiting
    if !state.rate_limiter.check(&client_ip).await {
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Rate limit exceeded",
        );
    }

    // 3. Quota Check
    if let Err(e) = state.quota_manager.check_quota(&auth_context.client_id) {
        return error_response(StatusCode::PAYMENT_REQUIRED, "quota_exceeded", &e);
    }

    // 4. Plugin Pre-processing
    let mut ctx = PluginContext {
        request: request.clone(),
        auth: auth_context.clone(),
        state: state.inner.clone(),
    };
    if let Some(should_continue) = state.plugin_manager.pre_process(&mut ctx).await {
        if !should_continue {
            return error_response(
                StatusCode::FORBIDDEN,
                "plugin_rejected",
                "Request rejected by plugin",
            );
        }
    }

    // 5. Request Transformation
    let transformed = state.transform_pipeline.transform_request(ctx.request).await;

    // 6. Route to correct version
    let routed = state.router.route(&transformed).await;

    // 7. Execute request
    let response = next.run(routed).await;

    // 8. Response Transformation
    let final_response = state.transform_pipeline.transform_response(response).await;

    // 9. Plugin Post-processing
    ctx.response = Some(final_response.clone());
    state.plugin_manager.post_process(&mut ctx).await;

    // 10. Record Analytics
    let call = ApiCall {
        timestamp: chrono::Utc::now(),
        path: path.clone(),
        method: method.to_string(),
        status: final_response.status().as_u16(),
        latency_ms: start.elapsed().as_millis() as u64,
        client_id: auth_context.client_id.clone(),
        client_ip,
    };
    state.analytics.write().await.record_call(call);

    // 11. Update quota
    state.quota_manager.record_request(&auth_context.client_id).await;

    final_response
}

fn get_client_ip(request: &Request<Body>) -> String {
    request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(ErrorResponse::new(code, message))).into_response()
}

/// Gateway configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GatewayConfig {
    pub auth: AuthConfig,
    pub rate_limit: RateLimitConfig,
    pub router: RouterConfig,
    pub plugins: Vec<PluginConfig>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginConfig {
    pub name: String,
    pub enabled: bool,
    pub config: std::collections::HashMap<String, String>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            auth: AuthConfig::default(),
            rate_limit: RateLimitConfig::default(),
            router: RouterConfig::default(),
            plugins: vec![],
        }
    }
}