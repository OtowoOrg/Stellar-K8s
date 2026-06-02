//! Request/Response Transformation Pipeline
//!
//! Provides flexible transformation of HTTP requests and responses
//! including header manipulation, body mapping, and protocol translation.

use std::collections::HashMap;
use std::sync::Arc;
use axum::{
    body::Body,
    extract::Request,
    response::Response,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Transformation rule types
#[derive(Debug, Clone)]
pub enum TransformRule {
    /// Add headers to request/response
    AddHeaders(HashMap<String, String>),
    /// Remove headers from request/response
    RemoveHeaders(Vec<String>),
    /// Rename headers
    RenameHeaders(HashMap<String, String>),
    /// Map request path to different backend path
    MapPath { from: String, to: String },
    /// Add query parameters
    AddQueryParams(HashMap<String, String>),
    /// Remove query parameters
    RemoveQueryParams(Vec<String>),
    /// Transform body using JSONPath or custom logic
    TransformBody(BodyTransform),
    /// Add authentication headers
    AddAuth(String),
    /// Set response status override
    StatusOverride(u16),
}

/// Body transformation types
#[derive(Debug, Clone)]
pub enum BodyTransform {
    /// Wrap response in envelope
    Wrap { field: String },
    /// Extract field from JSON
    Extract(String),
    /// Flatten nested JSON
    Flatten,
    /// Convert to different format (placeholder)
    Convert(String),
    /// Remove fields from JSON
    RemoveFields(Vec<String>),
    /// Add static fields
    AddFields(HashMap<String, serde_json::Value>),
}

impl TransformRule {
    /// Apply transformation to request
    pub async fn apply_request(&self, mut req: Request<Body>) -> Request<Body> {
        match self {
            TransformRule::AddHeaders(headers) => {
                for (key, value) in headers {
                    req.headers_mut().insert(
                        key.as_str().parse().unwrap(),
                        value.parse().unwrap(),
                    );
                }
            }
            TransformRule::RemoveHeaders(names) => {
                for name in names {
                    req.headers_mut().remove(name.as_str());
                }
            }
            TransformRule::RenameHeaders(map) => {
                let mut new_headers = req.headers().clone();
                for (from, to) in map {
                    if let Some(value) = new_headers.remove(from.as_str()) {
                        new_headers.insert(to.as_str().parse().unwrap(), value);
                    }
                }
                *req.headers_mut() = new_headers;
            }
            TransformRule::MapPath { from, to } => {
                let uri = req.uri().to_string();
                if uri.starts_with(from) {
                    let new_uri = uri.replacen(from, to, 1);
                    *req.uri_mut() = new_uri.parse().unwrap();
                }
            }
            TransformRule::AddQueryParams(params) => {
                let uri = req.uri();
                let mut query = uri.query().map(|s| s.to_string()).unwrap_or_default();
                for (key, value) in params {
                    if !query.is_empty() {
                        query.push('&');
                    }
                    query.push_str(&format!("{}={}", key, value));
                }
                let new_uri = format!("{}?{}", uri.path(), query);
                *req.uri_mut() = new_uri.parse().unwrap();
            }
            TransformRule::RemoveQueryParams(names) => {
                let uri = req.uri();
                let query = uri.query().unwrap_or("");
                let mut new_params: Vec<String> = vec![];
                for param in query.split('&') {
                    if let Some((key, _)) = param.split_once('=') {
                        if !names.contains(&key.to_string()) {
                            new_params.push(param.to_string());
                        }
                    }
                }
                let new_query = new_params.join("&");
                let new_uri = if new_query.is_empty() {
                    uri.path().to_string()
                } else {
                    format!("{}?{}", uri.path(), new_query)
                };
                *req.uri_mut() = new_uri.parse().unwrap();
            }
            TransformRule::AddAuth(token) => {
                req.headers_mut().insert(
                    "authorization",
                    format!("Bearer {}", token).parse().unwrap(),
                );
            }
            _ => {}
        }
        req
    }

    /// Apply transformation to response
    pub async fn apply_response(&self, mut res: Response<Body>) -> Response<Body> {
        match self {
            TransformRule::AddHeaders(headers) => {
                for (key, value) in headers {
                    res.headers_mut().insert(
                        key.as_str().parse().unwrap(),
                        value.parse().unwrap(),
                    );
                }
            }
            TransformRule::RemoveHeaders(names) => {
                for name in names {
                    res.headers_mut().remove(name.as_str());
                }
            }
            TransformRule::RenameHeaders(map) => {
                let mut new_headers = res.headers().clone();
                for (from, to) in map {
                    if let Some(value) = new_headers.remove(from.as_str()) {
                        new_headers.insert(to.as_str().parse().unwrap(), value);
                    }
                }
                *res.headers_mut() = new_headers;
            }
            TransformRule::StatusOverride(code) => {
                *res.status_mut() = StatusCode::from_u16(*code).unwrap_or(StatusCode::OK);
            }
            TransformRule::TransformBody(transform) => {
                let body = hyper::body::to_bytes(res.body_mut()).await.unwrap_or_default();
                if let Ok(new_body) = transform_json(&body, transform) {
                    *res.body_mut() = Body::from(new_body);
                }
            }
            _ => {}
        }
        res
    }
}

/// Transform JSON body based on transform type
fn transform_json(body: &[u8], transform: &BodyTransform) -> Result<Bytes, String> {
    let json: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| format!("Failed to parse JSON: {}", e))?;

    let result = match transform {
        BodyTransform::Wrap { field } => {
            serde_json::json!({ field: json })
        }
        BodyTransform::Extract(path) => {
            // Simple JSONPath-like extraction (supports dot notation)
            extract_json_path(&json, path)
        }
        BodyTransform::Flatten => {
            flatten_json(json)
        }
        BodyTransform::Convert(_) => {
            // Format conversion placeholder
            json
        }
        BodyTransform::RemoveFields(fields) => {
            remove_json_fields(&json, fields)
        }
        BodyTransform::AddFields(add_fields) => {
            let mut result = json.clone();
            for (key, value) in add_fields {
                result[key] = value.clone();
            }
            result
        }
    };

    serde_json::to_vec(&result).map(Bytes::from).map_err(|e| e.to_string())
}

/// Extract value from JSON using dot notation path
fn extract_json_path(json: &serde_json::Value, path: &str) -> serde_json::Value {
    let mut current = json.clone();
    for key in path.split('.') {
        if let serde_json::Value::Object(map) = current {
            current = map.get(key).cloned().unwrap_or(serde_json::Value::Null);
        } else {
            return serde_json::Value::Null;
        }
    }
    current
}

/// Flatten nested JSON structure
fn flatten_json(json: serde_json::Value) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    flatten_json_recursive(json, String::new(), &mut result);
    serde_json::Value::Object(result)
}

fn flatten_json_recursive(
    value: serde_json::Value,
    prefix: String,
    result: &mut serde_json::Map<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let new_prefix = if prefix.is_empty() {
                    key
                } else {
                    format!("{}.{}", prefix, key)
                };
                flatten_json_recursive(val, new_prefix, result);
            }
        }
        _ => {
            result.insert(prefix, value);
        }
    }
}

/// Remove specified fields from JSON
fn remove_json_fields(json: &serde_json::Value, fields: &[String]) -> serde_json::Value {
    if let serde_json::Value::Object(map) = json {
        let mut result = map.clone();
        for field in fields {
            result.remove(field);
        }
        serde_json::Value::Object(result)
    } else {
        json.clone()
    }
}

/// Transformation pipeline that chains multiple rules
#[derive(Default)]
pub struct TransformPipeline {
    request_rules: Vec<TransformRule>,
    response_rules: Vec<TransformRule>,
}

impl TransformPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a request transformation rule
    pub fn add_request_rule(&mut self, rule: TransformRule) {
        self.request_rules.push(rule);
    }

    /// Add a response transformation rule
    pub fn add_response_rule(&mut self, rule: TransformRule) {
        self.response_rules.push(rule);
    }

    /// Add multiple request rules
    pub fn add_request_rules(&mut self, rules: Vec<TransformRule>) {
        self.request_rules.extend(rules);
    }

    /// Add multiple response rules
    pub fn add_response_rules(&mut self, rules: Vec<TransformRule>) {
        self.response_rules.extend(rules);
    }

    /// Transform a request through all rules
    pub async fn transform_request(&self, mut req: Request<Body>) -> Request<Body> {
        for rule in &self.request_rules {
            req = rule.apply_request(req).await;
        }
        req
    }

    /// Transform a response through all rules
    pub async fn transform_response(&self, res: Response<Body>) -> Response<Body> {
        let mut res = res;
        for rule in &self.response_rules {
            res = rule.apply_response(res).await;
        }
        res
    }

    /// Clear all rules
    pub fn clear(&mut self) {
        self.request_rules.clear();
        self.response_rules.clear();
    }

    /// Get request rule count
    pub fn request_rule_count(&self) -> usize {
        self.request_rules.len()
    }

    /// Get response rule count
    pub fn response_rule_count(&self) -> usize {
        self.response_rules.len()
    }
}

/// Transformation pipeline configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformConfig {
    pub request_transforms: Vec<TransformRuleConfig>,
    pub response_transforms: Vec<TransformRuleConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformRuleConfig {
    pub rule_type: String,
    pub config: serde_json::Value,
}

impl TransformPipeline {
    /// Create pipeline from configuration
    pub fn from_config(config: &TransformConfig) -> Self {
        let mut pipeline = Self::new();
        
        for rule_config in &config.request_transforms {
            if let Some(rule) = parse_rule_config(&rule_config.rule_type, &rule_config.config) {
                pipeline.add_request_rule(rule);
            }
        }
        
        for rule_config in &config.response_transforms {
            if let Some(rule) = parse_rule_config(&rule_config.rule_type, &rule_config.config) {
                pipeline.add_response_rule(rule);
            }
        }
        
        pipeline
    }
}

/// Parse rule configuration into TransformRule
fn parse_rule_config(rule_type: &str, config: &serde_json::Value) -> Option<TransformRule> {
    match rule_type {
        "add_headers" => {
            let headers: HashMap<String, String> = 
                serde_json::from_value(config.clone()).ok()?;
            Some(TransformRule::AddHeaders(headers))
        }
        "remove_headers" => {
            let headers: Vec<String> = serde_json::from_value(config.clone()).ok()?;
            Some(TransformRule::RemoveHeaders(headers))
        }
        "map_path" => {
            let from = config.get("from")?.as_str()?.to_string();
            let to = config.get("to")?.as_str()?.to_string();
            Some(TransformRule::MapPath { from, to })
        }
        "add_query_params" => {
            let params: HashMap<String, String> = 
                serde_json::from_value(config.clone()).ok()?;
            Some(TransformRule::AddQueryParams(params))
        }
        "wrap" => {
            let field = config.get("field")?.as_str()?.to_string();
            Some(TransformRule::TransformBody(BodyTransform::Wrap { field }))
        }
        "extract" => {
            let path = config.get("path")?.as_str()?.to_string();
            Some(TransformRule::TransformBody(BodyTransform::Extract(path)))
        }
        "remove_fields" => {
            let fields: Vec<String> = serde_json::from_value(config.clone()).ok()?;
            Some(TransformRule::TransformBody(BodyTransform::RemoveFields(fields)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_pipeline() {
        let mut pipeline = TransformPipeline::new();
        
        pipeline.add_request_rule(TransformRule::AddHeaders({
            let mut h = HashMap::new();
            h.insert("X-Custom-Header".to_string(), "value".to_string());
            h
        }));
        
        pipeline.add_response_rule(TransformRule::RemoveHeaders(vec!["x-internal".to_string()]));
        
        assert_eq!(pipeline.request_rule_count(), 1);
        assert_eq!(pipeline.response_rule_count(), 1);
    }

    #[test]
    fn test_flatten_json() {
        let json = serde_json::json!({
            "user": {
                "name": "John",
                "email": "john@example.com"
            },
            "active": true
        });
        
        let flattened = flatten_json(json);
        assert_eq!(flattened.get("user.name").unwrap(), "John");
        assert_eq!(flattened.get("user.email").unwrap(), "john@example.com");
        assert_eq!(flattened.get("active").unwrap(), true);
    }

    #[test]
    fn test_extract_json_path() {
        let json = serde_json::json!({
            "user": {
                "profile": {
                    "name": "John"
                }
            }
        });
        
        let result = extract_json_path(&json, "user.profile.name");
        assert_eq!(result, "John");
    }
}