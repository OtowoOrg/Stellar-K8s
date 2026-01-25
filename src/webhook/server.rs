//! Admission Webhook Server
//!
//! This module implements a Kubernetes ValidatingAdmissionWebhook server
//! that executes Wasm plugins for custom StellarNode validation.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{error, info, instrument, warn};

use super::runtime::WasmRuntime;
use super::types::{
    Operation, PluginConfig, PluginExecutionResult, PluginMetadata, UserInfo, ValidationInput,
    ValidationOutput,
};
use crate::crd::StellarNode;
use crate::error::{Error, Result};

/// Webhook server state
pub struct WebhookServer {
    /// Wasm runtime for plugin execution
    runtime: Arc<WasmRuntime>,

    /// Configured plugins
    plugins: Arc<RwLock<Vec<PluginConfig>>>,

    /// TLS configuration
    tls_config: Option<TlsConfig>,
}

/// TLS configuration for the webhook server
#[derive(Clone)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

/// Plugin management request
#[derive(Debug, Deserialize)]
pub struct LoadPluginRequest {
    pub metadata: PluginMetadata,
    #[serde(with = "base64_serde")]
    pub wasm_binary: Vec<u8>,
    pub operations: Vec<Operation>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub fail_open: bool,
}

fn default_true() -> bool {
    true
}

/// Plugin list response
#[derive(Debug, Serialize)]
pub struct PluginListResponse {
    pub plugins: Vec<PluginInfo>,
}

/// Plugin info
#[derive(Debug, Serialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub operations: Vec<Operation>,
    pub enabled: bool,
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub plugins_loaded: usize,
}

/// Server-side validation result (simplified from AggregatedValidationResult)
#[derive(Debug)]
pub struct ServerValidationResult {
    pub allowed: bool,
    pub message: Option<String>,
    pub warnings: Vec<String>,
    pub plugin_results: Vec<PluginExecutionResult>,
    pub total_execution_time_ms: u64,
}

/// Validation result response
#[derive(Debug, Serialize)]
pub struct ValidationResultResponse {
    pub allowed: bool,
    pub message: Option<String>,
    pub results: Vec<PluginResultInfo>,
}

#[derive(Debug, Serialize)]
pub struct PluginResultInfo {
    pub plugin_name: String,
    pub allowed: bool,
    pub message: Option<String>,
    pub execution_time_ms: u64,
}

impl WebhookServer {
    /// Create a new webhook server
    pub fn new(runtime: WasmRuntime) -> Self {
        Self {
            runtime: Arc::new(runtime),
            plugins: Arc::new(RwLock::new(Vec::new())),
            tls_config: None,
        }
    }

    /// Configure TLS
    pub fn with_tls(mut self, cert_path: String, key_path: String) -> Self {
        self.tls_config = Some(TlsConfig {
            cert_path,
            key_path,
        });
        self
    }

    /// Add a plugin
    pub async fn add_plugin(&self, config: PluginConfig) -> Result<()> {
        // Decode base64 wasm_binary
        let wasm_binary_str = config
            .wasm_binary
            .as_ref()
            .ok_or_else(|| Error::PluginError("Plugin wasm_binary is required".to_string()))?;

        let wasm_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, wasm_binary_str)
                .map_err(|e| Error::PluginError(format!("Invalid base64 wasm_binary: {}", e)))?;

        // Load into runtime
        self.runtime
            .load_plugin(&wasm_bytes, config.metadata.clone())
            .await?;

        // Add to plugins list
        let mut plugins = self.plugins.write().await;

        // Remove existing plugin with same name
        plugins.retain(|p| p.metadata.name != config.metadata.name);

        plugins.push(config);

        Ok(())
    }

    /// Remove a plugin
    pub async fn remove_plugin(&self, name: &str) -> Result<()> {
        self.runtime.unload_plugin(name).await?;

        let mut plugins = self.plugins.write().await;
        plugins.retain(|p| p.metadata.name != name);

        Ok(())
    }

    /// Get loaded plugins
    pub async fn list_plugins(&self) -> Vec<PluginConfig> {
        self.plugins.read().await.clone()
    }

    /// Validate a StellarNode
    #[instrument(skip(self, input))]
    pub async fn validate(&self, input: ValidationInput) -> ServerValidationResult {
        let plugins = self.plugins.read().await.clone();

        if plugins.is_empty() {
            return ServerValidationResult {
                allowed: true,
                message: Some("No validation plugins configured".to_string()),
                warnings: vec![],
                plugin_results: vec![],
                total_execution_time_ms: 0,
            };
        }

        let start = std::time::Instant::now();
        let results = self.runtime.execute_all(&plugins, &input).await;

        let mut allowed = true;
        let mut messages = Vec::new();
        let mut warnings = Vec::new();
        let mut plugin_results = Vec::new();

        for result in results {
            match result {
                Ok(exec_result) => {
                    if !exec_result.output.allowed {
                        allowed = false;
                        if let Some(msg) = &exec_result.output.message {
                            messages.push(format!("{}: {}", exec_result.plugin_name, msg));
                        }
                    }
                    warnings.extend(exec_result.output.warnings.clone());
                    plugin_results.push(exec_result);
                }
                Err(e) => {
                    allowed = false;
                    messages.push(format!("Plugin execution error: {}", e));
                    plugin_results.push(PluginExecutionResult {
                        plugin_name: "unknown".to_string(),
                        output: ValidationOutput::denied(e.to_string()),
                        execution_time_ms: 0,
                        memory_used_bytes: 0,
                        fuel_consumed: 0,
                    });
                }
            }
        }

        ServerValidationResult {
            allowed,
            message: if messages.is_empty() {
                None
            } else {
                Some(messages.join("; "))
            },
            warnings,
            plugin_results,
            total_execution_time_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Start the webhook server
    pub async fn start(self, addr: SocketAddr) -> Result<()> {
        // Check TLS config before moving self into Arc
        let has_tls = self.tls_config.is_some();

        let state = Arc::new(self);

        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/healthz", get(health_handler))
            .route("/ready", get(ready_handler))
            .route("/validate", post(validate_handler))
            .route("/mutate", post(mutate_handler))
            .route("/plugins", get(list_plugins_handler))
            .route("/plugins", post(add_plugin_handler))
            .route(
                "/plugins/:name",
                axum::routing::delete(remove_plugin_handler),
            )
            .with_state(state);

        info!("Starting webhook server on {}", addr);

        // Check if TLS is configured
        if has_tls {
            // TODO: Implement TLS server with rustls
            // For now, fall back to non-TLS
            warn!("TLS configuration provided but not yet implemented, using plain HTTP");
        }

        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| Error::PluginError(format!("Failed to bind to {}: {}", addr, e)))?;

        axum::serve(listener, app)
            .await
            .map_err(|e| Error::PluginError(format!("Server error: {}", e)))?;

        Ok(())
    }
}

// HTTP Handlers

async fn health_handler(State(state): State<Arc<WebhookServer>>) -> impl IntoResponse {
    let plugins = state.runtime.list_plugins().await;
    Json(HealthResponse {
        status: "healthy".to_string(),
        plugins_loaded: plugins.len(),
    })
}

async fn ready_handler(State(state): State<Arc<WebhookServer>>) -> impl IntoResponse {
    let plugins = state.plugins.read().await;
    if plugins.is_empty() {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "no plugins loaded".to_string(),
                plugins_loaded: 0,
            }),
        )
    } else {
        (
            StatusCode::OK,
            Json(HealthResponse {
                status: "ready".to_string(),
                plugins_loaded: plugins.len(),
            }),
        )
    }
}

#[instrument(skip(state, review))]
async fn validate_handler(
    State(state): State<Arc<WebhookServer>>,
    Json(review): Json<AdmissionReview<StellarNode>>,
) -> impl IntoResponse {
    let request = match review.try_into() {
        Ok(req) => req,
        Err(e) => {
            error!("Failed to parse admission request: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    AdmissionResponse::invalid(format!("Invalid admission request: {}", e))
                        .into_review(),
                ),
            );
        }
    };

    let req: AdmissionRequest<StellarNode> = request;

    // Build validation input
    let input = build_validation_input(&req);

    // Execute validation
    let result = state.validate(input).await;

    // Build response
    let mut response = if result.allowed {
        AdmissionResponse::from(&req)
    } else {
        AdmissionResponse::from(&req).deny(
            result
                .message
                .unwrap_or_else(|| "Validation failed".to_string()),
        )
    };

    // Add warnings if any
    if !result.warnings.is_empty() {
        response.warnings = Some(result.warnings);
    }

    info!(
        "Validation result: allowed={}, time={}ms",
        result.allowed, result.total_execution_time_ms
    );

    (StatusCode::OK, Json(response.into_review()))
}

#[instrument(skip(_state, review))]
async fn mutate_handler(
    State(_state): State<Arc<WebhookServer>>,
    Json(review): Json<AdmissionReview<StellarNode>>,
) -> impl IntoResponse {
    // For now, mutation is not supported - just pass through
    let request: Result<AdmissionRequest<StellarNode>, _> = review.try_into();

    match request {
        Ok(req) => {
            let response = AdmissionResponse::from(&req);
            (StatusCode::OK, Json(response.into_review()))
        }
        Err(e) => {
            error!("Failed to parse admission request: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(
                    AdmissionResponse::invalid(format!("Invalid admission request: {}", e))
                        .into_review(),
                ),
            )
        }
    }
}

async fn list_plugins_handler(State(state): State<Arc<WebhookServer>>) -> impl IntoResponse {
    let plugins = state.plugins.read().await;
    let infos: Vec<PluginInfo> = plugins
        .iter()
        .map(|p| PluginInfo {
            name: p.metadata.name.clone(),
            version: p.metadata.version.clone(),
            description: p.metadata.description.clone(),
            operations: p.operations.clone(),
            enabled: p.enabled,
        })
        .collect();

    Json(PluginListResponse { plugins: infos })
}

async fn add_plugin_handler(
    State(state): State<Arc<WebhookServer>>,
    Json(request): Json<LoadPluginRequest>,
) -> impl IntoResponse {
    // Convert Vec<u8> to base64 String for storage in PluginConfig
    let wasm_binary_base64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &request.wasm_binary,
    );

    let config = PluginConfig {
        metadata: request.metadata,
        wasm_binary: Some(wasm_binary_base64),
        config_map_ref: None,
        secret_ref: None,
        url: None,
        operations: request.operations,
        enabled: request.enabled,
        fail_open: request.fail_open,
        plugin_config: BTreeMap::new(),
    };

    match state.add_plugin(config).await {
        Ok(_) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"status": "created"})),
        ),
        Err(e) => {
            error!("Failed to add plugin: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        }
    }
}

async fn remove_plugin_handler(
    State(state): State<Arc<WebhookServer>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> impl IntoResponse {
    match state.remove_plugin(&name).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed"})),
        ),
        Err(e) => {
            error!("Failed to remove plugin: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        }
    }
}

/// Build ValidationInput from AdmissionRequest
fn build_validation_input(req: &AdmissionRequest<StellarNode>) -> ValidationInput {
    let operation = match req.operation {
        kube::core::admission::Operation::Create => Operation::Create,
        kube::core::admission::Operation::Update => Operation::Update,
        kube::core::admission::Operation::Delete => Operation::Delete,
        kube::core::admission::Operation::Connect => Operation::Connect,
    };

    let user_info = UserInfo {
        username: req.user_info.username.clone().unwrap_or_default(),
        uid: req.user_info.uid.clone(),
        groups: req.user_info.groups.clone().unwrap_or_default(),
        extra: req.user_info.extra.clone().unwrap_or_default(),
    };

    ValidationInput {
        operation,
        object: req
            .object
            .as_ref()
            .map(|o| serde_json::to_value(o).unwrap_or_default()),
        old_object: req
            .old_object
            .as_ref()
            .map(|o| serde_json::to_value(o).unwrap_or_default()),
        namespace: req.namespace.clone().unwrap_or_default(),
        name: req.name.clone(),
        user_info,
        context: BTreeMap::new(),
    }
}

// Base64 serde helper
mod base64_serde {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serializer};

    #[allow(dead_code)]
    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        STANDARD.decode(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_webhook_server_creation() {
        let runtime = WasmRuntime::new().unwrap();
        let server = WebhookServer::new(runtime);
        assert!(server.list_plugins().await.is_empty());
    }

    #[tokio::test]
    async fn test_validation_no_plugins() {
        let runtime = WasmRuntime::new().unwrap();
        let server = WebhookServer::new(runtime);

        let input = ValidationInput {
            operation: Operation::Create,
            object: Some(serde_json::json!({})),
            old_object: None,
            namespace: "default".to_string(),
            name: "test".to_string(),
            user_info: UserInfo {
                username: "test-user".to_string(),
                uid: None,
                groups: vec![],
                extra: BTreeMap::new(),
            },
            context: BTreeMap::new(),
        };

        let result = server.validate(input).await;
        assert!(result.allowed);
    }
}
