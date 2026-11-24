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
//! API versioning and deprecation management.
//!
//! # Issue #1419 — Comprehensive API Versioning Strategy
//!
//! ## Versioning Scheme
//!
//! The Stellar-K8s operator REST API uses **URL-path versioning** as the
//! primary strategy.  Version identifiers take the form `vN` (e.g. `v1`,
//! `v2`).  The base path for every versioned endpoint is:
//!
//! ```text
//! /api/{version}/{resource}
//! ```
//!
//! Header-based versioning (`Accept: application/vnd.stellar.vN+json` and
//! `X-API-Version: vN`) is supported as an opt-in alternative configured via
//! [`VersioningConfig::strategy`].
//!
//! ## Lifecycle
//!
//! ```text
//! Current  ──►  Deprecated  ──►  Sunset (410 Gone)
//! ```
//!
//! | State      | Served? | Extra response headers |
//! |------------|---------|------------------------|
//! | Current    | ✅       | —                      |
//! | Deprecated | ✅       | `Deprecation: true`, `Sunset: <date>` (RFC 8594), `Link: </api/v2>; rel="successor-version"` |
//! | Sunset     | ❌ 410   | `Sunset: <date>`       |
//!
//! ## Migration Timeline
//!
//! | Version | Status     | Sunset Date |
//! |---------|------------|-------------|
//! | v1      | Deprecated | 2026-12-31  |
//! | v2      | Current    | —           |
//!
//! Clients using `v1` will receive deprecation headers on every response and
//! should migrate to `v2` before the sunset date.  See the
//! [API Migration Guide](../../docs/api-versioning.md) for a field-level diff.

use crate::api_gateway::config::{VersionStrategy, VersioningConfig};

/// Outcome of a version check.
#[derive(Debug, PartialEq, Eq)]
pub enum VersionStatus {
    /// The requested version is the current stable version.
    Current,
    /// The requested version is still served but has a published end-of-life
    /// date.  Callers must migrate before `sunset_date`.
    Deprecated { sunset_date: Option<String> },
    /// The requested version is no longer served.  The gateway responds
    /// `410 Gone` for all requests to this version.
    Sunset { sunset_date: Option<String> },
    /// The version identifier could not be determined from the request using
    /// the configured [`VersionStrategy`].
    Unknown,
}

/// Check the lifecycle status of an API version string against configuration.
pub fn check_version(version: &str, cfg: &VersioningConfig) -> VersionStatus {
    if cfg.sunset_versions.iter().any(|v| v == version) {
        let sunset_date = cfg.sunset_dates.get(version).cloned();
        return VersionStatus::Sunset { sunset_date };
    }
    if cfg.deprecated_versions.iter().any(|v| v == version) {
        let sunset_date = cfg.sunset_dates.get(version).cloned();
        return VersionStatus::Deprecated { sunset_date };
    }
    if version == cfg.current_version {
        return VersionStatus::Current;
    }
    VersionStatus::Unknown
}

/// Extract the API version from a URL path.
///
/// Expects paths of the form `/api/v2/...` or `/v2/...`.
/// Returns `None` when no `vN` segment is found.
pub fn extract_version_from_path(path: &str) -> Option<String> {
    path.split('/')
        .find(|segment| {
            segment.starts_with('v')
                && segment.len() > 1
                && segment[1..].chars().all(|c| c.is_ascii_digit())
        })
        .map(|s| s.to_string())
}

/// Extract the API version from request headers according to the configured
/// [`VersionStrategy`].
///
/// Returns `None` when the strategy does not use headers, or when the required
/// header is absent / malformed.
pub fn extract_version_from_headers(
    strategy: &VersionStrategy,
    headers: &std::collections::HashMap<String, String>,
) -> Option<String> {
    match strategy {
        VersionStrategy::UrlPath => None,
        VersionStrategy::AcceptHeader => {
            // Accept: application/vnd.stellar.v2+json
            headers
                .get("accept")
                .or_else(|| headers.get("Accept"))
                .and_then(|v| {
                    v.split(',').find_map(|part| {
                        let part = part.trim();
                        let prefix = "application/vnd.stellar.";
                        if let Some(rest) = part.strip_prefix(prefix) {
                            // rest = "v2+json" or "v2"
                            let version = rest.split('+').next().unwrap_or(rest);
                            if version.starts_with('v') {
                                return Some(version.to_string());
                            }
                        }
                        None
                    })
                })
        }
        VersionStrategy::CustomHeader { header_name } => headers
            .get(header_name.as_str())
            .or_else(|| headers.get(&header_name.to_lowercase()))
            .cloned(),
    }
}

/// Build the HTTP response headers that must accompany a **deprecated** route.
///
/// Per [RFC 8594](https://datatracker.ietf.org/doc/html/rfc8594):
/// - `Deprecation: true`  — signals the deprecation to clients.
/// - `Sunset: <HTTP-date>` — the date on which the version will be removed.
/// - `Link: </api/vN>; rel="successor-version"` — points to the current API.
pub fn deprecation_headers(
    sunset_date: Option<&str>,
    successor_version: &str,
) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Deprecation".into(), "true".into()),
        (
            "Link".into(),
            format!(r#"</api/{successor_version}>; rel="successor-version""#),
        ),
    ];
    if let Some(date) = sunset_date {
        headers.push(("Sunset".into(), date.into()));
    }
    headers
}

/// Build the HTTP response headers for a **sunset** (410 Gone) route.
pub fn sunset_headers(sunset_date: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![];
    if let Some(date) = sunset_date {
        headers.push(("Sunset".into(), date.into()));
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_gateway::config::VersioningConfig;
    use std::collections::HashMap;

    fn make_cfg() -> VersioningConfig {
        let mut dates = HashMap::new();
        dates.insert("v1".to_string(), "2026-12-31".to_string());
        VersioningConfig {
            current_version: "v2".into(),
            deprecated_versions: vec!["v1".into()],
            sunset_versions: vec!["v0".into()],
            sunset_dates: dates,
            ..Default::default()
        }
    }

    #[test]
    fn detects_sunset() {
        let cfg = make_cfg();
        assert!(matches!(
            check_version("v0", &cfg),
            VersionStatus::Sunset { .. }
        ));
    }

    #[test]
    fn detects_deprecated_with_date() {
        let cfg = make_cfg();
        match check_version("v1", &cfg) {
            VersionStatus::Deprecated { sunset_date } => {
                assert_eq!(sunset_date, Some("2026-12-31".to_string()));
            }
            other => panic!("expected Deprecated, got {other:?}"),
        }
    }

    #[test]
    fn detects_current() {
        let cfg = make_cfg();
        assert_eq!(check_version("v2", &cfg), VersionStatus::Current);
    }

    #[test]
    fn detects_unknown() {
        let cfg = make_cfg();
        assert!(matches!(check_version("v99", &cfg), VersionStatus::Unknown));
    }

    #[test]
    fn extracts_version_from_path() {
        assert_eq!(
            extract_version_from_path("/api/v2/nodes"),
            Some("v2".to_string())
        );
        assert_eq!(
            extract_version_from_path("/v1/health"),
            Some("v1".to_string())
        );
        assert_eq!(extract_version_from_path("/health"), None);
    }

    #[test]
    fn extracts_version_from_accept_header() {
        let strategy = VersionStrategy::AcceptHeader;
        let mut headers = HashMap::new();
        headers.insert(
            "accept".to_string(),
            "application/vnd.stellar.v2+json".to_string(),
        );
        assert_eq!(
            extract_version_from_headers(&strategy, &headers),
            Some("v2".to_string())
        );
    }

    #[test]
    fn extracts_version_from_custom_header() {
        let strategy = VersionStrategy::CustomHeader {
            header_name: "X-API-Version".to_string(),
        };
        let mut headers = HashMap::new();
        headers.insert("X-API-Version".to_string(), "v2".to_string());
        assert_eq!(
            extract_version_from_headers(&strategy, &headers),
            Some("v2".to_string())
        );
    }

    #[test]
    fn deprecation_headers_include_link_and_sunset() {
        let hdrs = deprecation_headers(Some("2026-12-31"), "v2");
        let names: Vec<&str> = hdrs.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"Deprecation"));
        assert!(names.contains(&"Sunset"));
        assert!(names.contains(&"Link"));
    }
}
