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
//! Request/response schema validation middleware for the REST API (#1396)
//!
//! This middleware validates HTTP request and response bodies against the
//! OpenAPI 3.0 specification. It is designed as an optional Tower layer
//! that can be enabled for development/testing environments.
//!
//! # Usage
//!
//! ```rust,no_run
//! use stellar_k8s::rest_api::schema_validation::SchemaValidationLayer;
//!
//! // Add to your Axum router:
//! // let app = Router::new()
//! //     .route("/api/v1/nodes", get(list_nodes))
//! //     .layer(SchemaValidationLayer::new());
//! ```

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, warn};

/// Shared OpenAPI specification loaded at startup.
#[derive(Clone)]
pub struct OpenApiSpec {
    spec: Arc<Value>,
}

impl OpenApiSpec {
    /// Load the OpenAPI spec from the embedded YAML file.
    pub fn load() -> Result<Self, String> {
        let spec_bytes = include_bytes!("../../docs/api/openapi.yaml");
        let spec: Value = serde_yaml::from_slice(spec_bytes)
            .map_err(|e| format!("Failed to parse OpenAPI spec: {e}"))?;
        Ok(Self {
            spec: Arc::new(spec),
        })
    }

    /// Get the response schema for a given method, path, and status code.
    pub fn response_schema(&self, method: &str, path: &str, status: &str) -> Option<Value> {
        let path_item = self.spec["paths"].get(path)?;
        let operation = path_item.get(method.to_lowercase())?;
        let response = operation["responses"].get(status)?;
        let content = response.get("content")?;
        let json_content = content.get("application/json")?;
        let schema = json_content.get("schema")?;
        self.resolve_schema(schema)
    }

    /// Get the request body schema for a given method and path.
    pub fn request_schema(&self, method: &str, path: &str) -> Option<Value> {
        let path_item = self.spec["paths"].get(path)?;
        let operation = path_item.get(method.to_lowercase())?;
        let request_body = operation.get("requestBody")?;
        let content = request_body.get("content")?;
        let json_content = content.get("application/json")?;
        let schema = json_content.get("schema")?;
        self.resolve_schema(schema)
    }

    /// Resolve a $ref pointer to the actual schema.
    fn resolve_schema(&self, schema: &Value) -> Option<Value> {
        if let Some(ref_str) = schema.get("$ref").and_then(|v| v.as_str()) {
            let ref_path = ref_str.trim_start_matches("#/");
            self.spec.pointer(ref_path).cloned()
        } else {
            Some(schema.clone())
        }
    }

    /// Check if a path is documented in the spec.
    pub fn has_path(&self, method: &str, path: &str) -> bool {
        self.spec["paths"]
            .get(path)
            .and_then(|p| p.get(method.to_lowercase()))
            .is_some()
    }
}

/// Middleware layer that validates request/response bodies against the OpenAPI spec.
///
/// This layer is intended for development/testing use. In production, the
/// overhead of JSON schema validation on every request may not be acceptable.
#[derive(Clone)]
pub struct SchemaValidationLayer {
    spec: OpenApiSpec,
}

impl SchemaValidationLayer {
    /// Create a new validation layer using the embedded OpenAPI spec.
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            spec: OpenApiSpec::load()?,
        })
    }
}

/// Validate a JSON body against a JSON Schema.
///
/// Returns `Ok(())` if valid, or `Err(message)` with details.
pub fn validate_json_schema(json: &Value, schema: &Value) -> Result<(), String> {
    validate_value(json, schema)
}

/// Core validation function that recursively validates JSON values against schemas.
fn validate_value(json: &Value, schema: &Value) -> Result<(), String> {
    // Resolve $ref if present
    let schema = if let Some(ref_str) = schema.get("$ref").and_then(|v| v.as_str()) {
        let spec = OpenApiSpec::load().map_err(|e| format!("Failed to load spec: {e}"))?;
        let ref_path = ref_str.trim_start_matches("#/");
        spec.spec
            .pointer(ref_path)
            .cloned()
            .unwrap_or_else(|| schema.clone())
    } else {
        schema.clone()
    };

    match schema.get("type").and_then(|t| t.as_str()) {
        Some("object") => validate_object(json, &schema),
        Some("array") => validate_array(json, &schema),
        Some("string") => validate_string(json, &schema),
        Some("integer") => validate_integer(json, &schema),
        Some("number") => validate_number(json, &schema),
        Some("boolean") => {
            if !json.is_boolean() {
                return Err(format!("Expected boolean, got {}", json_type_name(json)));
            }
            Ok(())
        }
        None => {
            if let Some(enum_values) = schema.get("enum") {
                validate_enum(json, enum_values)
            } else {
                Ok(())
            }
        }
        other => Err(format!("Unsupported schema type: {other:?}")),
    }
}

fn validate_object(json: &Value, schema: &Value) -> Result<(), String> {
    if !json.is_object() {
        return Err(format!("Expected object, got {}", json_type_name(json)));
    }

    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        for field in required {
            let field_name = field.as_str().unwrap_or("");
            if json.get(field_name).is_none() {
                return Err(format!("Missing required field: {field_name}"));
            }
        }
    }

    if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
        if let Some(obj) = json.as_object() {
            for (key, value) in obj {
                if let Some(prop_schema) = properties.get(key) {
                    validate_value(value, prop_schema)
                        .map_err(|e| format!("Property '{key}': {e}"))?;
                }
            }
        }
    }

    Ok(())
}

fn validate_array(json: &Value, schema: &Value) -> Result<(), String> {
    if !json.is_array() {
        return Err(format!("Expected array, got {}", json_type_name(json)));
    }

    if let Some(items_schema) = schema.get("items") {
        if let Some(arr) = json.as_array() {
            for (i, item) in arr.iter().enumerate() {
                validate_value(item, items_schema)
                    .map_err(|e| format!("Array item {i}: {e}"))?;
            }
        }
    }

    Ok(())
}

fn validate_string(json: &Value, schema: &Value) -> Result<(), String> {
    if !json.is_string() {
        return Err(format!("Expected string, got {}", json_type_name(json)));
    }

    if let Some(enum_values) = schema.get("enum") {
        validate_enum(json, enum_values)?;
    }

    Ok(())
}

fn validate_integer(json: &Value, schema: &Value) -> Result<(), String> {
    if !json.is_number() {
        return Err(format!("Expected integer, got {}", json_type_name(json)));
    }

    if let Some(num) = json.as_f64() {
        if num.fract() != 0.0 {
            return Err(format!("Expected integer, got float: {num}"));
        }

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

fn validate_number(json: &Value, schema: &Value) -> Result<(), String> {
    if !json.is_number() {
        return Err(format!("Expected number, got {}", json_type_name(json)));
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

fn json_type_name(json: &Value) -> &'static str {
    match json {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Tower middleware function for request validation.
pub async fn validate_request_middleware(
    State(spec): State<OpenApiSpec>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path();

    if !spec.has_path(method.as_str(), path) {
        debug!(path = %path, "Skipping validation for undocumented path");
        return Ok(next.run(request).await);
    }

    if let Some(schema) = spec.request_schema(method.as_str(), path) {
        let is_json = request
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("application/json"))
            .unwrap_or(false);

        if is_json {
            let (parts, body) = request.into_parts();
            match axum::body::to_bytes(body, 1024 * 1024).await {
                Ok(bytes) => {
                    if !bytes.is_empty() {
                        match serde_json::from_slice::<Value>(&bytes) {
                            Ok(json) => {
                                if let Err(e) = validate_json_schema(&json, &schema) {
                                    warn!(
                                        path = %path,
                                        method = %method,
                                        error = %e,
                                        "Request body schema validation failed"
                                    );
                                    return Err(StatusCode::BAD_REQUEST);
                                }
                            }
                            Err(e) => {
                                warn!(
                                    path = %path,
                                    method = %method,
                                    error = %e,
                                    "Failed to parse request body as JSON"
                                );
                                return Err(StatusCode::BAD_REQUEST);
                            }
                        }
                    }
                    let request = Request::from_parts(parts, Body::from(bytes));
                    Ok(next.run(request).await)
                }
                Err(_) => Ok(next.run(Request::from_parts(parts, Body::empty())).await),
            }
        } else {
            Ok(next.run(request).await)
        }
    } else {
        Ok(next.run(request).await)
    }
}

/// Tower middleware function for response validation.
pub async fn validate_response_middleware(
    State(spec): State<OpenApiSpec>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path();

    let response = next.run(request).await;

    if !spec.has_path(method.as_str(), path) {
        return Ok(response);
    }

    let status = response.status().as_u16().to_string();

    let is_json = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("application/json"))
        .unwrap_or(false);

    if !is_json {
        return Ok(response);
    }

    if let Some(schema) = spec.response_schema(method.as_str(), path, &status) {
        let (parts, body) = response.into_parts();
        match axum::body::to_bytes(body, 1024 * 1024).await {
            Ok(bytes) => {
                if !bytes.is_empty() {
                    match serde_json::from_slice::<Value>(&bytes) {
                        Ok(json) => {
                            if let Err(e) = validate_json_schema(&json, &schema) {
                                warn!(
                                    path = %path,
                                    method = %method,
                                    status = %status,
                                    error = %e,
                                    "Response body schema validation failed"
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                path = %path,
                                method = %method,
                                status = %status,
                                error = %e,
                                "Failed to parse response body as JSON"
                            );
                        }
                    }
                }
                Ok(Response::from_parts(parts, Body::from(bytes)))
            }
            Err(_) => Ok(Response::new(Body::empty())),
        }
    } else {
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_openapi_spec_loads() {
        let spec = OpenApiSpec::load();
        assert!(
            spec.is_ok(),
            "Failed to load OpenAPI spec: {:?}",
            spec.err()
        );
    }

    #[test]
    fn test_response_schema_found() {
        let spec = OpenApiSpec::load().unwrap();
        let schema = spec.response_schema("get", "/health", "200");
        assert!(
            schema.is_some(),
            "Response schema not found for GET /health 200"
        );
    }

    #[test]
    fn test_request_schema_found() {
        let spec = OpenApiSpec::load().unwrap();
        let schema = spec.request_schema("post", "/config/log-level");
        assert!(
            schema.is_some(),
            "Request schema not found for POST /config/log-level"
        );
    }

    #[test]
    fn test_has_path() {
        let spec = OpenApiSpec::load().unwrap();
        assert!(spec.has_path("get", "/health"));
        assert!(spec.has_path("get", "/api/v1/nodes"));
        assert!(!spec.has_path("get", "/nonexistent"));
    }

    #[test]
    fn test_validate_valid_health_response() {
        let spec = OpenApiSpec::load().unwrap();
        let schema = spec.response_schema("get", "/health", "200").unwrap();
        let valid = json!({"status": "healthy", "version": "0.1.0"});
        assert!(validate_json_schema(&valid, &schema).is_ok());
    }

    #[test]
    fn test_validate_invalid_health_response() {
        let spec = OpenApiSpec::load().unwrap();
        let schema = spec.response_schema("get", "/health", "200").unwrap();
        let invalid = json!({"status": 123}); // status should be string
        assert!(validate_json_schema(&invalid, &schema).is_err());
    }

    #[test]
    fn test_validate_valid_log_level_request() {
        let spec = OpenApiSpec::load().unwrap();
        let schema = spec.request_schema("post", "/config/log-level").unwrap();
        let valid = json!({"level": "debug"});
        assert!(validate_json_schema(&valid, &schema).is_ok());
    }

    #[test]
    fn test_validate_invalid_log_level_request() {
        let spec = OpenApiSpec::load().unwrap();
        let schema = spec.request_schema("post", "/config/log-level").unwrap();
        let invalid = json!({}); // missing required 'level' field
        assert!(validate_json_schema(&invalid, &schema).is_err());
    }
}
