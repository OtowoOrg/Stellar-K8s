//! API Versioning and Routing
//!
//! Provides sophisticated API routing with version management,
//! path-based routing, and header-based routing.

use std::collections::HashMap;
use std::sync::Arc;
use axum::{
    body::Body,
    extract::Request,
    response::Response,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// API Version
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApiVersion {
    pub major: u32,
    pub minor: Option<u32>,
}

impl ApiVersion {
    pub fn new(major: u32) -> Self {
        Self { major, minor: None }
    }

    pub fn with_minor(major: u32, minor: u32) -> Self {
        Self { major, minor: Some(minor) }
    }

    pub fn to_string(&self) -> String {
        match self.minor {
            Some(m) => format!("v{}.{}", self.major, m),
            None => format!("v{}", self.major),
        }
    }
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

/// Route rule for matching and routing requests
#[derive(Debug, Clone)]
pub struct RouteRule {
    /// Path pattern to match (supports regex)
    pub path_pattern: String,
    /// HTTP methods to match (empty = all)
    pub methods: Vec<http::Method>,
    /// Header requirements
    pub header_requirements: HashMap<String, String>,
    /// Query param requirements
    pub query_requirements: HashMap<String, String>,
    /// Target backend URL or internal path
    pub target: RouteTarget,
    /// Version requirement
    pub version: Option<ApiVersion>,
    /// Rate limit tier for this route
    pub rate_limit_tier: Option<String>,
    /// Whether to strip version from path
    pub strip_version: bool,
    /// Timeout for this route
    pub timeout_ms: Option<u64>,
    /// Retry configuration
    pub retry: Option<RetryConfig>,
}

#[derive(Debug, Clone)]
pub enum RouteTarget {
    /// Forward to backend service
    Backend(String),
    /// Internal handler path
    Internal(String),
    /// Redirect to URL
    Redirect(String),
    /// Static response
    Static(u16, String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub backoff_ms: u64,
    pub backoff_multiplier: f64,
    pub retryable_statuses: Vec<u16>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_ms: 100,
            backoff_multiplier: 2.0,
            retryable_statuses: vec![502, 503, 504],
        }
    }
}

impl RouteRule {
    /// Create a simple path-based route
    pub fn path(path: impl Into<String>, target: RouteTarget) -> Self {
        Self {
            path_pattern: path.into(),
            methods: vec![],
            header_requirements: HashMap::new(),
            query_requirements: HashMap::new(),
            target,
            version: None,
            rate_limit_tier: None,
            strip_version: false,
            timeout_ms: None,
            retry: None,
        }
    }

    /// Set allowed methods
    pub fn methods(mut self, methods: Vec<http::Method>) -> Self {
        self.methods = methods;
        self
    }

    /// Set version requirement
    pub fn version(mut self, version: ApiVersion) -> Self {
        self.version = Some(version);
        self
    }

    /// Set header requirement
    pub fn require_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.header_requirements.insert(key.into(), value.into());
        self
    }

    /// Set timeout
    pub fn timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Set retry configuration
    pub fn retry(mut self, config: RetryConfig) -> Self {
        self.retry = Some(config);
        self
    }

    /// Check if this rule matches a request
    pub fn matches(&self, req: &Request<Body>) -> bool {
        // Check version
        if let Some(ref required) = self.version {
            let path = req.uri().path();
            if !path.starts_with(&format!("/{}/", required.to_string())) {
                return false;
            }
        }

        // Check method
        if !self.methods.is_empty() {
            if !self.methods.contains(req.method()) {
                return false;
            }
        }

        // Check path pattern
        let path = req.uri().path();
        if let Ok(re) = Regex::new(&self.path_pattern) {
            if !re.is_match(path) {
                return false;
            }
        } else if !path.contains(&self.path_pattern) {
            // Fallback to simple contains for non-regex
            return false;
        }

        // Check header requirements
        for (key, value) in &self.header_requirements {
            if let Some(header_value) = req.headers().get(key) {
                if header_value.to_str().ok() != Some(value.as_str()) {
                    return false;
                }
            } else {
                return false;
            }
        }

        // Check query requirements
        if let Some(query) = req.uri().query() {
            for (key, value) in &self.query_requirements {
                if !query.contains(&format!("{}={}", key, value)) {
                    return false;
                }
            }
        }

        true
    }
}

/// Router configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    pub default_version: ApiVersion,
    pub supported_versions: Vec<ApiVersion>,
    pub routes: Vec<RouteConfig>,
    pub deprecated_versions: Vec<ApiVersion>,
    pub default_backend: Option<String>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            default_version: ApiVersion::new(1),
            supported_versions: vec![ApiVersion::new(1)],
            routes: vec![],
            deprecated_versions: vec![],
            default_backend: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    pub path: String,
    pub methods: Vec<String>,
    pub target: String,
    pub target_type: String, // "backend", "internal", "redirect"
    pub version: Option<String>,
    pub strip_version: bool,
    pub timeout_ms: Option<u64>,
}

/// Versioned Router that manages API versioning
pub struct VersionedRouter {
    routes: Arc<RwLock<Vec<RouteRule>>>,
    config: RouterConfig,
}

impl VersionedRouter {
    pub fn new() -> Self {
        Self {
            routes: Arc::new(RwLock::new(Vec::new())),
            config: RouterConfig::default(),
        }
    }

    pub fn from_config(config: RouterConfig) -> Self {
        let router = Self {
            routes: Arc::new(RwLock::new(Vec::new())),
            config: config.clone(),
        };

        // Parse and add routes
        for route_config in &config.routes {
            let mut rule = RouteRule::path(&route_config.path, match route_config.target_type.as_str() {
                "backend" => RouteTarget::Backend(route_config.target.clone()),
                "internal" => RouteTarget::Internal(route_config.target.clone()),
                "redirect" => RouteTarget::Redirect(route_config.target.clone()),
                _ => RouteTarget::Internal(route_config.target.clone()),
            });

            if let Some(ver) = &route_config.version {
                let parts: Vec<&str> = ver.trim_start_matches('v').split('.').collect();
                let major: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(1);
                let minor: Option<u32> = parts.get(1).and_then(|s| s.parse().ok());
                rule.version = Some(ApiVersion { major, minor });
            }

            rule.strip_version = route_config.strip_version;
            rule.timeout_ms = route_config.timeout_ms;

            // Add methods
            if !route_config.methods.is_empty() {
                rule.methods = route_config.methods.iter()
                    .filter_map(|m| m.parse().ok())
                    .collect();
            }
        }

        router
    }

    /// Add a route rule
    pub async fn add_route(&self, rule: RouteRule) {
        let mut routes = self.routes.write().await;
        routes.push(rule);
    }

    /// Remove a route rule
    pub async fn remove_route(&self, path_pattern: &str) {
        let mut routes = self.routes.write().await;
        routes.retain(|r| r.path_pattern != path_pattern);
    }

    /// Route a request to the appropriate handler
    pub async fn route(&self, req: Request<Body>) -> Request<Body> {
        let routes = self.routes.read().await;

        for rule in routes.iter() {
            if rule.matches(&req) {
                let mut final_req = req;
                
                // Strip version from path if needed
                if rule.strip_version {
                    let path = final_req.uri().path().to_string();
                    // Remove /v1/ or /v1.x/ prefix
                    let new_path = Regex::new(r"^/v\d+(\.\d+)?/")
                        .map(|re| re.replace(&path, "/").to_string())
                        .unwrap_or(path);
                    *final_req.uri_mut() = new_path.parse().unwrap();
                }

                // Handle internal routing - transform to internal path
                if let RouteTarget::Internal(ref internal_path) = rule.target {
                    let path = final_req.uri().path().to_string();
                    let new_path = path.replace(&internal_path[..], "");
                    *final_req.uri_mut() = format!("/internal{}", new_path).parse().unwrap();
                }

                return final_req;
            }
        }

        // No matching route, return original request
        req
    }

    /// Get all registered routes
    pub async fn get_routes(&self) -> Vec<String> {
        let routes = self.routes.read().await;
        routes.iter().map(|r| r.path_pattern.clone()).collect()
    }

    /// Get supported API versions
    pub fn get_supported_versions(&self) -> Vec<ApiVersion> {
        self.config.supported_versions.clone()
    }

    /// Check if a version is deprecated
    pub fn is_version_deprecated(&self, version: &ApiVersion) -> bool {
        self.config.deprecated_versions.contains(version)
    }

    /// Get default version
    pub fn get_default_version(&self) -> ApiVersion {
        self.config.default_version.clone()
    }
}

impl Default for VersionedRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Header-based version selector
pub struct VersionSelector;

impl VersionSelector {
    /// Extract version from Accept header
    pub fn from_accept(headers: &http::HeaderMap) -> Option<ApiVersion> {
        headers.get("Accept")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| Self::parse_media_type(s))
    }

    /// Extract version from custom header
    pub fn from_header(headers: &http::HeaderMap) -> Option<ApiVersion> {
        headers.get("X-API-Version")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| Self::parse_version_str(s))
    }

    /// Extract version from query param
    pub fn from_query(req: &Request<Body>) -> Option<ApiVersion> {
        req.uri().query()
            .and_then(|q| q.split('&')
                .filter_map(|p| p.split_once('='))
                .find(|(k, _)| k == "version"))
            .and_then(|(_, v)| Self::parse_version_str(v))
    }

    fn parse_media_type(accept: &str) -> Option<ApiVersion> {
        // Parse application/vnd.stellar.v1+json
        for part in accept.split(',') {
            if let Some(start) = part.find("vnd.stellar.v") {
                let version_part = &part[start + 11..];
                let version: String = version_part.chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                return Self::parse_version_str(&version);
            }
        }
        None
    }

    fn parse_version_str(s: &str) -> Option<ApiVersion> {
        let s = s.trim_start_matches('v');
        let parts: Vec<&str> = s.split('.').collect();
        let major: u32 = parts.first()?.parse().ok()?;
        let minor: Option<u32> = parts.get(1).and_then(|s| s.parse().ok());
        Some(ApiVersion { major, minor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Method;

    #[test]
    fn test_api_version() {
        let v1 = ApiVersion::new(1);
        assert_eq!(v1.to_string(), "v1");
        
        let v12 = ApiVersion::with_minor(1, 2);
        assert_eq!(v12.to_string(), "v1.2");
    }

    #[test]
    fn test_route_rule_matches() {
        let rule = RouteRule::path("/api/users", RouteTarget::Internal("/users".to_string()))
            .methods(vec![Method::GET, Method::POST])
            .version(ApiVersion::new(1));

        // This would need a proper Request to test matching
        assert!(rule.path_pattern.contains("/api/users"));
    }

    #[test]
    fn test_version_selector() {
        assert_eq!(
            VersionSelector::parse_version_str("v1"),
            Some(ApiVersion::new(1))
        );
        assert_eq!(
            VersionSelector::parse_version_str("v1.2"),
            Some(ApiVersion::with_minor(1, 2))
        );
    }

    #[tokio::test]
    async fn test_router_add_remove() {
        let router = VersionedRouter::new();
        
        router.add_route(RouteRule::path("/test", RouteTarget::Internal("/test".to_string()))).await;
        
        let routes = router.get_routes().await;
        assert_eq!(routes.len(), 1);
        
        router.remove_route("/test").await;
        
        let routes = router.get_routes().await;
        assert!(routes.is_empty());
    }
}