//! Admission Webhook Server
//!
//! This module implements a Kubernetes ValidatingAdmissionWebhook server
//! that executes Wasm plugins for custom StellarNode validation.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

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
use crate::crd::{StellarNode, StellarNodeSpec};
use crate::error::{Error, Result};

/// Webhook server state
pub struct WebhookServer {
    /// Wasm runtime for plugin execution
    runtime: Arc<WasmRuntime>,

    /// Configured plugins
    plugins: Arc<RwLock<Vec<PluginConfig>>>,

    /// TLS configuration
    tls_config: Option<TlsConfig>,

    /// External policy delegation configuration for OPA/Gatekeeper.
    policy_config: PolicyDelegationConfig,

    /// HTTP client used for external policy delegation requests.
    policy_http: reqwest::Client,
}

#[derive(Clone, Debug)]
struct PolicyDelegationConfig {
    endpoint: Option<String>,
    timeout: Duration,
    fail_open: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DelegatedPolicyResponse {
    allowed: bool,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecurityPolicyInfo {
    name: String,
    engine: String,
    description: String,
    path: String,
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
        let endpoint = std::env::var("OPA_GATEKEEPER_WEBHOOK_URL")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let timeout_ms = std::env::var("OPA_GATEKEEPER_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1500);
        let fail_open = std::env::var("OPA_GATEKEEPER_FAIL_OPEN")
            .ok()
            .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        let timeout = Duration::from_millis(timeout_ms);
        let policy_http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            runtime: Arc::new(runtime),
            plugins: Arc::new(RwLock::new(Vec::new())),
            tls_config: None,
            policy_config: PolicyDelegationConfig {
                endpoint,
                timeout,
                fail_open,
            },
            policy_http,
        }
    }

    async fn delegate_policy_check(&self, input: &ValidationInput) -> ValidationOutput {
        let Some(endpoint) = self.policy_config.endpoint.as_ref() else {
            return ValidationOutput::allowed();
        };

        let result = self.policy_http.post(endpoint).json(input).send().await;

        let response = match result {
            Ok(resp) => resp,
            Err(e) => {
                let message = format!(
                    "OPA/Gatekeeper delegation request failed (timeout={}ms): {}",
                    self.policy_config.timeout.as_millis(),
                    e
                );
                return if self.policy_config.fail_open {
                    ValidationOutput::allowed_with_warnings(vec![message])
                } else {
                    ValidationOutput::denied(message)
                };
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let message = format!(
                "OPA/Gatekeeper delegation returned HTTP {}: {}",
                status,
                body.trim()
            );
            return if self.policy_config.fail_open {
                ValidationOutput::allowed_with_warnings(vec![message])
            } else {
                ValidationOutput::denied(message)
            };
        }

        match response.json::<DelegatedPolicyResponse>().await {
            Ok(payload) => {
                if payload.allowed {
                    if payload.warnings.is_empty() {
                        ValidationOutput::allowed()
                    } else {
                        ValidationOutput::allowed_with_warnings(payload.warnings)
                    }
                } else {
                    ValidationOutput {
                        allowed: false,
                        message: payload
                            .message
                            .or(Some("Denied by OPA/Gatekeeper policy".to_string())),
                        reason: Some("DelegatedPolicyDenied".to_string()),
                        errors: Vec::new(),
                        warnings: payload.warnings,
                        audit_annotations: BTreeMap::new(),
                    }
                }
            }
            Err(e) => {
                let message = format!("Invalid OPA/Gatekeeper response payload: {}", e);
                if self.policy_config.fail_open {
                    ValidationOutput::allowed_with_warnings(vec![message])
                } else {
                    ValidationOutput::denied(message)
                }
            }
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
                .map_err(|e| Error::PluginError(format!("Invalid base64 wasm_binary: {e}")))?;

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

    /// Validate a StellarNode (built-in spec validation first, then Wasm plugins)
    #[instrument(
        skip(self, input),
        fields(node_name = "-", namespace = "-", reconcile_id = "-")
    )]
    pub async fn validate(&self, input: ValidationInput) -> ServerValidationResult {
        let mut warnings = Vec::new();

        if let Some(ref object) = input.object {
            if matches!(input.operation, Operation::Create | Operation::Update) {
                // Collect image pinning warnings
                if let Ok(node) = serde_json::from_value::<StellarNode>(object.clone()) {
                    warnings.extend(check_image_pinning(&node.spec));
                }

                if let Some(mut builtin) = validate_spec_builtin(object) {
                    builtin.warnings.extend(warnings);
                    return builtin;
                }

                let delegated = self.delegate_policy_check(&input).await;
                warnings.extend(delegated.warnings.clone());
                if !delegated.allowed {
                    return ServerValidationResult {
                        allowed: false,
                        message: delegated
                            .message
                            .or(Some("Denied by OPA/Gatekeeper policy".to_string())),
                        warnings,
                        plugin_results: vec![],
                        total_execution_time_ms: 0,
                    };
                }
            }
        }

        let plugins = self.plugins.read().await.clone();

        if plugins.is_empty() {
            return ServerValidationResult {
                allowed: true,
                message: Some("No validation plugins configured".to_string()),
                warnings,
                plugin_results: vec![],
                total_execution_time_ms: 0,
            };
        }

        let start = std::time::Instant::now();
        let results = self.runtime.execute_all(&plugins, &input).await;

        let mut allowed = true;
        let mut messages = Vec::new();
        let mut warnings_from_plugins = Vec::new();
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
                    warnings_from_plugins.extend(exec_result.output.warnings.clone());
                    plugin_results.push(exec_result);
                }
                Err(e) => {
                    allowed = false;
                    messages.push(format!("Plugin execution error: {e}"));
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
            warnings: {
                let mut w = warnings;
                w.extend(warnings_from_plugins);
                w
            },
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
            .route("/validate/policy", post(validate_policy_handler))
            .route("/policy/library", get(policy_library_handler))
            .route("/mutate", post(mutate_handler))
            .route("/db-trigger", post(db_trigger_handler))
            .route("/plugins", get(list_plugins_handler))
            .route("/plugins", post(add_plugin_handler))
            .route(
                "/plugins/{name}",
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
            .map_err(|e| Error::PluginError(format!("Failed to bind to {addr}: {e}")))?;

        axum::serve(listener, app)
            .await
            .map_err(|e| Error::PluginError(format!("Server error: {e}")))?;

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

#[instrument(
    skip(state, review),
    fields(node_name = "-", namespace = "-", reconcile_id = "-")
)]
async fn validate_handler(
    State(state): State<Arc<WebhookServer>>,
    Json(review): Json<AdmissionReview<StellarNode>>,
) -> impl IntoResponse {
    let request = match review.try_into() {
        Ok(req) => req,
        Err(e) => {
            error!("Failed to parse admission request: {e}");
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    AdmissionResponse::invalid(format!("Invalid admission request: {e}"))
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

    if !result.allowed {
        let reason = result.message.as_deref().unwrap_or("Validation failed");
        log_validation_rejection(&req, reason);
    }

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

fn log_validation_rejection(req: &AdmissionRequest<StellarNode>, reason: &str) {
    let name = if req.name.is_empty() {
        req.object
            .as_ref()
            .and_then(|node| node.metadata.name.as_deref())
            .unwrap_or("<unknown>")
    } else {
        req.name.as_str()
    };
    let namespace = req.namespace.as_deref().unwrap_or("<cluster>");
    let sanitized_reason = sanitize_validation_log_message(reason);

    warn!(
        resource_name = %name,
        namespace = %namespace,
        operation = ?req.operation,
        validation_error = %sanitized_reason,
        "Admission request rejected for StellarNode validation"
    );
}

fn sanitize_validation_log_message(message: &str) -> String {
    const MAX_LOGGED_REASON_LEN: usize = 2048;
    const SENSITIVE_KEYS: [&str; 13] = [
        "secret",
        "api_key",
        "apikey",
        "access_key",
        "accesskey",
        "password",
        "passwd",
        "token",
        "credential",
        "privatekey",
        "private_key",
        "mnemonic",
        "secretvalue",
    ];

    let normalized = message.replace(['\n', '\r'], " ");
    let sanitized = normalized
        .split_whitespace()
        .map(|token| sanitize_log_token(token, &SENSITIVE_KEYS))
        .collect::<Vec<_>>()
        .join(" ");

    if sanitized.len() > MAX_LOGGED_REASON_LEN {
        let truncated = sanitized
            .chars()
            .take(MAX_LOGGED_REASON_LEN)
            .collect::<String>();
        format!("{truncated}...<truncated>")
    } else {
        sanitized
    }
}

fn sanitize_log_token(token: &str, sensitive_keys: &[&str]) -> String {
    let lower = token.to_ascii_lowercase();
    let Some(separator_index) = token.find('=').or_else(|| token.find(':')) else {
        return token.to_string();
    };

    let key = &lower[..separator_index];
    if sensitive_keys
        .iter()
        .any(|sensitive| key.contains(sensitive))
    {
        format!("{}=<redacted>", &token[..separator_index])
    } else {
        token.to_string()
    }
}

#[instrument(
    skip(state, review),
    fields(node_name = "-", namespace = "-", reconcile_id = "-")
)]
async fn validate_policy_handler(
    State(state): State<Arc<WebhookServer>>,
    Json(review): Json<AdmissionReview<StellarNode>>,
) -> impl IntoResponse {
    let request = match review.try_into() {
        Ok(req) => req,
        Err(e) => {
            error!("Failed to parse admission request: {e}");
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    AdmissionResponse::invalid(format!("Invalid admission request: {e}"))
                        .into_review(),
                ),
            );
        }
    };

    let req: AdmissionRequest<StellarNode> = request;
    let input = build_validation_input(&req);
    let delegated = state.delegate_policy_check(&input).await;

    if !delegated.allowed {
        let reason = delegated.message.as_deref().unwrap_or("Denied by policy");
        log_validation_rejection(&req, reason);
    }

    let mut response = if delegated.allowed {
        AdmissionResponse::from(&req)
    } else {
        AdmissionResponse::from(&req).deny(
            delegated
                .message
                .unwrap_or_else(|| "Denied by policy".to_string()),
        )
    };

    if !delegated.warnings.is_empty() {
        response.warnings = Some(delegated.warnings);
    }

    (StatusCode::OK, Json(response.into_review()))
}

fn default_security_policy_library() -> Vec<SecurityPolicyInfo> {
    vec![
        SecurityPolicyInfo {
            name: "required-labels".to_string(),
            engine: "gatekeeper".to_string(),
            description: "Enforces required organizational labels on resources.".to_string(),
            path: "config/manifests/gatekeeper/required-labels-template.yaml".to_string(),
        },
        SecurityPolicyInfo {
            name: "approved-registries".to_string(),
            engine: "gatekeeper".to_string(),
            description: "Restricts container images to approved registries.".to_string(),
            path: "config/manifests/gatekeeper/approved-registries-template.yaml".to_string(),
        },
        SecurityPolicyInfo {
            name: "resource-limits".to_string(),
            engine: "gatekeeper".to_string(),
            description: "Requires CPU and memory limits on all containers.".to_string(),
            path: "config/manifests/gatekeeper/resource-limits-template.yaml".to_string(),
        },
        SecurityPolicyInfo {
            name: "stellarnode-cel-validation".to_string(),
            engine: "cel".to_string(),
            description: "Built-in CEL rules in the StellarNode CRD schema.".to_string(),
            path: "config/crd/stellarnode-crd.yaml".to_string(),
        },
    ]
}

async fn policy_library_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(default_security_policy_library()))
}

#[instrument(
    skip(_state, review),
    fields(node_name = "-", namespace = "-", reconcile_id = "-")
)]
async fn mutate_handler(
    State(_state): State<Arc<WebhookServer>>,
    Json(review): Json<AdmissionReview<StellarNode>>,
) -> impl IntoResponse {
    use super::mutation::apply_mutations;

    let request: Result<AdmissionRequest<StellarNode>, _> = review.try_into();

    match request {
        Ok(req) => {
            // Apply mutations to the StellarNode
            match apply_mutations(&req) {
                Ok(Some(patch)) => {
                    let mut response = AdmissionResponse::from(&req);
                    // Convert JSON patch to bytes
                    let patch_bytes = serde_json::to_vec(&patch)
                        .map_err(|e| format!("Failed to serialize patch: {e}"))
                        .unwrap_or_default();
                    response.patch = Some(patch_bytes);

                    info!("Applied mutations to StellarNode {}", req.name);
                    (StatusCode::OK, Json(response.into_review()))
                }
                Ok(None) => {
                    // No mutations needed
                    let response = AdmissionResponse::from(&req);
                    (StatusCode::OK, Json(response.into_review()))
                }
                Err(e) => {
                    error!("Failed to apply mutations: {e}");
                    let response =
                        AdmissionResponse::from(&req).deny(format!("Mutation failed: {e}"));
                    (StatusCode::OK, Json(response.into_review()))
                }
            }
        }
        Err(e) => {
            error!("Failed to parse admission request: {e}");
            (
                StatusCode::BAD_REQUEST,
                Json(
                    AdmissionResponse::invalid(format!("Invalid admission request: {e}"))
                        .into_review(),
                ),
            )
        }
    }
}

#[instrument(
    skip(state, payload),
    fields(node_name = "-", namespace = "-", reconcile_id = "-")
)]
async fn db_trigger_handler(
    State(state): State<Arc<WebhookServer>>,
    Json(payload): Json<super::types::DbTriggerInput>,
) -> impl IntoResponse {
    let plugins = state.plugins.read().await.clone();
    if plugins.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ignored", "message": "No plugins configured"})),
        );
    }

    let mut updated_nodes = Vec::new();
    let mut errors = Vec::new();

    for plugin in plugins {
        if !plugin.enabled || !plugin.operations.contains(&Operation::DbTrigger) {
            continue;
        }

        match state
            .runtime
            .execute_db_trigger(
                &plugin.metadata.name,
                &payload,
                Some(plugin.metadata.limits.clone()),
            )
            .await
        {
            Ok(result) => {
                let output = result.output;
                info!(
                    "DB Trigger plugin {} processed event for node {}/{}",
                    plugin.metadata.name, output.namespace, output.name
                );

                // Initialize Kube client to update status
                match kube::Client::try_default().await {
                    Ok(client) => {
                        let api: kube::Api<StellarNode> =
                            kube::Api::namespaced(client.clone(), &output.namespace);
                        let now = chrono::Utc::now().to_rfc3339();
                        let patch = serde_json::json!({
                            "status": {
                                "ledgerSequence": output.ledger_sequence,
                                "ledgerUpdatedAt": now
                            }
                        });

                        match api
                            .patch_status(
                                &output.name,
                                &kube::api::PatchParams::apply("stellar-operator-reactive"),
                                &kube::api::Patch::Merge(&patch),
                            )
                            .await
                        {
                            Ok(_) => {
                                // Record metrics
                                crate::controller::metrics::inc_reactive_status_update(
                                    &output.namespace,
                                    &output.name,
                                );
                                crate::controller::metrics::inc_api_polls_avoided(
                                    &output.namespace,
                                    &output.name,
                                );
                                updated_nodes.push(output.name.clone());
                            }
                            Err(e) => {
                                error!("Failed to update node status reactively: {e}");
                                errors.push(e.to_string());
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to create Kube client: {e}");
                        errors.push(e.to_string());
                    }
                }
            }
            Err(e) => {
                warn!("Plugin {} failed on db trigger: {e}", plugin.metadata.name);
                errors.push(e.to_string());
            }
        }
    }

    if !errors.is_empty() {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "completed_with_errors", "errors": errors})),
        )
    } else {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "success", "updated_nodes": updated_nodes})),
        )
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
            error!("Failed to add plugin: {e}");
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
            error!("Failed to remove plugin: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        }
    }
}

/// Run built-in StellarNode spec validation. Returns Some(ServerValidationResult) if invalid.
fn validate_spec_builtin(object: &serde_json::Value) -> Option<ServerValidationResult> {
    let node: StellarNode = match serde_json::from_value(object.clone()) {
        Ok(n) => n,
        Err(e) => {
            return Some(ServerValidationResult {
                allowed: false,
                message: Some(format!(
                    "Invalid StellarNode manifest: {e}\n\
                     Hint: Ensure the manifest matches the StellarNode CRD schema (apiVersion: stellar.org/v1alpha1, kind: StellarNode)."
                )),
                warnings: vec![],
                plugin_results: vec![],
                total_execution_time_ms: 0,
            });
        }
    };

    // PSS 'restricted' bypass check — runs before spec validation so security
    // violations are surfaced with a clear, actionable message.
    let pss_violations = crate::controller::pss::validate_pss_compliance(&node.spec);
    if !pss_violations.is_empty() {
        let message = pss_violations
            .iter()
            .map(|v| format!("{}: {}", v.field, v.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Some(ServerValidationResult {
            allowed: false,
            message: Some(format!(
                "PSS 'restricted' violation(s) detected — zero-trust policy forbids these settings: {message}"
            )),
            warnings: vec![],
            plugin_results: vec![],
            total_execution_time_ms: 0,
        });
    }

    // Organizational standards: resource limits, required labels.
    let org_errors = super::org_validator::validate_org_standards(&node);
    if !org_errors.is_empty() {
        let message = org_errors
            .iter()
            .map(|e| format!("[{}] {} — Hint: {}", e.field, e.message, e.hint))
            .collect::<Vec<_>>()
            .join("; ");
        return Some(ServerValidationResult {
            allowed: false,
            message: Some(message),
            warnings: vec![],
            plugin_results: vec![],
            total_execution_time_ms: 0,
        });
    }

    let errors = node.spec.validate().err()?;
    // Format each error as: [spec.field] Message — Hint: how_to_fix
    let message = errors
        .iter()
        .map(|e| format!("[{}] {} — Hint: {}", e.field, e.message, e.how_to_fix))
        .collect::<Vec<_>>()
        .join("; ");
    Some(ServerValidationResult {
        allowed: false,
        message: Some(message),
        warnings: vec![],
        plugin_results: vec![],
        total_execution_time_ms: 0,
    })
}

/// Check if image is pinned by digest and return warnings if not.
fn check_image_pinning(spec: &StellarNodeSpec) -> Vec<String> {
    let mut warnings = Vec::new();
    if !spec.version.contains("@sha256:") {
        if spec.version == "latest" {
            warnings.push(format!(
                "Using mutable tag 'latest' is a security risk. For production, always use an image digest (e.g., 'version: {}@sha256:...') to ensure reproducibility and prevent supply chain attacks.",
                spec.version
            ));
        } else {
            warnings.push(format!(
                "Mutable image tag '{}' used. For production, it is recommended to pin the image by digest (e.g., 'version: {}@sha256:...') for better security.",
                spec.version, spec.version
            ));
        }
    }
    warnings
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
    use super::super::types::PluginLimits;
    use super::*;

    fn default_user_info() -> UserInfo {
        UserInfo {
            username: "test-user".to_string(),
            uid: None,
            groups: vec![],
            extra: BTreeMap::new(),
        }
    }

    fn validation_input(
        operation: Operation,
        object: Option<serde_json::Value>,
    ) -> ValidationInput {
        ValidationInput {
            operation,
            object,
            old_object: None,
            namespace: "default".to_string(),
            name: "test-node".to_string(),
            user_info: default_user_info(),
            context: BTreeMap::new(),
        }
    }

    /// Valid StellarNode spec is admitted (returns Allowed: true)
    #[tokio::test]
    async fn valid_stellarnode_spec_admitted() {
        let runtime = WasmRuntime::new().unwrap();
        let server = WebhookServer::new(runtime);

        let valid_object = serde_json::json!({
            "metadata": {
                "name": "my-validator",
                "namespace": "default",
                "labels": {
                    "project-id": "test",
                    "owner": "test"
                }
            },
            "spec": {
                "nodeType": "Validator",
                "network": "testnet",
                "version": "v21.0.0",
                "replicas": 1,
                "validatorConfig": {
                    "seedSecretRef": "validator-seed",
                    "enableHistoryArchive": false,
                    "historyArchiveUrls": []
                }
            }
        });

        let input = validation_input(Operation::Create, Some(valid_object));
        let result = server.validate(input).await;
        assert!(
            result.allowed,
            "valid spec should be admitted: {:?}",
            result.message
        );
    }

    /// A spec with an invalid nodeType is rejected with a descriptive message
    #[tokio::test]
    async fn invalid_node_type_rejected() {
        let runtime = WasmRuntime::new().unwrap();
        let server = WebhookServer::new(runtime);

        let invalid_object = serde_json::json!({
            "metadata": {
                "name": "bad",
                "namespace": "default",
                "labels": {
                    "project-id": "test",
                    "owner": "test"
                }
            },
            "spec": {
                "nodeType": "InvalidType",
                "network": "testnet",
                "version": "v21.0.0"
            }
        });

        let input = validation_input(Operation::Create, Some(invalid_object));
        let result = server.validate(input).await;
        assert!(!result.allowed);
        let msg = result.message.unwrap_or_default();
        assert!(
            msg.contains("Invalid")
                || msg.contains("nodeType")
                || msg.contains("parse")
                || msg.contains("unknown"),
            "expected descriptive rejection message, got: {msg}"
        );
    }

    /// A spec missing required fields is rejected
    #[tokio::test]
    async fn missing_required_fields_rejected() {
        let runtime = WasmRuntime::new().unwrap();
        let server = WebhookServer::new(runtime);

        let missing_required = serde_json::json!({
            "metadata": {
                "name": "no-config",
                "namespace": "default",
                "labels": {
                    "project-id": "test",
                    "owner": "test"
                }
            },
            "spec": {
                "nodeType": "Validator",
                "network": "testnet",
                "version": "v21.0.0",
                "replicas": 1
            }
        });

        let input = validation_input(Operation::Create, Some(missing_required));
        let result = server.validate(input).await;
        assert!(!result.allowed);
        let msg = result.message.unwrap_or_default();
        assert!(
            msg.contains("validatorConfig") || msg.contains("required"),
            "expected message about missing required field, got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_webhook_server_creation() {
        let runtime = WasmRuntime::new().unwrap();
        let server = WebhookServer::new(runtime);
        assert!(server.list_plugins().await.is_empty());
    }

    /// With no plugins loaded, a valid StellarNode spec is still admitted by built-in validation.
    #[tokio::test]
    async fn test_validation_no_plugins() {
        let runtime = WasmRuntime::new().unwrap();
        let server = WebhookServer::new(runtime);

        let valid_object = serde_json::json!({
            "metadata": {
                "name": "my-validator",
                "namespace": "default",
                "labels": {
                    "project-id": "test",
                    "owner": "test"
                }
            },
            "spec": {
                "nodeType": "Validator",
                "network": "testnet",
                "version": "v21.0.0",
                "replicas": 1,
                "validatorConfig": {
                    "seedSecretRef": "validator-seed",
                    "enableHistoryArchive": false,
                    "historyArchiveUrls": []
                }
            }
        });
        let input = validation_input(Operation::Create, Some(valid_object));
        let result = server.validate(input).await;
        assert!(
            result.allowed,
            "valid spec with no plugins should be admitted: {:?}",
            result.message
        );
    }

    /// Wasm plugin that traps is handled gracefully (operator doesn't crash, returns denied or fail-open)
    #[tokio::test]
    async fn wasm_plugin_trap_handled_gracefully() {
        let runtime = WasmRuntime::new().unwrap();
        let server = WebhookServer::new(runtime);

        let wasm = wat::parse_str(
            r#"
            (module
                (func (export "validate") unreachable)
                (memory (export "memory") 1)
            )
            "#,
        )
        .unwrap();

        let config = PluginConfig {
            metadata: PluginMetadata {
                name: "trap-plugin".to_string(),
                version: "0.0.1".to_string(),
                description: None,
                author: None,
                sha256: None,
                limits: PluginLimits::default(),
            },
            wasm_binary: Some(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &wasm,
            )),
            config_map_ref: None,
            secret_ref: None,
            url: None,
            operations: vec![Operation::Create],
            enabled: true,
            fail_open: false,
            plugin_config: BTreeMap::new(),
        };

        server.add_plugin(config).await.unwrap();

        let valid_object = serde_json::json!({
            "metadata": {
                "name": "test",
                "namespace": "default",
                "labels": {
                    "project-id": "test",
                    "owner": "test"
                }
            },
            "spec": {
                "nodeType": "Validator",
                "network": "testnet",
                "version": "v21.0.0",
                "replicas": 1,
                "validatorConfig": {
                    "seedSecretRef": "x",
                    "enableHistoryArchive": false,
                    "historyArchiveUrls": []
                }
            }
        });

        let input = validation_input(Operation::Create, Some(valid_object));
        let result = server.validate(input).await;

        assert!(
            !result.allowed,
            "trap plugin should cause denial when fail_open is false"
        );
        assert!(
            result
                .message
                .as_ref()
                .map(|m| m.contains("Plugin")
                    || m.contains("trap")
                    || m.contains("execution")
                    || m.contains("unreachable"))
                .unwrap_or(false),
            "expected plugin error message, got: {:?}",
            result.message
        );
    }

    /// Wasm plugin that traps with fail_open still returns a result (allowed with warning, no crash)
    #[tokio::test]
    async fn wasm_plugin_trap_fail_open_allowed_with_warning() {
        let runtime = WasmRuntime::new().unwrap();
        let server = WebhookServer::new(runtime);

        let wasm = wat::parse_str(
            r#"
            (module
                (func (export "validate") unreachable)
                (memory (export "memory") 1)
            )
            "#,
        )
        .unwrap();

        let config = PluginConfig {
            metadata: PluginMetadata {
                name: "trap-fail-open".to_string(),
                version: "0.0.1".to_string(),
                description: None,
                author: None,
                sha256: None,
                limits: PluginLimits::default(),
            },
            wasm_binary: Some(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &wasm,
            )),
            config_map_ref: None,
            secret_ref: None,
            url: None,
            operations: vec![Operation::Create],
            enabled: true,
            fail_open: true,
            plugin_config: BTreeMap::new(),
        };

        server.add_plugin(config).await.unwrap();

        let valid_object = serde_json::json!({
            "metadata": {
                "name": "test",
                "namespace": "default",
                "labels": {
                    "project-id": "test",
                    "owner": "test"
                }
            },
            "spec": {
                "nodeType": "Validator",
                "network": "testnet",
                "version": "v21.0.0",
                "replicas": 1,
                "validatorConfig": {
                    "seedSecretRef": "x",
                    "enableHistoryArchive": false,
                    "historyArchiveUrls": []
                }
            }
        });

        let input = validation_input(Operation::Create, Some(valid_object));
        let result = server.validate(input).await;

        assert!(result.allowed, "fail_open should allow when plugin traps");
        assert!(
            !result.warnings.is_empty(),
            "expected warning about plugin failure"
        );
    }

    #[test]
    fn policy_library_contains_gatekeeper_and_cel_entries() {
        let policies = default_security_policy_library();
        assert!(
            policies.iter().any(|p| p.engine == "gatekeeper"),
            "expected at least one gatekeeper policy"
        );
        assert!(
            policies.iter().any(|p| p.engine == "cel"),
            "expected at least one CEL policy"
        );
    }

    #[test]
    fn validation_log_sanitizer_preserves_field_errors() {
        let message =
            "[spec.nodeType] nodeType must be one of Validator, Horizon, SorobanRpc - Hint: fix it";

        let sanitized = sanitize_validation_log_message(message);

        assert!(sanitized.contains("spec.nodeType"));
        assert!(sanitized.contains("nodeType must be one of"));
    }

    #[test]
    fn validation_log_sanitizer_redacts_inline_sensitive_values() {
        let message = "validation failed: token=abc123 password:super-secret clientSecret=secret-value field=spec.version";

        let sanitized = sanitize_validation_log_message(message);

        assert!(sanitized.contains("token=<redacted>"));
        assert!(sanitized.contains("password=<redacted>"));
        assert!(sanitized.contains("clientSecret=<redacted>"));
        assert!(sanitized.contains("field=spec.version"));
        assert!(!sanitized.contains("abc123"));
        assert!(!sanitized.contains("super-secret"));
        assert!(!sanitized.contains("secret-value"));
    }
}
