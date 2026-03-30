//! Central error types for the Stellar-K8s operator
//!
//! Uses `thiserror` for ergonomic, type-safe error handling with
//! automatic `Display` and `Error` trait implementations.

use thiserror::Error;

/// Central error type for the Stellar-K8s operator
#[derive(Error, Debug)]
pub enum Error {
    /// Kubernetes API error from kube-rs
    #[error("[SK8S-001] Kubernetes API error: {0}")]
    KubeError(#[from] kube::Error),

    /// JSON serialization/deserialization error
    #[error("[SK8S-002] Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Finalizer-related error during cleanup
    #[error("[SK8S-003] Finalizer error: {0}")]
    FinalizerError(String),

    /// Configuration validation error
    #[error("[SK8S-004] Configuration error: {0}")]
    ConfigError(String),

    /// Node spec validation error
    #[error("[SK8S-005] Node validation error: {0}")]
    ValidationError(String),

    /// Resource not found in the cluster
    #[error("[SK8S-006] Resource not found: {kind}/{name} in namespace {namespace}")]
    NotFound {
        kind: String,
        name: String,
        namespace: String,
    },

    /// Invalid node type specified
    #[error("[SK8S-007] Invalid node type: {0}")]
    InvalidNodeType(String),

    /// Missing required field in spec
    #[error("[SK8S-008] Missing required field: {field} for node type {node_type}")]
    MissingRequiredField { field: String, node_type: String },

    /// History archive health check error
    #[error("[SK8S-009] Archive health check failed: {0}")]
    ArchiveHealthCheckError(String),

    /// HTTP request error (from reqwest)
    #[error("[SK8S-010] HTTP request error: {0}")]
    HttpError(#[from] reqwest::Error),

    /// Remediation action failed
    #[error("[SK8S-011] Remediation failed: {0}")]
    RemediationError(String),

    /// Wasm plugin error
    #[error("[SK8S-012] Plugin error: {0}")]
    PluginError(String),

    /// Webhook server error
    #[error("[SK8S-013] Webhook error: {0}")]
    WebhookError(String),

    /// Network connectivity error
    #[error("[SK8S-014] Network error: {0}")]
    NetworkError(String),

    /// Certificate generation error
    #[error("[SK8S-015] Certificate error: {0}")]
    CertificateError(#[from] rcgen::Error),

    /// I/O error
    #[error("[SK8S-016] I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Database maintenance error
    #[error("[SK8S-017] Database maintenance error: {0}")]
    MaintenanceError(String),

    /// SQLx error
    #[error("[SK8S-018] SQL error: {0}")]
    SqlxError(#[from] sqlx::Error),

    /// Kubeconfig error
    #[error("[SK8S-019] Kubeconfig error: {0}")]
    KubeconfigError(#[from] kube::config::KubeconfigError),
}

/// Result type alias for operator operations
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// Check if this error type should trigger a retry
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            Error::KubeError(_) | Error::FinalizerError(_) | Error::RemediationError(_)
        )
    }

    /// Convert to a human-readable message for status updates
    pub fn status_message(&self) -> String {
        match self {
            Error::KubeError(e) => format!("[SK8S-001] Kubernetes error: {e}"),
            Error::SerializationError(e) => format!("[SK8S-002] Serialization error: {e}"),
            Error::FinalizerError(msg) => format!("[SK8S-003] Finalizer error: {msg}"),
            Error::ConfigError(msg) => format!("[SK8S-004] Configuration error: {msg}"),
            Error::ValidationError(msg) => format!("[SK8S-005] Validation failed: {msg}"),
            Error::NotFound {
                kind,
                name,
                namespace,
            } => format!("[SK8S-006] Resource not found: {kind}/{name} in namespace {namespace}"),
            Error::InvalidNodeType(msg) => format!("[SK8S-007] Invalid node type: {msg}"),
            Error::MissingRequiredField { field, node_type } => {
                format!("[SK8S-008] Missing {field} for {node_type} node")
            }
            Error::ArchiveHealthCheckError(msg) => {
                format!("[SK8S-009] Archive health check failed: {msg}")
            }
            Error::HttpError(e) => format!("[SK8S-010] HTTP request failed: {e}"),
            Error::RemediationError(msg) => format!("[SK8S-011] Remediation failed: {msg}"),
            Error::PluginError(msg) => format!("[SK8S-012] Plugin error: {msg}"),
            Error::WebhookError(msg) => format!("[SK8S-013] Webhook error: {msg}"),
            Error::NetworkError(msg) => format!("[SK8S-014] Network error: {msg}"),
            Error::CertificateError(e) => format!("[SK8S-015] Certificate error: {e}"),
            Error::IoError(e) => format!("[SK8S-016] I/O error: {e}"),
            Error::MaintenanceError(msg) => format!("[SK8S-017] Database maintenance error: {msg}"),
            Error::SqlxError(e) => format!("[SK8S-018] SQL error: {e}"),
            Error::KubeconfigError(e) => format!("[SK8S-019] Kubeconfig error: {e}"),
        }
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
        // Test that KubeError status_message includes error code and description
        let kube_serde_err =
            kube::Error::SerdeError(serde_json::from_str::<serde_json::Value>("bad").unwrap_err());
        let our_err = Error::KubeError(kube_serde_err);
        let status = our_err.status_message();
        assert!(status.contains("[SK8S-001]"));
        assert!(status.contains("Kubernetes error"));
    }

    #[test]
    fn test_kube_error_display_trait() {
        // Verify Display trait implementation for KubeError
        let kube_serde_err =
            kube::Error::SerdeError(serde_json::from_str::<serde_json::Value>("x").unwrap_err());
        let our_err = Error::KubeError(kube_serde_err);

        // The Display implementation should include the error code prefix
        let display = format!("{}", our_err);
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

        let validation_err = Error::ValidationError("invalid".to_string());
        assert_eq!(
            validation_err.to_string(),
            "[SK8S-005] Node validation error: invalid"
        );
        assert_eq!(
            validation_err.status_message(),
            "[SK8S-005] Validation failed: invalid"
        );

        let not_found_err = Error::NotFound {
            kind: "Pod".to_string(),
            name: "test-pod".to_string(),
            namespace: "default".to_string(),
        };
        assert_eq!(
            not_found_err.to_string(),
            "[SK8S-006] Resource not found: Pod/test-pod in namespace default"
        );
        assert_eq!(
            not_found_err.status_message(),
            "[SK8S-006] Resource not found: Pod/test-pod in namespace default"
        );

        let invalid_node_err = Error::InvalidNodeType("bad_type".to_string());
        assert_eq!(
            invalid_node_err.to_string(),
            "[SK8S-007] Invalid node type: bad_type"
        );
        assert_eq!(
            invalid_node_err.status_message(),
            "[SK8S-007] Invalid node type: bad_type"
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
            "[SK8S-008] Missing image for core node"
        );

        let archive_health_err = Error::ArchiveHealthCheckError("unreachable".to_string());
        assert_eq!(
            archive_health_err.to_string(),
            "[SK8S-009] Archive health check failed: unreachable"
        );
        assert_eq!(
            archive_health_err.status_message(),
            "[SK8S-009] Archive health check failed: unreachable"
        );

        let remediation_err = Error::RemediationError("failed to restart".to_string());
        assert_eq!(
            remediation_err.to_string(),
            "[SK8S-011] Remediation failed: failed to restart"
        );
        assert_eq!(
            remediation_err.status_message(),
            "[SK8S-011] Remediation failed: failed to restart"
        );

        let plugin_err = Error::PluginError("crash".to_string());
        assert_eq!(plugin_err.to_string(), "[SK8S-012] Plugin error: crash");
        assert_eq!(
            plugin_err.status_message(),
            "[SK8S-012] Plugin error: crash"
        );

        let webhook_err = Error::WebhookError("timeout".to_string());
        assert_eq!(webhook_err.to_string(), "[SK8S-013] Webhook error: timeout");
        assert_eq!(
            webhook_err.status_message(),
            "[SK8S-013] Webhook error: timeout"
        );

        let network_err = Error::NetworkError("offline".to_string());
        assert_eq!(network_err.to_string(), "[SK8S-014] Network error: offline");
        assert_eq!(
            network_err.status_message(),
            "[SK8S-014] Network error: offline"
        );

        let io_err = Error::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        assert_eq!(io_err.to_string(), "[SK8S-016] I/O error: file not found");
        assert_eq!(
            io_err.status_message(),
            "[SK8S-016] I/O error: file not found"
        );

        let maintenance_err = Error::MaintenanceError("db locked".to_string());
        assert_eq!(
            maintenance_err.to_string(),
            "[SK8S-017] Database maintenance error: db locked"
        );
        assert_eq!(
            maintenance_err.status_message(),
            "[SK8S-017] Database maintenance error: db locked"
        );
    }
}
