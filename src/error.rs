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
//! Central error types for the Stellar-K8s operator
//!
//! Uses `thiserror` for ergonomic, type-safe error handling with
//! automatic `Display` and `Error` trait implementations.
//!
//! # HTTP error mapping (issue #1393)
//!
//! [`Error::status_code`] and [`Error::api_error_code`] map every `Error`
//! variant to an HTTP status and a stable [`ApiErrorCode`] so that all REST
//! surfaces (the operator's `rest_api` module and the `api_gateway` proxy)
//! render failures through the same [`ErrorResponse`] JSON envelope. See
//! `docs/errors.md` for the full mapping table and the correlation-ID
//! mechanism that populates `ErrorResponse::correlation_id`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Central error type for the Stellar-K8s operator
#[derive(Error, Debug)]
pub enum Error {
    /// Triggered when an operation with the Kubernetes API server fails.
    /// This includes connection timeouts, permission denied (RBAC), or
    /// resource conflicts during a Patch or Update.
    #[error("[SK8S-001] Kubernetes API error: {0}")]
    KubeError(#[from] kube::Error),

    /// Occurs when failing to parse JSON from the Kubernetes API or
    /// when serializing internal state (like status conditions) into the CRD.
    #[error("[SK8S-002] Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Represents a failure during the resource deletion phase.
    /// If a finalizer cannot be removed, the resource will remain in a 'Terminating' state.
    #[error("[SK8S-003] Finalizer error: {0}")]
    FinalizerError(String),

    /// A catch-all for invalid operator configuration, such as missing
    /// environment variables or malformed ConfigMaps used for feature flags.
    #[error("[SK8S-004] Configuration error: {0}")]
    ConfigError(String),

    /// Triggered during pre-reconciliation validation if the StellarNode spec
    /// violates business logic (e.g., mutually exclusive flags or invalid replicas).
    #[error("[SK8S-005] Node validation error: {0}")]
    ValidationError(String),

    /// The requested Kubernetes resource (Pod, Secret, etc.) was not found.
    /// Usually implies a dependency hasn't been created yet.
    #[error("[SK8S-006] Resource not found: {kind}/{name} in namespace {namespace}")]
    NotFound {
        kind: String,
        name: String,
        namespace: String,
    },

    /// The `nodeType` provided in the spec is not recognized by this version of the operator.
    #[error("[SK8S-007] Invalid node type: {0}")]
    InvalidNodeType(String),

    /// A required field for the specific node type (e.g., `seedSecretRef` for Validators) is missing.
    #[error("[SK8S-008] Missing required field: {field} for node type {node_type}")]
    MissingRequiredField { field: String, node_type: String },

    /// Failure during the background integrity check of a history archive bucket.
    #[error("[SK8S-009] Archive health check failed: {0}")]
    ArchiveHealthCheckError(String),

    /// Generic HTTP failure, often from querying the Stellar network's SCP status or Horizon health.
    #[error("[SK8S-010] HTTP request error: {0}")]
    HttpError(#[from] reqwest::Error),

    /// An automated remediation action (e.g., restarting a stuck pod) failed to execute.
    #[error("[SK8S-011] Remediation failed: {0}")]
    RemediationError(String),

    /// Error returned by a WASM admission plugin during the validation phase.
    #[error("[SK8S-012] Plugin error: {0}")]
    PluginError(String),

    /// Failure within the internal admission webhook server (e.g., port binding issues).
    #[error("[SK8S-013] Webhook error: {0}")]
    WebhookError(String),

    /// Connectivity issues between operator components or between the operator and the cluster.
    #[error("[SK8S-014] Network error: {0}")]
    NetworkError(String),

    /// Failure to generate or rotate TLS certificates for mTLS or webhooks.
    #[error("[SK8S-015] Certificate error: {0}")]
    CertificateError(#[from] rcgen::Error),

    /// File system interaction failure, usually related to local configuration or WASM module loading.
    #[error("[SK8S-016] I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Failure during database schema migrations or automated vacuum/pruning tasks.
    #[error("[SK8S-017] Database maintenance error: {0}")]
    MaintenanceError(String),

    /// Error from the SQLx driver during database interactions.
    #[error("[SK8S-018] SQL error: {0}")]
    SqlxError(#[from] sqlx::Error),

    /// Failure to load or parse the local Kubeconfig file.
    #[error("[SK8S-019] Kubeconfig error: {0}")]
    KubeconfigError(#[from] kube::config::KubeconfigError),

    /// Failure during the compression or extraction of node snapshots.
    #[error("[SK8S-020] Zip error: {0}")]
    ZipError(#[from] zip::result::ZipError),

    /// A security violation where nodes from different networks (e.g., Mainnet and Testnet)
    /// are detected in the same namespace, which could lead to ledger contamination.
    #[error("[SK8S-021] {0}")]
    NetworkSafetyViolation(#[from] crate::controller::network_isolation::NetworkSafetyViolation),

    /// An unexpected internal state error that doesn't fit other categories.
    #[error("[SK8S-022] Internal error: {0}")]
    InternalError(String),

    /// The reconciler attempted a phase transition that the state machine in
    /// [`crate::controller::phases`] does not permit. This always indicates a
    /// bug in the reconcile pipeline rather than a bad user input.
    #[error("[SK8S-023] Invalid reconcile phase transition: {0}")]
    PhaseTransitionError(String),
}

/// Result type alias for operator operations
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Standardised API Error Codes for REST endpoints (issue #1282, extended by #1393).
///
/// Shared by `rest_api` and `api_gateway` so both surfaces report failures
/// under the same stable, machine-readable vocabulary regardless of which
/// process produced the response.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiErrorCode {
    ErrNotFound,
    ErrBadRequest,
    ErrUnauthorized,
    ErrForbidden,
    ErrInternalServerError,
    ErrServiceUnavailable,
    ErrPartialDegradation,
    ErrReconcileStalled,
    /// Caller exceeded a rate limit or quota (`api_gateway`, issue #1393).
    ErrRateLimited,
    /// The requested API version has been sunset and no longer routes
    /// anywhere (`api_gateway`, issue #1393).
    ErrGone,
}

impl ApiErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ErrNotFound => "ERR_NOT_FOUND",
            Self::ErrBadRequest => "ERR_BAD_REQUEST",
            Self::ErrUnauthorized => "ERR_UNAUTHORIZED",
            Self::ErrForbidden => "ERR_FORBIDDEN",
            Self::ErrInternalServerError => "ERR_INTERNAL_SERVER_ERROR",
            Self::ErrServiceUnavailable => "ERR_SERVICE_UNAVAILABLE",
            Self::ErrPartialDegradation => "ERR_PARTIAL_DEGRADATION",
            Self::ErrReconcileStalled => "ERR_RECONCILE_STALLED",
            Self::ErrRateLimited => "ERR_RATE_LIMITED",
            Self::ErrGone => "ERR_GONE",
        }
    }
}

/// Structured error response for all REST API endpoints (`rest_api` and `api_gateway`).
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    pub error_code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub degraded: bool,
    pub timestamp: String,
}

impl ErrorResponse {
    pub fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            error_code: ApiErrorCode::ErrInternalServerError.as_str().to_string(),
            message: message.to_string(),
            correlation_id: None,
            details: None,
            degraded: false,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn structured(code: ApiErrorCode, message: &str, correlation_id: Option<String>) -> Self {
        Self {
            error: code.as_str().to_lowercase(),
            error_code: code.as_str().to_string(),
            message: message.to_string(),
            correlation_id,
            details: None,
            degraded: false,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Build a response for a partial-failure scenario where a (possibly
    /// stale) result is still returned alongside the error context, e.g. a
    /// gateway upstream that failed but a cached/last-known-good body is
    /// available, or an aggregate endpoint where some sub-queries failed.
    /// `degraded` is set to `true` so clients can distinguish "the data you
    /// got back may be incomplete" from a hard failure.
    pub fn degraded(
        code: ApiErrorCode,
        message: &str,
        details: serde_json::Value,
        correlation_id: Option<String>,
    ) -> Self {
        Self {
            error: code.as_str().to_lowercase(),
            error_code: code.as_str().to_string(),
            message: message.to_string(),
            correlation_id,
            details: Some(details),
            degraded: true,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Format a user-facing diagnostic message with an explicit pipeline step.
///
/// Example: `diagnostic("load kubeconfig", "file not found at /etc/kube/config")`
/// → `"[load kubeconfig] file not found at /etc/kube/config"`
pub fn diagnostic(step: &str, detail: impl std::fmt::Display) -> String {
    format!("[{step}] {detail}")
}

impl Error {
    /// Build a configuration error that names the failing step.
    pub fn config_step(step: &str, detail: impl std::fmt::Display) -> Self {
        Error::ConfigError(diagnostic(step, detail))
    }

    /// Build an internal error that names the failing step.
    pub fn internal_step(step: &str, detail: impl std::fmt::Display) -> Self {
        Error::InternalError(diagnostic(step, detail))
    }

    /// Build a validation error that names the failing step.
    pub fn validation_step(step: &str, detail: impl std::fmt::Display) -> Self {
        Error::ValidationError(diagnostic(step, detail))
    }

    /// Check if this error type should trigger a retry
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            Error::KubeError(_) | Error::FinalizerError(_) | Error::RemediationError(_)
        )
    }

    /// Convert to a human-readable message for status updates.
    ///
    /// Delegates to `Display` so there is a single source of truth for error formatting.
    pub fn status_message(&self) -> String {
        self.to_string()
    }

    /// Map this error to the process exit code documented in
    /// `docs/cli-commands-reference.md#exit-codes`.
    ///
    /// Rust's default `Termination` impl for `Result<(), Error>` always exits
    /// with code 1 on `Err`, regardless of the error variant, so every CLI
    /// command was silently ignoring the documented 2/3/4 exit codes unless it
    /// special-cased `std::process::exit` itself. Routing every command's
    /// error through this method instead keeps the exit code consistent with
    /// the error category everywhere.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::ValidationError(_)
            | Error::InvalidNodeType(_)
            | Error::MissingRequiredField { .. } => 2,
            Error::KubeError(_)
            | Error::KubeconfigError(_)
            | Error::FinalizerError(_)
            | Error::NotFound { .. } => 3,
            Error::ConfigError(_) | Error::MaintenanceError(_) | Error::CertificateError(_) => 4,
            _ => 1,
        }
    }

    /// Map this error to the [`ApiErrorCode`] reported in a REST API
    /// [`ErrorResponse`] body (issue #1393). Kept independent of
    /// [`Error::status_code`] so it works even without the `rest-api` /
    /// `admission-webhook` features (no `axum` type involved).
    pub fn api_error_code(&self) -> ApiErrorCode {
        match self {
            Error::NotFound { .. } => ApiErrorCode::ErrNotFound,
            Error::ValidationError(_)
            | Error::InvalidNodeType(_)
            | Error::MissingRequiredField { .. }
            | Error::SerializationError(_) => ApiErrorCode::ErrBadRequest,
            Error::PhaseTransitionError(_) => ApiErrorCode::ErrReconcileStalled,
            Error::KubeError(_)
            | Error::HttpError(_)
            | Error::NetworkError(_)
            | Error::KubeconfigError(_) => ApiErrorCode::ErrServiceUnavailable,
            _ => ApiErrorCode::ErrInternalServerError,
        }
    }

    /// Map this error to the HTTP status code a REST handler should return
    /// (issue #1393). Variants are grouped by whether the failure is caused
    /// by the caller (4xx) or by the operator / an upstream dependency
    /// (5xx); see `docs/errors.md` for the rationale behind each mapping.
    #[cfg(any(feature = "rest-api", feature = "admission-webhook"))]
    pub fn status_code(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;
        match self {
            // Caller error: the requested resource does not exist.
            Error::NotFound { .. } => StatusCode::NOT_FOUND,

            // Caller error: the request itself is malformed or fails
            // business-logic validation.
            Error::ValidationError(_)
            | Error::InvalidNodeType(_)
            | Error::MissingRequiredField { .. }
            | Error::SerializationError(_) => StatusCode::BAD_REQUEST,

            // The reconciler's state machine refused a transition; this is
            // never the caller's fault, but it is also not a transient
            // upstream failure, so it is reported as a conflict.
            Error::PhaseTransitionError(_) => StatusCode::CONFLICT,

            // Upstream / dependency unavailable — safe to retry.
            Error::KubeError(_)
            | Error::HttpError(_)
            | Error::NetworkError(_)
            | Error::KubeconfigError(_) => StatusCode::SERVICE_UNAVAILABLE,

            // Everything else (config errors, certificate/IO/SQL failures,
            // internal invariants, etc.) is an operator-side fault.
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Build a REST [`ErrorResponse`] body for this error, tagging it with
    /// the given correlation ID (issue #1393). Pairs with
    /// [`Error::status_code`] to build the full `(StatusCode, Json<ErrorResponse>)`
    /// axum response.
    #[cfg(any(feature = "rest-api", feature = "admission-webhook"))]
    pub fn to_error_response(&self, correlation_id: Option<String>) -> ErrorResponse {
        ErrorResponse::structured(self.api_error_code(), &self.to_string(), correlation_id)
    }
}

impl From<&Error> for ApiErrorCode {
    fn from(err: &Error) -> Self {
        err.api_error_code()
    }
}

// Implement From for kube::runtime::finalizer::Error to enable ? operator
impl From<kube::runtime::finalizer::Error<Error>> for Error {
    fn from(e: kube::runtime::finalizer::Error<Error>) -> Self {
        Error::FinalizerError(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_includes_step_and_detail() {
        let msg = diagnostic("validate source", "path does not exist: /tmp/data");
        assert_eq!(msg, "[validate source] path does not exist: /tmp/data");
    }

    #[test]
    fn test_config_step_wraps_diagnostic() {
        let err = Error::config_step("parse db uri", "missing database name");
        assert_eq!(
            err.to_string(),
            "[SK8S-004] Configuration error: [parse db uri] missing database name"
        );
    }

    #[test]
    fn test_kube_error_conversion() {
        // Test that kube errors are correctly mapped to our internal Error type.
        // We use wiremock to create realistic kube::Error instances and verify
        // they are properly converted via the #[from] attribute.

        // Note: kube::Error variants (Api, SerdeError, BuildRequest, etc.) are
        // converted to Error::KubeError automatically via #[from].
        // This test verifies the conversion preserves error information.

        // Test with a SerdeError variant (easiest to construct for testing)
        let kube_serde_err = kube::Error::SerdeError(
            serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err(),
        );

        // Convert to our error type
        let our_err = Error::from(kube_serde_err);

        // Verify it's mapped to Error::KubeError
        assert!(matches!(our_err, Error::KubeError(_)));

        // Verify error code is present in message
        let msg = our_err.to_string();
        assert!(msg.contains("[SK8S-001]"));
        assert!(msg.contains("Kubernetes API error"));
    }

    #[test]
    fn test_kube_error_is_retriable() {
        // Test that KubeError is correctly identified as retriable
        let kube_serde_err = kube::Error::SerdeError(
            serde_json::from_str::<serde_json::Value>("invalid").unwrap_err(),
        );
        let our_err = Error::KubeError(kube_serde_err);
        assert!(our_err.is_retriable());
    }

    #[test]
    fn test_kube_error_status_message() {
        // status_message delegates to Display, so the output must match to_string().
        let kube_serde_err =
            kube::Error::SerdeError(serde_json::from_str::<serde_json::Value>("bad").unwrap_err());
        let our_err = Error::KubeError(kube_serde_err);
        assert_eq!(our_err.status_message(), our_err.to_string());
        assert!(our_err.status_message().contains("[SK8S-001]"));
    }

    #[test]
    fn test_kube_error_display_trait() {
        // Verify Display trait implementation for KubeError
        let kube_serde_err =
            kube::Error::SerdeError(serde_json::from_str::<serde_json::Value>("x").unwrap_err());
        let our_err = Error::KubeError(kube_serde_err);

        // The Display implementation should include the error code prefix
        let display = format!("{our_err}");
        assert!(display.contains("[SK8S-001]"));
        assert!(display.contains("Kubernetes API error"));
    }

    #[test]
    fn test_kube_api_error_pattern_matching() {
        // Test that we can pattern match on KubeError to extract inner error
        let kube_err =
            kube::Error::SerdeError(serde_json::from_str::<serde_json::Value>("y").unwrap_err());
        let our_err = Error::KubeError(kube_err);

        // Verify we can match and extract the inner kube error
        match our_err {
            Error::KubeError(e) => {
                // The inner error should be the same as what we put in
                assert!(matches!(e, kube::Error::SerdeError(_)));
            }
            _ => panic!("Expected Error::KubeError"),
        }
    }

    #[test]
    fn test_error_code_formatting() {
        // We only instantiate the errors that we can easily construct without complex external types.
        let finalizer_err = Error::FinalizerError("test".to_string());
        assert_eq!(
            finalizer_err.to_string(),
            "[SK8S-003] Finalizer error: test"
        );
        assert_eq!(
            finalizer_err.status_message(),
            "[SK8S-003] Finalizer error: test"
        );

        let config_err = Error::ConfigError("invalid config".to_string());
        assert_eq!(
            config_err.to_string(),
            "[SK8S-004] Configuration error: invalid config"
        );
        assert_eq!(
            config_err.status_message(),
            "[SK8S-004] Configuration error: invalid config"
        );

        // Since status_message() now delegates to Display, to_string() == status_message().
        let validation_err = Error::ValidationError("invalid".to_string());
        assert_eq!(
            validation_err.to_string(),
            "[SK8S-005] Node validation error: invalid"
        );
        assert_eq!(validation_err.status_message(), validation_err.to_string());

        let not_found_err = Error::NotFound {
            kind: "Pod".to_string(),
            name: "test-pod".to_string(),
            namespace: "default".to_string(),
        };
        assert_eq!(
            not_found_err.to_string(),
            "[SK8S-006] Resource not found: Pod/test-pod in namespace default"
        );
        assert_eq!(not_found_err.status_message(), not_found_err.to_string());

        let invalid_node_err = Error::InvalidNodeType("bad_type".to_string());
        assert_eq!(
            invalid_node_err.to_string(),
            "[SK8S-007] Invalid node type: bad_type"
        );
        assert_eq!(
            invalid_node_err.status_message(),
            invalid_node_err.to_string()
        );

        let missing_field_err = Error::MissingRequiredField {
            field: "image".to_string(),
            node_type: "core".to_string(),
        };
        assert_eq!(
            missing_field_err.to_string(),
            "[SK8S-008] Missing required field: image for node type core"
        );
        assert_eq!(
            missing_field_err.status_message(),
            missing_field_err.to_string()
        );

        let archive_health_err = Error::ArchiveHealthCheckError("unreachable".to_string());
        assert_eq!(
            archive_health_err.to_string(),
            "[SK8S-009] Archive health check failed: unreachable"
        );
        assert_eq!(
            archive_health_err.status_message(),
            archive_health_err.to_string()
        );

        let remediation_err = Error::RemediationError("failed to restart".to_string());
        assert_eq!(
            remediation_err.to_string(),
            "[SK8S-011] Remediation failed: failed to restart"
        );
        assert_eq!(
            remediation_err.status_message(),
            remediation_err.to_string()
        );

        let plugin_err = Error::PluginError("crash".to_string());
        assert_eq!(plugin_err.to_string(), "[SK8S-012] Plugin error: crash");
        assert_eq!(plugin_err.status_message(), plugin_err.to_string());

        let webhook_err = Error::WebhookError("timeout".to_string());
        assert_eq!(webhook_err.to_string(), "[SK8S-013] Webhook error: timeout");
        assert_eq!(webhook_err.status_message(), webhook_err.to_string());

        let network_err = Error::NetworkError("offline".to_string());
        assert_eq!(network_err.to_string(), "[SK8S-014] Network error: offline");
        assert_eq!(network_err.status_message(), network_err.to_string());

        let io_err = Error::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        assert_eq!(io_err.to_string(), "[SK8S-016] I/O error: file not found");
        assert_eq!(io_err.status_message(), io_err.to_string());

        let maintenance_err = Error::MaintenanceError("db locked".to_string());
        assert_eq!(
            maintenance_err.to_string(),
            "[SK8S-017] Database maintenance error: db locked"
        );
        assert_eq!(
            maintenance_err.status_message(),
            maintenance_err.to_string()
        );
    }

    #[test]
    fn test_api_error_code_mapping() {
        let not_found = Error::NotFound {
            kind: "Pod".to_string(),
            name: "p".to_string(),
            namespace: "ns".to_string(),
        };
        assert_eq!(not_found.api_error_code(), ApiErrorCode::ErrNotFound);
        assert_eq!(ApiErrorCode::from(&not_found), ApiErrorCode::ErrNotFound);

        assert_eq!(
            Error::ValidationError("bad".into()).api_error_code(),
            ApiErrorCode::ErrBadRequest
        );
        assert_eq!(
            Error::InvalidNodeType("bad".into()).api_error_code(),
            ApiErrorCode::ErrBadRequest
        );
        assert_eq!(
            Error::MissingRequiredField {
                field: "f".into(),
                node_type: "core".into()
            }
            .api_error_code(),
            ApiErrorCode::ErrBadRequest
        );

        assert_eq!(
            Error::PhaseTransitionError("bad transition".into()).api_error_code(),
            ApiErrorCode::ErrReconcileStalled
        );

        assert_eq!(
            Error::NetworkError("offline".into()).api_error_code(),
            ApiErrorCode::ErrServiceUnavailable
        );

        assert_eq!(
            Error::InternalError("boom".into()).api_error_code(),
            ApiErrorCode::ErrInternalServerError
        );
        assert_eq!(
            Error::CertificateError(rcgen::Error::UnsupportedSignatureAlgorithm).api_error_code(),
            ApiErrorCode::ErrInternalServerError
        );
    }

    #[cfg(any(feature = "rest-api", feature = "admission-webhook"))]
    #[test]
    fn test_status_code_mapping() {
        use axum::http::StatusCode;

        let not_found = Error::NotFound {
            kind: "Pod".to_string(),
            name: "p".to_string(),
            namespace: "ns".to_string(),
        };
        assert_eq!(not_found.status_code(), StatusCode::NOT_FOUND);

        assert_eq!(
            Error::ValidationError("bad".into()).status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            Error::MissingRequiredField {
                field: "f".into(),
                node_type: "core".into()
            }
            .status_code(),
            StatusCode::BAD_REQUEST
        );

        assert_eq!(
            Error::PhaseTransitionError("bad transition".into()).status_code(),
            StatusCode::CONFLICT
        );

        assert_eq!(
            Error::NetworkError("offline".into()).status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        assert_eq!(
            Error::InternalError("boom".into()).status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            Error::ConfigError("bad config".into()).status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[cfg(any(feature = "rest-api", feature = "admission-webhook"))]
    #[test]
    fn test_to_error_response_carries_correlation_id_and_code() {
        let err = Error::NotFound {
            kind: "Pod".to_string(),
            name: "p".to_string(),
            namespace: "ns".to_string(),
        };
        let resp = err.to_error_response(Some("corr-123".to_string()));
        assert_eq!(resp.error_code, "ERR_NOT_FOUND");
        assert_eq!(resp.correlation_id.as_deref(), Some("corr-123"));
        assert!(!resp.degraded);
        assert!(resp.message.contains("SK8S-006"));
    }

    #[test]
    fn test_error_response_degraded_sets_flag_and_details() {
        let resp = ErrorResponse::degraded(
            ApiErrorCode::ErrPartialDegradation,
            "upstream partially unavailable",
            serde_json::json!({"stale": true}),
            Some("corr-456".to_string()),
        );
        assert!(resp.degraded);
        assert_eq!(resp.error_code, "ERR_PARTIAL_DEGRADATION");
        assert_eq!(resp.correlation_id.as_deref(), Some("corr-456"));
        assert_eq!(resp.details, Some(serde_json::json!({"stale": true})));
    }
}
