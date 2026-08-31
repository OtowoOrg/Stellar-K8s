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
//! Comprehensive API contract tests for the Stellar-K8s REST API (#1396)
//!
//! Validates that all API endpoints produce responses conforming to the
//! OpenAPI 3.0 specification at `docs/api/openapi.yaml`.
//!
//! These tests use a lightweight mock router (no K8s cluster required)
//! with sample response payloads that match the actual handler return types.
//! Each response is validated against the corresponding JSON Schema from the
//! OpenAPI spec.
//!
//! ```bash
//! cargo test --test api_contract_tests
//! ```

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use std::sync::LazyLock;
use tower::ServiceExt;

// ── OpenAPI spec loading ──────────────────────────────────────────────────────

static OPENAPI_SPEC: LazyLock<Value> = LazyLock::new(|| {
    let spec_bytes = include_bytes!("../docs/api/openapi.yaml");
    serde_yaml::from_slice(spec_bytes).expect("Failed to parse OpenAPI spec")
});

fn get_schema(schema_ref: &str) -> Value {
    let ref_path = schema_ref.trim_start_matches("#/components/schemas/");
    OPENAPI_SPEC["components"]["schemas"][ref_path]
        .clone()
        .unwrap_or_else(|| panic!("Schema not found: {schema_ref}"))
}

fn resolve_schema(schema: &Value) -> Value {
    if let Some(ref_str) = schema.get("$ref").and_then(|v| v.as_str()) {
        get_schema(ref_str)
    } else {
        schema.clone()
    }
}

fn get_response_schema(method: &str, path: &str, status: &str) -> Option<Value> {
    let path_item = OPENAPI_SPEC["paths"].get(path)?;
    let operation = path_item.get(method.to_lowercase())?;
    let response = operation["responses"].get(status)?;
    let content = response.get("content")?;
    let json_content = content.get("application/json")?;
    let schema = json_content.get("schema")?;
    Some(resolve_schema(schema))
}

/// Basic JSON Schema validation using serde_json.
///
/// Validates that a JSON value conforms to a JSON Schema definition.
/// This is a simplified validator that handles the schemas used in the
/// Stellar-K8s OpenAPI spec (objects, arrays, primitives, required fields,
/// enum values, and $ref resolution).
fn validate_response(json: &Value, schema: &Value) -> Result<(), String> {
    // Resolve $ref if present
    let schema = resolve_schema(schema);

    match schema.get("type").and_then(|t| t.as_str()) {
        Some("object") => validate_object(json, &schema),
        Some("array") => validate_array(json, &schema),
        Some("string") => validate_string(json, &schema),
        Some("integer") => validate_integer(json, &schema),
        Some("number") => validate_number(json, &schema),
        Some("boolean") => {
            if !json.is_boolean() {
                return Err(format!("Expected boolean, got {}", json_type(json)));
            }
            Ok(())
        }
        None => {
            // No type specified - check for enum or other constraints
            if let Some(enum_values) = schema.get("enum") {
                validate_enum(json, enum_values)
            } else {
                Ok(()) // Permissive: accept any value if no type constraint
            }
        }
        other => Err(format!("Unsupported schema type: {:?}", other)),
    }
}

fn validate_object(json: &Value, schema: &Value) -> Result<(), String> {
    if !json.is_object() {
        return Err(format!("Expected object, got {}", json_type(json)));
    }

    // Check required fields
    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        for field in required {
            let field_name = field.as_str().unwrap_or("");
            if json.get(field_name).is_none() {
                return Err(format!("Missing required field: {field_name}"));
            }
        }
    }

    // Validate properties if defined
    if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
        if let Some(obj) = json.as_object() {
            for (key, value) in obj {
                if let Some(prop_schema) = properties.get(key) {
                    validate_response(value, prop_schema)?;
                }
            }
        }
    }

    Ok(())
}

fn validate_array(json: &Value, schema: &Value) -> Result<(), String> {
    if !json.is_array() {
        return Err(format!("Expected array, got {}", json_type(json)));
    }

    if let Some(items_schema) = schema.get("items") {
        if let Some(arr) = json.as_array() {
            for (i, item) in arr.iter().enumerate() {
                validate_response(item, items_schema)
                    .map_err(|e| format!("Array item {i}: {e}"))?;
            }
        }
    }

    Ok(())
}

fn validate_string(json: &Value, schema: &Value) -> Result<(), String> {
    if !json.is_string() {
        return Err(format!("Expected string, got {}", json_type(json)));
    }

    // Check enum values
    if let Some(enum_values) = schema.get("enum") {
        validate_enum(json, enum_values)?;
    }

    Ok(())
}

fn validate_integer(json: &Value, schema: &Value) -> Result<(), String> {
    if !json.is_number() {
        return Err(format!("Expected integer, got {}", json_type(json)));
    }

    if let Some(num) = json.as_f64() {
        if num.fract() != 0.0 {
            return Err(format!("Expected integer, got float: {num}"));
        }

        // Check minimum
        if let Some(min) = schema.get("minimum").and_then(|m| m.as_f64()) {
            if num < min {
                return Err(format!("Value {num} is less than minimum {min}"));
            }
        }

        // Check maximum
        if let Some(max) = schema.get("maximum").and_then(|m| m.as_f64()) {
            if num > max {
                return Err(format!("Value {num} is greater than maximum {max}"));
            }
        }
    }

    Ok(())
}

fn validate_number(json: &Value, schema: &Value) -> Result<(), String> {
    if !json.is_number() {
        return Err(format!("Expected number, got {}", json_type(json)));
    }

    if let Some(num) = json.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(|m| m.as_f64()) {
            if num < min {
                return Err(format!("Value {num} is less than minimum {min}"));
            }
        }
        if let Some(max) = schema.get("maximum").and_then(|m| m.as_f64()) {
            if num > max {
                return Err(format!("Value {num} is greater than maximum {max}"));
            }
        }
    }

    Ok(())
}

fn validate_enum(json: &Value, enum_values: &Value) -> Result<(), String> {
    if let Some(arr) = enum_values.as_array() {
        if !arr.contains(json) {
            return Err(format!(
                "Value {} is not in enum: {:?}",
                json,
                arr.iter()
                    .map(|v| v.as_str().unwrap_or("?"))
                    .collect::<Vec<_>>()
            ));
        }
    }
    Ok(())
}

fn json_type(json: &Value) -> &'static str {
    match json {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ── Mock handlers returning sample responses ──────────────────────────────────

fn mock_health() -> Value {
    json!({
        "status": "healthy",
        "version": "0.1.0"
    })
}

fn mock_probe() -> Value {
    json!({
        "status": "ok",
        "reason": null
    })
}

fn mock_version_catalog() -> Value {
    json!({
        "canonicalScheme": "url_path",
        "current": "v1",
        "versions": [
            {
                "id": "v1",
                "status": "current",
                "basePath": "/api/v1",
                "sunset": null
            }
        ]
    })
}

fn mock_leader() -> Value {
    json!({
        "isLeader": true,
        "holderId": "operator-0"
    })
}

fn mock_node_list() -> Value {
    json!({
        "items": [
            {
                "name": "validator-1",
                "namespace": "stellar",
                "nodeType": "Validator",
                "network": "testnet",
                "phase": "Running",
                "replicas": 1,
                "readyReplicas": 1
            }
        ],
        "total": 1
    })
}

fn mock_node_detail() -> Value {
    json!({
        "name": "validator-1",
        "namespace": "stellar",
        "nodeType": "Validator",
        "network": "testnet",
        "version": "v21.0.0",
        "status": {},
        "createdAt": "2024-01-01T00:00:00Z"
    })
}

fn mock_log_level_response() -> Value {
    json!({
        "currentLevel": "info",
        "expiresAt": null,
        "message": "Log level status"
    })
}

fn mock_error_response() -> Value {
    json!({
        "error": "err_not_found",
        "errorCode": "ERR_NOT_FOUND",
        "message": "Resource not found",
        "correlationId": null,
        "details": null,
        "degraded": false,
        "timestamp": "2024-01-01T00:00:00Z"
    })
}

fn mock_dashboard_overview() -> Value {
    json!({
        "totalNodes": 5,
        "healthyNodes": 4,
        "syncingNodes": 1,
        "unhealthyNodes": 0,
        "nodesByType": {
            "validators": 3,
            "horizon": 1,
            "soroban": 1
        },
        "nodesByNetwork": {
            "mainnet": 2,
            "testnet": 2,
            "futurenet": 1,
            "custom": 0
        }
    })
}

fn mock_log_analytics() -> Value {
    json!({
        "topPatterns": [
            {
                "template": "Reconciled {}",
                "count": 42,
                "lastSeen": "2024-01-01T00:00:00Z"
            }
        ]
    })
}

fn mock_security_posture() -> Value {
    json!({
        "posture": {
            "score": 85,
            "level": "good"
        }
    })
}

fn mock_capacity_planning() -> Value {
    json!({
        "recommendations": [],
        "forecasts": [],
        "bottlenecks": []
    })
}

fn mock_node_logs() -> Value {
    json!({
        "namespace": "stellar",
        "name": "validator-1",
        "podName": "validator-1-0",
        "logs": "2024-01-01 INFO Starting...",
        "timestamp": "2024-01-01T00:00:00Z"
    })
}

fn mock_node_conditions() -> Value {
    json!({
        "namespace": "stellar",
        "name": "validator-1",
        "conditions": [
            {
                "conditionType": "Ready",
                "status": "True",
                "reason": "PodReady",
                "message": "Pod is ready",
                "lastTransitionTime": "2024-01-01T00:00:00Z",
                "severity": "success"
            }
        ]
    })
}

fn mock_dr_status() -> Value {
    json!({
        "namespace": "stellar",
        "name": "validator-1",
        "drEnabled": false,
        "currentRole": null,
        "failoverActive": false,
        "lastFailoverTime": null,
        "syncLag": null,
        "complianceStatus": null,
        "lastDrillResult": null
    })
}

fn mock_metrics_summary() -> Value {
    json!({
        "namespace": "stellar",
        "name": "validator-1",
        "ledgerSequence": 12345,
        "readyReplicas": 1,
        "replicas": 1,
        "quorumFragility": 0.1
    })
}

fn mock_node_action_response() -> Value {
    json!({
        "success": true,
        "message": "Action restart initiated",
        "action": "restart"
    })
}

fn mock_operator_logs() -> Value {
    json!({
        "logs": ["2024-01-01 INFO Operator started"],
        "timestamp": "2024-01-01T00:00:00Z"
    })
}

fn mock_job_list() -> Value {
    json!({
        "items": [],
        "total": 0
    })
}

fn mock_job_stats() -> Value {
    json!({
        "pending": 0,
        "running": 1,
        "succeeded": 5,
        "failed": 0,
        "cancelled": 0,
        "totalRegistered": 6
    })
}

fn mock_audit_log() -> Value {
    json!({
        "items": [],
        "total": 0
    })
}

fn mock_audit_anomalies() -> Value {
    json!({
        "items": [],
        "total": 0
    })
}

fn mock_config_impact() -> Value {
    json!({
        "impact": {},
        "validationErrors": []
    })
}

// ── Build mock router ─────────────────────────────────────────────────────────

fn build_mock_router() -> Router {
    Router::new()
        .route("/health", get(|| async { axum::Json(mock_health()) }))
        .route("/healthz", get(|| async { axum::Json(mock_probe()) }))
        .route("/readyz", get(|| async { axum::Json(mock_probe()) }))
        .route("/livez", get(|| async { axum::Json(mock_probe()) }))
        .route(
            "/api/versions",
            get(|| async { axum::Json(mock_version_catalog()) }),
        )
        .route("/leader", get(|| async { axum::Json(mock_leader()) }))
        .route(
            "/api/v1/nodes",
            get(|| async { axum::Json(mock_node_list()) }),
        )
        .route(
            "/api/v1/nodes/:namespace/:name",
            get(|| async { axum::Json(mock_node_detail()) }),
        )
        .route(
            "/config/log-level",
            get(|| async { axum::Json(mock_log_level_response()) }),
        )
        .route(
            "/api/v1/compliance/report",
            get(|| async { axum::Json(json!([])) }),
        )
        .route(
            "/api/v1/compliance/status",
            get(|| async { axum::Json(json!({})) }),
        )
        .route(
            "/api/v1/compliance/regulatory-report",
            get(|| async { axum::Json(json!({})) }),
        )
        .route(
            "/api/v1/horizon/cache/status",
            get(|| async { axum::Json(json!({})) }),
        )
        .route(
            "/api/v1/dashboard/overview",
            get(|| async { axum::Json(mock_dashboard_overview()) }),
        )
        .route(
            "/api/v1/dashboard/metrics",
            get(|| async { axum::Json(json!({})) }),
        )
        .route(
            "/api/v1/analytics/logs",
            get(|| async { axum::Json(mock_log_analytics()) }),
        )
        .route(
            "/api/v1/security/posture",
            get(|| async { axum::Json(mock_security_posture()) }),
        )
        .route(
            "/api/v1/capacity/plan",
            get(|| async { axum::Json(mock_capacity_planning()) }),
        )
        .route(
            "/api/v1/optimization/recommendations",
            get(|| async { axum::Json(json!({})) }),
        )
        .route(
            "/api/v1/optimization/forecast",
            get(|| async { axum::Json(json!({})) }),
        )
        .route(
            "/api/v1/traffic/dashboard",
            get(|| async { axum::Json(json!({})) }),
        )
        .route(
            "/api/v1/dashboard/nodes/:namespace/:name/logs",
            get(|| async { axum::Json(mock_node_logs()) }),
        )
        .route(
            "/api/v1/dashboard/nodes/:namespace/:name/conditions",
            get(|| async { axum::Json(mock_node_conditions()) }),
        )
        .route(
            "/api/v1/dashboard/nodes/:namespace/:name/dr",
            get(|| async { axum::Json(mock_dr_status()) }),
        )
        .route(
            "/api/v1/dashboard/nodes/:namespace/:name/metrics",
            get(|| async { axum::Json(mock_metrics_summary()) }),
        )
        .route(
            "/api/v1/dashboard/operator/logs",
            get(|| async { axum::Json(mock_operator_logs()) }),
        )
        .route(
            "/api/v1/quorum/topology",
            get(|| async { axum::Json(json!({})) }),
        )
        .route("/api/v1/docs/search-index", get(|| async { "[]" }))
        .route(
            "/api/v1/jobs",
            get(|| async { axum::Json(mock_job_list()) }),
        )
        .route(
            "/api/v1/jobs/stats",
            get(|| async { axum::Json(mock_job_stats()) }),
        )
        .route(
            "/api/v1/audit-log",
            get(|| async { axum::Json(mock_audit_log()) }),
        )
        .route(
            "/api/v1/audit-log/search",
            get(|| async { axum::Json(mock_audit_log()) }),
        )
        .route(
            "/api/v1/audit-log/anomalies",
            get(|| async { axum::Json(mock_audit_anomalies()) }),
        )
}

async fn get_json(router: &Router, path: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(json!(""));
    (status, body)
}

// ── Contract tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn contract_health_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/health").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/health", "200").unwrap();
    validate_response(&body, &schema).expect("HealthResponse schema mismatch");
}

#[tokio::test]
async fn contract_healthz_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/healthz", "200").unwrap();
    validate_response(&body, &schema).expect("ProbeResponse schema mismatch");
}

#[tokio::test]
async fn contract_readyz_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/readyz").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/readyz", "200").unwrap();
    validate_response(&body, &schema).expect("ProbeResponse schema mismatch");
}

#[tokio::test]
async fn contract_livez_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/livez").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/livez", "200").unwrap();
    validate_response(&body, &schema).expect("ProbeResponse schema mismatch");
}

#[tokio::test]
async fn contract_versions_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/versions").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/versions", "200").unwrap();
    validate_response(&body, &schema).expect("VersionCatalog schema mismatch");
}

#[tokio::test]
async fn contract_leader_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/leader").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/leader", "200").unwrap();
    validate_response(&body, &schema).expect("LeaderResponse schema mismatch");
}

#[tokio::test]
async fn contract_node_list_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/nodes").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/nodes", "200").unwrap();
    validate_response(&body, &schema).expect("NodeListResponse schema mismatch");
}

#[tokio::test]
async fn contract_node_detail_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/nodes/stellar/validator-1").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/nodes/{namespace}/{name}", "200").unwrap();
    validate_response(&body, &schema).expect("NodeDetailResponse schema mismatch");
}

#[tokio::test]
async fn contract_log_level_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/config/log-level").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/config/log-level", "200").unwrap();
    validate_response(&body, &schema).expect("LogLevelResponse schema mismatch");
}

#[tokio::test]
async fn contract_dashboard_overview_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/dashboard/overview").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/dashboard/overview", "200").unwrap();
    validate_response(&body, &schema).expect("DashboardOverview schema mismatch");
}

#[tokio::test]
async fn contract_log_analytics_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/analytics/logs").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/analytics/logs", "200").unwrap();
    validate_response(&body, &schema).expect("LogAnalyticsResponse schema mismatch");
}

#[tokio::test]
async fn contract_security_posture_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/security/posture").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/security/posture", "200").unwrap();
    validate_response(&body, &schema).expect("SecurityPostureResponse schema mismatch");
}

#[tokio::test]
async fn contract_capacity_planning_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/capacity/plan").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/capacity/plan", "200").unwrap();
    validate_response(&body, &schema).expect("CapacityPlanningResponse schema mismatch");
}

#[tokio::test]
async fn contract_node_logs_response_matches_schema() {
    let (status, body) = get_json(
        &build_mock_router(),
        "/api/v1/dashboard/nodes/stellar/validator-1/logs",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema(
        "get",
        "/api/v1/dashboard/nodes/{namespace}/{name}/logs",
        "200",
    )
    .unwrap();
    validate_response(&body, &schema).expect("NodeLogsResponse schema mismatch");
}

#[tokio::test]
async fn contract_node_conditions_response_matches_schema() {
    let (status, body) = get_json(
        &build_mock_router(),
        "/api/v1/dashboard/nodes/stellar/validator-1/conditions",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema(
        "get",
        "/api/v1/dashboard/nodes/{namespace}/{name}/conditions",
        "200",
    )
    .unwrap();
    validate_response(&body, &schema).expect("NodeConditionsResponse schema mismatch");
}

#[tokio::test]
async fn contract_dr_status_response_matches_schema() {
    let (status, body) = get_json(
        &build_mock_router(),
        "/api/v1/dashboard/nodes/stellar/validator-1/dr",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema(
        "get",
        "/api/v1/dashboard/nodes/{namespace}/{name}/dr",
        "200",
    )
    .unwrap();
    validate_response(&body, &schema).expect("DRStatusResponse schema mismatch");
}

#[tokio::test]
async fn contract_node_metrics_response_matches_schema() {
    let (status, body) = get_json(
        &build_mock_router(),
        "/api/v1/dashboard/nodes/stellar/validator-1/metrics",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema(
        "get",
        "/api/v1/dashboard/nodes/{namespace}/{name}/metrics",
        "200",
    )
    .unwrap();
    validate_response(&body, &schema).expect("MetricsSummary schema mismatch");
}

#[tokio::test]
async fn contract_operator_logs_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/dashboard/operator/logs").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/dashboard/operator/logs", "200").unwrap();
    validate_response(&body, &schema).expect("OperatorLogsResponse schema mismatch");
}

#[tokio::test]
async fn contract_job_list_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/jobs").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/jobs", "200").unwrap();
    validate_response(&body, &schema).expect("JobListResponse schema mismatch");
}

#[tokio::test]
async fn contract_job_stats_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/jobs/stats").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/jobs/stats", "200").unwrap();
    validate_response(&body, &schema).expect("JobStatsResponse schema mismatch");
}

#[tokio::test]
async fn contract_audit_log_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/audit-log").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/audit-log", "200").unwrap();
    validate_response(&body, &schema).expect("AuditLogResponse schema mismatch");
}

#[tokio::test]
async fn contract_audit_search_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/audit-log/search").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/audit-log/search", "200").unwrap();
    validate_response(&body, &schema).expect("AuditLogResponse schema mismatch");
}

#[tokio::test]
async fn contract_audit_anomalies_response_matches_schema() {
    let (status, body) = get_json(&build_mock_router(), "/api/v1/audit-log/anomalies").await;
    assert_eq!(status, StatusCode::OK);
    let schema = get_response_schema("get", "/api/v1/audit-log/anomalies", "200").unwrap();
    validate_response(&body, &schema).expect("AuditAnomalyResponse schema mismatch");
}

#[tokio::test]
async fn contract_error_response_matches_schema() {
    let error_body = mock_error_response();
    let schema = get_response_schema("get", "/api/v1/nodes/{namespace}/{name}", "404").unwrap();
    validate_response(&error_body, &schema).expect("ErrorResponse schema mismatch");
}

// ── Schema completeness tests ─────────────────────────────────────────────────

#[test]
fn openapi_spec_has_all_expected_paths() {
    let spec = &*OPENAPI_SPEC;
    let paths = spec["paths"]
        .as_object()
        .expect("paths should be an object");
    let expected = [
        "/health",
        "/healthz",
        "/readyz",
        "/livez",
        "/api/versions",
        "/leader",
        "/api/v1/nodes",
        "/api/v1/nodes/{namespace}/{name}",
        "/config/log-level",
        "/api/v1/compliance/report",
        "/api/v1/compliance/status",
        "/api/v1/compliance/regulatory-report",
        "/api/v1/horizon/cache/status",
        "/api/v1/dashboard/overview",
        "/api/v1/dashboard/metrics",
        "/api/v1/analytics/logs",
        "/api/v1/config/analyze",
        "/api/v1/security/posture",
        "/api/v1/capacity/plan",
        "/api/v1/capacity/what-if",
        "/api/v1/optimization/recommendations",
        "/api/v1/optimization/simulate",
        "/api/v1/optimization/forecast",
        "/api/v1/traffic/dashboard",
        "/api/v1/dashboard/nodes/{namespace}/{name}/logs",
        "/api/v1/dashboard/nodes/{namespace}/{name}/conditions",
        "/api/v1/dashboard/nodes/{namespace}/{name}/dr",
        "/api/v1/dashboard/nodes/{namespace}/{name}/metrics",
        "/api/v1/dashboard/nodes/{namespace}/{name}/actions",
        "/api/v1/dashboard/operator/logs",
        "/api/v1/quorum/topology",
        "/api/v1/docs/search-index",
        "/api/v1/jobs",
        "/api/v1/jobs/stats",
        "/api/v1/audit-log",
        "/api/v1/audit-log/search",
        "/api/v1/audit-log/anomalies",
        "/metrics",
    ];
    let mut missing = Vec::new();
    for path in &expected {
        if !paths.contains_key(*path) {
            missing.push(path.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "Missing paths in OpenAPI spec: {missing:?}"
    );
}

#[test]
fn openapi_spec_schemas_have_required_fields() {
    let spec = &*OPENAPI_SPEC;
    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("schemas should be an object");

    let must_have_required = [
        "HealthResponse",
        "ProbeResponse",
        "VersionCatalog",
        "LeaderResponse",
        "LogLevelRequest",
        "ErrorResponse",
        "NodeLogsResponse",
        "NodeConditionsResponse",
        "DRStatusResponse",
        "NodeActionRequest",
        "NodeActionResponse",
        "OperatorLogsResponse",
        "AuditLogResponse",
        "AuditAnomalyResponse",
    ];

    for name in &must_have_required {
        let schema = schemas
            .get(*name)
            .unwrap_or_else(|| panic!("Schema {name} not found"));
        assert!(
            schema.get("required").is_some(),
            "Schema {name} is missing required fields array"
        );
        let required = schema["required"]
            .as_array()
            .expect("required should be an array");
        assert!(
            !required.is_empty(),
            "Schema {name} has empty required fields"
        );
    }
}

#[test]
fn openapi_spec_version_info_enum_values() {
    let spec = &*OPENAPI_SPEC;
    let version_info = &spec["components"]["schemas"]["VersionInfo"];
    let status_enum = version_info["properties"]["status"]["enum"]
        .as_array()
        .expect("VersionInfo.status should have enum");
    assert!(status_enum.contains(&json!("current")));
    assert!(status_enum.contains(&json!("deprecated")));
    assert!(status_enum.contains(&json!("sunset")));
}

#[test]
fn openapi_spec_node_type_enum_values() {
    let spec = &*OPENAPI_SPEC;
    let node_summary = &spec["components"]["schemas"]["NodeSummary"];
    let node_type_enum = node_summary["properties"]["nodeType"]["enum"]
        .as_array()
        .expect("NodeSummary.nodeType should have enum");
    assert!(node_type_enum.contains(&json!("Validator")));
    assert!(node_type_enum.contains(&json!("Horizon")));
    assert!(node_type_enum.contains(&json!("SorobanRpc")));
}

#[test]
fn openapi_spec_action_enum_values() {
    let spec = &*OPENAPI_SPEC;
    let action_request = &spec["components"]["schemas"]["NodeActionRequest"];
    let action_enum = action_request["properties"]["action"]["enum"]
        .as_array()
        .expect("NodeActionRequest.action should have enum");
    assert!(action_enum.contains(&json!("restart")));
    assert!(action_enum.contains(&json!("snapshot")));
    assert!(action_enum.contains(&json!("suspend")));
    assert!(action_enum.contains(&json!("resume")));
    assert!(action_enum.contains(&json!("maintenance_mode")));
    assert!(action_enum.contains(&json!("prune")));
}

#[test]
fn openapi_spec_condition_severity_enum_values() {
    let spec = &*OPENAPI_SPEC;
    let condition_display = &spec["components"]["schemas"]["ConditionDisplay"];
    let severity_enum = condition_display["properties"]["severity"]["enum"]
        .as_array()
        .expect("ConditionDisplay.severity should have enum");
    assert!(severity_enum.contains(&json!("success")));
    assert!(severity_enum.contains(&json!("warning")));
    assert!(severity_enum.contains(&json!("error")));
    assert!(severity_enum.contains(&json!("info")));
}

// ── Coverage summary ──────────────────────────────────────────────────────────

#[test]
fn api_endpoint_coverage_report() {
    let spec = &*OPENAPI_SPEC;
    let paths = spec["paths"].as_object().unwrap();

    let mut total_endpoints = 0;
    let mut documented_with_schema = 0;
    let mut documented_with_error = 0;
    let mut documented_with_auth = 0;

    let expected_endpoints = [
        ("get", "/health", false),
        ("get", "/healthz", false),
        ("get", "/readyz", false),
        ("get", "/livez", false),
        ("get", "/api/versions", false),
        ("get", "/leader", true),
        ("get", "/api/v1/nodes", true),
        ("get", "/api/v1/nodes/{namespace}/{name}", true),
        ("get", "/config/log-level", true),
        ("post", "/config/log-level", true),
        ("get", "/api/v1/compliance/report", true),
        ("get", "/api/v1/compliance/status", true),
        ("get", "/api/v1/compliance/regulatory-report", true),
        ("get", "/api/v1/horizon/cache/status", true),
        ("get", "/api/v1/dashboard/overview", true),
        ("get", "/api/v1/dashboard/metrics", true),
        ("get", "/api/v1/analytics/logs", true),
        ("post", "/api/v1/config/analyze", true),
        ("get", "/api/v1/security/posture", true),
        ("get", "/api/v1/capacity/plan", true),
        ("post", "/api/v1/capacity/what-if", true),
        ("get", "/api/v1/optimization/recommendations", true),
        ("post", "/api/v1/optimization/simulate", true),
        ("get", "/api/v1/optimization/forecast", true),
        ("get", "/api/v1/traffic/dashboard", true),
        ("get", "/api/v1/dashboard/nodes/{namespace}/{name}/logs", true),
        ("get", "/api/v1/dashboard/nodes/{namespace}/{name}/conditions", true),
        ("get", "/api/v1/dashboard/nodes/{namespace}/{name}/dr", true),
        ("get", "/api/v1/dashboard/nodes/{namespace}/{name}/metrics", true),
        ("post", "/api/v1/dashboard/nodes/{namespace}/{name}/actions", true),
        ("get", "/api/v1/dashboard/operator/logs", true),
        ("get", "/api/v1/quorum/topology", true),
        ("get", "/api/v1/docs/search-index", false),
        ("get", "/api/v1/jobs", true),
        ("get", "/api/v1/jobs/stats", true),
        ("get", "/api/v1/audit-log", true),
        ("get", "/api/v1/audit-log/search", true),
        ("get", "/api/v1/audit-log/anomalies", true),
        ("get", "/metrics", false),
    ];

    for (method, path, requires_auth) in &expected_endpoints {
        total_endpoints += 1;
        if let Some(path_item) = paths.get(*path) {
            if let Some(operation) = path_item.get(*method) {
                let has_200_schema = operation["responses"]["200"]["content"]
                    .get("application/json")
                    .and_then(|c| c.get("schema"))
                    .is_some();
                if has_200_schema {
                    documented_with_schema += 1;
                }

                let has_error = operation["responses"]
                    .as_object()
                    .map(|r| r.keys().any(|k| k.starts_with('4') || k.starts_with('5')))
                    .unwrap_or(false);
                if has_error {
                    documented_with_error += 1;
                }

                let has_auth = operation.get("security").is_some();
                if *requires_auth && has_auth {
                    documented_with_auth += 1;
                } else if !requires_auth && !has_auth {
                    documented_with_auth += 1;
                }
            }
        }
    }

    let coverage_pct = (documented_with_schema as f64 / total_endpoints as f64) * 100.0;
    println!("\n=== API Contract Coverage Report ===");
    println!("Total endpoints:          {total_endpoints}");
    println!("With response schema:     {documented_with_schema}/{total_endpoints}");
    println!("With error responses:     {documented_with_error}/{total_endpoints}");
    println!("With correct auth:        {documented_with_auth}/{total_endpoints}");
    println!("Schema coverage:          {coverage_pct:.1}%");
    println!("===================================\n");

    assert!(
        coverage_pct >= 90.0,
        "Schema coverage {coverage_pct:.1}% is below 90% threshold"
    );
}
