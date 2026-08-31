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
//! Gateway configuration types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Address to bind the gateway listener
    pub bind_addr: String,
    pub routes: Vec<RouteConfig>,
    pub rate_limiting: RateLimitConfig,
    pub versioning: VersioningConfig,
    /// Enable request/response logging for analytics
    pub analytics_enabled: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".into(),
            routes: vec![],
            rate_limiting: RateLimitConfig::default(),
            versioning: VersioningConfig::default(),
            analytics_enabled: true,
        }
    }
}

/// A single route definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    /// Route ID used in analytics and logs
    pub id: String,
    /// Path prefix to match (e.g. "/api/v1/transactions")
    pub path_prefix: String,
    /// HTTP methods to match; empty = all methods
    pub methods: Vec<String>,
    /// Upstream URL to proxy to
    pub upstream: String,
    /// Protocol of the upstream service
    pub protocol: Protocol,
    /// API version this route belongs to
    pub version: String,
    /// Header-based routing predicates (all must match)
    pub header_predicates: HashMap<String, String>,
    /// Whether this route is deprecated
    pub deprecated: bool,
    /// Sunset date for deprecated routes (ISO-8601)
    pub sunset_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Rest,
    Grpc,
    GraphQL,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Default requests per second per API key
    pub default_rps: u32,
    /// Default burst size
    pub default_burst: u32,
    /// Per-key overrides: api_key_id → rps
    pub key_overrides: HashMap<String, u32>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            default_rps: 100,
            default_burst: 200,
            key_overrides: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersioningConfig {
    /// Current stable version (e.g. "v2")
    pub current_version: String,
    /// Versions that are deprecated but still served.
    /// Each entry carries an optional ISO-8601 sunset date used to populate
    /// the `Sunset` response header (RFC 8594).
    pub deprecated_versions: Vec<String>,
    /// Versions that are no longer served (return 410 Gone)
    pub sunset_versions: Vec<String>,
    /// Per-version sunset dates: version string → ISO-8601 date string.
    /// When present the date is included in `Sunset` and `Deprecation` headers.
    #[serde(default)]
    pub sunset_dates: HashMap<String, String>,
    /// Strategy used to determine the requested API version from inbound
    /// requests.  Defaults to `UrlPath`.
    #[serde(default)]
    pub strategy: VersionStrategy,
}

/// How the gateway extracts the requested API version from a request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VersionStrategy {
    /// Version is embedded in the URL path: `/api/v2/nodes`.
    /// This is the default and is the most cache-friendly option.
    #[default]
    UrlPath,
    /// Version is supplied via the `Accept` header using content negotiation:
    /// `Accept: application/vnd.stellar.v2+json`
    AcceptHeader,
    /// Version is supplied via a custom request header: `X-API-Version: v2`
    CustomHeader { header_name: String },
}

impl Default for VersioningConfig {
    fn default() -> Self {
        Self {
            current_version: "v1".into(),
            deprecated_versions: vec![],
            sunset_versions: vec![],
            sunset_dates: HashMap::new(),
            strategy: VersionStrategy::UrlPath,
        }
    }
}
