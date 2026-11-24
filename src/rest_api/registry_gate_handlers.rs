// Copyright 2026 Stellar-K8s Contributors
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
//! REST API for the registry pull-path vulnerability gate (#2000).
//!
//! # Endpoints
//!
//! | Method | Path                                                         | Description |
//! |--------|--------------------------------------------------------------|-------------|
//! | GET    | `/api/v1/registry/:namespace/:name/pull-gate/:digest`        | Authorize a manifest pull. `200` with the decision, or `403` with an OCI distribution error that names the reason and links the scan report. |
//! | GET    | `/api/v1/registry/scan-reports/:digest`                      | Scan report for a digest. This is where a denial's `reportUrl` points. |
//! | POST   | `/api/v1/registry/:namespace/:name/push`                     | Admin only. Scan a pushed digest before the push is acknowledged, and store its report. Body: `{"repository": "app", "digest": "sha256:…"}`. |
//!
//! The gate is built from the `StellarRegistry` CR by the registry reconciler
//! (`spec.admission.pullGate`, `spec.scanning.maxCriticalCves`), so a policy
//! change takes effect on the next reconcile without losing scan reports.
//!
//! ```bash
//! curl -i https://operator:9090/api/v1/registry/stellar/main/pull-gate/sha256:<digest>
//! curl    https://operator:9090/api/v1/registry/scan-reports/sha256:<digest>
//! curl -X POST https://operator:9090/api/v1/registry/stellar/main/push \
//!      -H 'content-type: application/json' \
//!      -d '{"repository":"app","digest":"sha256:<digest>"}'
//! ```
//!
//! A digest only has a report once it has been scanned, and `enforce` mode
//! denies digests without one. Point the registry's push notification at the
//! `push` endpoint (or call it from a sidecar) so every push is scanned; if the
//! scanner fails the endpoint returns an error and the digest stays unscanned,
//! so it is denied rather than waved through.

use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use serde::Deserialize;

use crate::controller::cve::RegistryScannerClient;
use crate::controller::registry_gate::{PullDecision, ScanReport};
use crate::controller::registry_gate_registry as gates;

use super::dto::{ApiErrorCode, ErrorResponse};

type ApiError = (StatusCode, Json<ErrorResponse>);

fn error(status: StatusCode, code: ApiErrorCode, message: &str) -> ApiError {
    (status, Json(ErrorResponse::structured(code, message, None)))
}

/// An OCI content digest: `sha256:` plus 64 hex digits, or `sha512:` plus 128.
pub fn is_valid_digest(digest: &str) -> bool {
    let Some((algorithm, hex)) = digest.split_once(':') else {
        return false;
    };
    let expected_len = match algorithm {
        "sha256" => 64,
        "sha512" => 128,
        _ => return false,
    };
    hex.len() == expected_len && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Turn a decision into the HTTP response a registry proxy forwards: denials
/// become a `403` OCI distribution error, everything else a `200` with the
/// decision body.
pub fn decision_response(decision: &PullDecision, digest: &str) -> Response {
    match decision.oci_response(digest) {
        Some((status, body)) => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::FORBIDDEN),
            Json(body),
        )
            .into_response(),
        None => (StatusCode::OK, Json(decision)).into_response(),
    }
}

/// `GET /api/v1/registry/:namespace/:name/pull-gate/:digest`
pub async fn authorize_pull(
    Path((namespace, name, digest)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    if !is_valid_digest(&digest) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::ErrBadRequest,
            "digest must look like sha256:<64 hex digits>; resolve tags to digests first",
        ));
    }
    let Some(gate) = gates::global().get(&namespace, &name).await else {
        return Err(error(
            StatusCode::NOT_FOUND,
            ApiErrorCode::ErrNotFound,
            &format!("no pull gate is registered for StellarRegistry {namespace}/{name}"),
        ));
    };
    let decision = gate.authorize_pull(&digest).await;
    Ok(decision_response(&decision, &digest))
}

/// Body of `POST /api/v1/registry/:namespace/:name/push`.
#[derive(Debug, Deserialize)]
pub struct PushNotification {
    /// Repository the manifest was pushed to, e.g. `stellar/validator`.
    pub repository: String,
    /// Digest of the pushed manifest.
    pub digest: String,
}

/// A repository name as the OCI distribution spec allows it: lowercase
/// alphanumeric components joined by `.`, `_`, `-` or `/`.
pub fn is_valid_repository(repository: &str) -> bool {
    !repository.is_empty()
        && repository.len() <= 255
        && !repository.starts_with('/')
        && !repository.ends_with('/')
        && !repository.contains("//")
        && repository
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' | b'/'))
}

/// `POST /api/v1/registry/:namespace/:name/push`
///
/// Scans the pushed digest synchronously and stores its report. The caller
/// acknowledges the push only on success.
pub async fn handle_push(
    Path((namespace, name)): Path<(String, String)>,
    Json(push): Json<PushNotification>,
) -> Result<Json<ScanReport>, ApiError> {
    if !is_valid_digest(&push.digest) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::ErrBadRequest,
            "digest must look like sha256:<64 hex digits>",
        ));
    }
    if !is_valid_repository(&push.repository) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::ErrBadRequest,
            "repository must be lowercase alphanumeric components joined by '.', '_', '-' or '/'",
        ));
    }
    let registry = gates::global();
    let Some(gate) = registry.get(&namespace, &name).await else {
        return Err(error(
            StatusCode::NOT_FOUND,
            ApiErrorCode::ErrNotFound,
            &format!("no pull gate is registered for StellarRegistry {namespace}/{name}"),
        ));
    };
    let Some(endpoint) = registry.scanner_endpoint(&namespace, &name).await else {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            ApiErrorCode::ErrServiceUnavailable,
            &format!("scanning is disabled for StellarRegistry {namespace}/{name}; the digest stays unscanned"),
        ));
    };

    let scanner = RegistryScannerClient::new(endpoint, None);
    gate.handle_push(&scanner, &push.repository, &push.digest)
        .await
        .map(Json)
        .map_err(|scan_error| {
            error(
                StatusCode::BAD_GATEWAY,
                ApiErrorCode::ErrServiceUnavailable,
                &format!(
                    "scan failed, so {} stays unscanned: {scan_error}",
                    push.digest
                ),
            )
        })
}

/// `GET /api/v1/registry/scan-reports/:digest`
pub async fn get_scan_report(Path(digest): Path<String>) -> Result<Json<ScanReport>, ApiError> {
    if !is_valid_digest(&digest) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            ApiErrorCode::ErrBadRequest,
            "digest must look like sha256:<64 hex digits>",
        ));
    }
    gates::global()
        .find_report(&digest)
        .await
        .map(Json)
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                ApiErrorCode::ErrNotFound,
                &format!("no scan report for {digest}; the digest has not been scanned"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn accepts_well_formed_digests_only() {
        assert!(is_valid_digest(DIGEST));
        assert!(is_valid_digest(&format!("sha512:{}", "a".repeat(128))));
        assert!(!is_valid_digest("latest"));
        assert!(!is_valid_digest("sha256:short"));
        assert!(!is_valid_digest(&format!("md5:{}", "a".repeat(32))));
        assert!(!is_valid_digest(&format!("sha256:{}", "G".repeat(64))));
        assert!(!is_valid_digest(&format!("sha256:{}", "A".repeat(64))));
    }

    #[test]
    fn deny_maps_to_403() {
        let decision = PullDecision::Deny {
            reason: "never scanned".to_string(),
            report_url: Some("http://reports/x".to_string()),
        };
        assert_eq!(
            decision_response(&decision, DIGEST).status(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn allow_and_audit_map_to_200() {
        assert_eq!(
            decision_response(&PullDecision::Allow, DIGEST).status(),
            StatusCode::OK
        );
        let audited = PullDecision::AllowAudited {
            reason: "would deny".to_string(),
        };
        assert_eq!(decision_response(&audited, DIGEST).status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_registry_is_a_404_and_bad_digest_a_400() {
        let missing = authorize_pull(Path((
            "ns-that-does-not-exist".to_string(),
            "reg".to_string(),
            DIGEST.to_string(),
        )))
        .await;
        assert_eq!(missing.unwrap_err().0, StatusCode::NOT_FOUND);

        let bad = authorize_pull(Path((
            "ns".to_string(),
            "reg".to_string(),
            "latest".to_string(),
        )))
        .await;
        assert_eq!(bad.unwrap_err().0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn accepts_oci_repository_names_only() {
        assert!(is_valid_repository("app"));
        assert!(is_valid_repository("stellar/validator-core.v2"));
        assert!(!is_valid_repository(""));
        assert!(!is_valid_repository("/app"));
        assert!(!is_valid_repository("app/"));
        assert!(!is_valid_repository("a//b"));
        assert!(!is_valid_repository("App"));
        assert!(!is_valid_repository("app name"));
        assert!(!is_valid_repository(&"a".repeat(256)));
    }

    #[tokio::test]
    async fn push_validates_input_before_looking_up_the_registry() {
        let bad_digest = handle_push(
            Path(("ns".to_string(), "reg".to_string())),
            Json(PushNotification {
                repository: "app".to_string(),
                digest: "latest".to_string(),
            }),
        )
        .await;
        assert_eq!(bad_digest.unwrap_err().0, StatusCode::BAD_REQUEST);

        let bad_repo = handle_push(
            Path(("ns".to_string(), "reg".to_string())),
            Json(PushNotification {
                repository: "Bad Repo".to_string(),
                digest: DIGEST.to_string(),
            }),
        )
        .await;
        assert_eq!(bad_repo.unwrap_err().0, StatusCode::BAD_REQUEST);

        let unknown = handle_push(
            Path(("ns-without-gate".to_string(), "reg".to_string())),
            Json(PushNotification {
                repository: "app".to_string(),
                digest: DIGEST.to_string(),
            }),
        )
        .await;
        assert_eq!(unknown.unwrap_err().0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn report_for_an_unscanned_digest_is_a_404() {
        let result = get_scan_report(Path(DIGEST.to_string())).await;
        assert_eq!(result.unwrap_err().0, StatusCode::NOT_FOUND);
    }
}
