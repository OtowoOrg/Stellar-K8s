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
//! REST API URL-path versioning, deprecation, and sunset signaling.
//!
//! # Canonical scheme
//!
//! Public operator REST routes are versioned by **URL path**:
//!
//! ```text
//! /api/v1/...
//! /api/v2/...
//! ```
//!
//! Header-based negotiation (`Accept`, `X-API-Version`) is intentionally **not**
//! used on the production `rest_api` surface so clients have a single unambiguous
//! version key. The separate `api_gateway` module may continue to use its own
//! helpers for proxy routes; this module owns operator REST behavior.
//!
//! # Lifecycle
//!
//! ```text
//! Current ? Deprecated ? Sunset (retired from the catalog)
//! ```
//!
//! Deprecated versions remain served and receive `Deprecation` / optional `Sunset`
//! response headers (RFC 8594). Sunset entries are listed as retired in
//! `GET /api/versions`; this module does not force HTTP 410 on the operator API.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Extension, Request},
    http::{header, HeaderValue},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

/// Default current API version for the operator REST surface.
pub const DEFAULT_CURRENT_VERSION: &str = "v1";

/// Outcome of classifying a path version against policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionLifecycle {
    /// Supported stable version - no deprecation headers.
    Current,
    /// Still served; clients should migrate before sunset.
    Deprecated { sunset: Option<String> },
    /// Retired in the catalog (may still be configured for signaling).
    Sunset { sunset: Option<String> },
    /// Path is not under `/api/vN` (probes, metrics, dashboard, etc.).
    Unversioned,
}

/// Policy controlling which URL API versions are current, deprecated, or sunset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionPolicy {
    /// Canonical current version id (e.g. `v1`).
    pub current_version: String,
    /// Deprecated version id ? optional HTTP-date `Sunset` value.
    pub deprecated: HashMap<String, Option<String>>,
    /// Sunset (retired) version ids ? optional last-published sunset date.
    pub sunset: HashMap<String, Option<String>>,
}

impl Default for VersionPolicy {
    fn default() -> Self {
        Self {
            current_version: DEFAULT_CURRENT_VERSION.to_string(),
            deprecated: HashMap::new(),
            sunset: HashMap::new(),
        }
    }
}

impl VersionPolicy {
    /// Build policy from environment (documented in `docs/api/versioning.md`).
    ///
    /// * `REST_API_CURRENT_VERSION` - default `v1`
    /// * `REST_API_DEPRECATED_VERSIONS` - comma-separated ids (`v0,v1`)
    /// * `REST_API_SUNSET_VERSIONS` - comma-separated retired ids
    /// * `REST_API_SUNSET_DATES` - `v0=Wed, 01 Jul 2027 00:00:00 GMT;v1=...`
    pub fn from_env() -> Self {
        let current_version = std::env::var("REST_API_CURRENT_VERSION")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_CURRENT_VERSION.to_string());

        let dates = parse_sunset_dates(&std::env::var("REST_API_SUNSET_DATES").unwrap_or_default());

        let mut deprecated = HashMap::new();
        for id in
            parse_csv_versions(&std::env::var("REST_API_DEPRECATED_VERSIONS").unwrap_or_default())
        {
            let sunset = dates.get(&id).cloned();
            deprecated.insert(id, sunset);
        }

        let mut sunset = HashMap::new();
        for id in parse_csv_versions(&std::env::var("REST_API_SUNSET_VERSIONS").unwrap_or_default())
        {
            let date = dates.get(&id).cloned();
            sunset.insert(id, date);
        }

        Self {
            current_version,
            deprecated,
            sunset,
        }
    }

    /// Classify a version id such as `v1`.
    pub fn classify(&self, version: &str) -> VersionLifecycle {
        if let Some(sunset) = self.sunset.get(version) {
            return VersionLifecycle::Sunset {
                sunset: sunset.clone(),
            };
        }
        if let Some(sunset) = self.deprecated.get(version) {
            return VersionLifecycle::Deprecated {
                sunset: sunset.clone(),
            };
        }
        if version == self.current_version {
            return VersionLifecycle::Current;
        }
        // Unknown but well-formed version ids are treated as current-compatible
        // mounts (future `/api/v2` routes) without deprecation headers.
        VersionLifecycle::Current
    }

    /// Catalog for `GET /api/versions`.
    pub fn catalog(&self) -> VersionCatalog {
        let mut versions = Vec::new();

        versions.push(VersionInfo {
            id: self.current_version.clone(),
            status: "current".into(),
            base_path: format!("/api/{}", self.current_version),
            sunset: None,
        });

        for (id, sunset) in &self.deprecated {
            if id == &self.current_version {
                continue;
            }
            versions.push(VersionInfo {
                id: id.clone(),
                status: "deprecated".into(),
                base_path: format!("/api/{id}"),
                sunset: sunset.clone(),
            });
        }

        for (id, sunset) in &self.sunset {
            versions.push(VersionInfo {
                id: id.clone(),
                status: "sunset".into(),
                base_path: format!("/api/{id}"),
                sunset: sunset.clone(),
            });
        }

        versions.sort_by(|a, b| a.id.cmp(&b.id));

        VersionCatalog {
            canonical_scheme: "url_path".into(),
            current: self.current_version.clone(),
            versions,
        }
    }
}

/// Public catalog returned by `GET /api/versions`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionCatalog {
    pub canonical_scheme: String,
    pub current: String,
    pub versions: Vec<VersionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionInfo {
    pub id: String,
    pub status: String,
    pub base_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sunset: Option<String>,
}

/// Extract `vN` from paths like `/api/v1/nodes` or `/api/v2`.
pub fn extract_path_version(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/api/")?;
    let version = rest.split('/').next()?;
    if is_version_id(version) {
        Some(version)
    } else {
        None
    }
}

fn is_version_id(s: &str) -> bool {
    let mut chars = s.chars();
    if chars.next() != Some('v') {
        return false;
    }
    let mut saw_digit = false;
    for c in chars {
        if !c.is_ascii_digit() {
            return false;
        }
        saw_digit = true;
    }
    saw_digit
}

fn parse_csv_versions(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && is_version_id(s))
        .map(str::to_string)
        .collect()
}

fn parse_sunset_dates(raw: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in raw.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((id, date)) = part.split_once('=') else {
            continue;
        };
        let id = id.trim();
        let date = date.trim();
        if is_version_id(id) && !date.is_empty() {
            out.insert(id.to_string(), date.to_string());
        }
    }
    out
}

/// Attach `Deprecation` / `Sunset` when the request path is a deprecated version.
pub async fn inject_api_version_headers(
    Extension(policy): Extension<Arc<VersionPolicy>>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let version = extract_path_version(&path).map(str::to_string);
    let mut response = next.run(req).await;

    let Some(version) = version else {
        return response;
    };

    match policy.classify(&version) {
        VersionLifecycle::Deprecated { sunset } | VersionLifecycle::Sunset { sunset } => {
            apply_deprecation_headers(response.headers_mut(), sunset.as_deref());
        }
        VersionLifecycle::Current | VersionLifecycle::Unversioned => {}
    }

    response
}

/// Apply RFC 8594-style deprecation headers (aligned with gateway semantics).
pub fn apply_deprecation_headers(
    headers: &mut axum::http::HeaderMap,
    sunset_http_date: Option<&str>,
) {
    if let Ok(value) = HeaderValue::from_str("true") {
        headers.insert(header::HeaderName::from_static("deprecation"), value);
    }
    if let Some(date) = sunset_http_date {
        if let Ok(value) = HeaderValue::from_str(date) {
            headers.insert(header::HeaderName::from_static("sunset"), value);
        }
    }
}

/// `GET /api/versions` - unversioned discovery document for coexistence.
pub async fn list_versions(Extension(policy): Extension<Arc<VersionPolicy>>) -> impl IntoResponse {
    Json(policy.catalog())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    fn policy_v1_current() -> Arc<VersionPolicy> {
        Arc::new(VersionPolicy::default())
    }

    fn policy_v1_deprecated() -> Arc<VersionPolicy> {
        let mut deprecated = HashMap::new();
        deprecated.insert("v1".into(), Some("Wed, 01 Sep 2027 00:00:00 GMT".into()));
        Arc::new(VersionPolicy {
            current_version: "v2".into(),
            deprecated,
            sunset: HashMap::new(),
        })
    }

    async fn ok_handler() -> &'static str {
        "ok"
    }

    fn test_app(policy: Arc<VersionPolicy>) -> Router {
        Router::new()
            .route("/api/v1/nodes", get(ok_handler))
            .route("/api/v2/nodes", get(ok_handler))
            .route("/api/versions", get(list_versions))
            .route("/healthz", get(ok_handler))
            .layer(axum::middleware::from_fn(inject_api_version_headers))
            .layer(Extension(policy))
    }

    #[test]
    fn extracts_path_versions() {
        assert_eq!(extract_path_version("/api/v1/nodes"), Some("v1"));
        assert_eq!(extract_path_version("/api/v2"), Some("v2"));
        assert_eq!(extract_path_version("/api/versions"), None);
        assert_eq!(extract_path_version("/v1/health/summary"), None);
        assert_eq!(extract_path_version("/healthz"), None);
    }

    #[test]
    fn classifies_lifecycle() {
        let policy = policy_v1_deprecated();
        assert_eq!(
            policy.classify("v1"),
            VersionLifecycle::Deprecated {
                sunset: Some("Wed, 01 Sep 2027 00:00:00 GMT".into())
            }
        );
        assert_eq!(policy.classify("v2"), VersionLifecycle::Current);
    }

    #[tokio::test]
    async fn current_version_has_no_deprecation_headers() {
        let app = test_app(policy_v1_current());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/nodes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("deprecation").is_none());
        assert!(response.headers().get("sunset").is_none());
    }

    #[tokio::test]
    async fn deprecated_version_includes_deprecation_and_sunset() {
        let app = test_app(policy_v1_deprecated());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/nodes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("deprecation").unwrap(), "true");
        assert_eq!(
            response.headers().get("sunset").unwrap(),
            "Wed, 01 Sep 2027 00:00:00 GMT"
        );
    }

    #[tokio::test]
    async fn coexistence_current_v2_without_deprecation_headers() {
        let app = test_app(policy_v1_deprecated());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v2/nodes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("deprecation").is_none());
        assert!(response.headers().get("sunset").is_none());
    }

    #[tokio::test]
    async fn unversioned_routes_skip_deprecation_headers() {
        let app = test_app(policy_v1_deprecated());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("deprecation").is_none());
    }

    #[tokio::test]
    async fn versions_catalog_lists_coexisting_versions() {
        let app = test_app(policy_v1_deprecated());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/versions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let catalog: VersionCatalog = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(catalog.canonical_scheme, "url_path");
        assert_eq!(catalog.current, "v2");
        assert!(catalog
            .versions
            .iter()
            .any(|v| v.id == "v1" && v.status == "deprecated"));
        assert!(catalog
            .versions
            .iter()
            .any(|v| v.id == "v2" && v.status == "current"));
    }

    #[test]
    fn parse_env_style_sunset_dates() {
        let dates =
            parse_sunset_dates("v1=Wed, 01 Sep 2027 00:00:00 GMT;v0=Thu, 01 Jan 2026 00:00:00 GMT");
        assert_eq!(
            dates.get("v1").map(String::as_str),
            Some("Wed, 01 Sep 2027 00:00:00 GMT")
        );
    }
}
