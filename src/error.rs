//! Central error types for the Stellar-K8s operator
//!
//! Uses `thiserror` for ergonomic, type-safe error handling with
//! automatic `Display` and `Error` trait implementations.

use thiserror::Error;

/// Central error type for the Stellar-K8s operator
#[derive(Error, Debug)]
pub enum Error {
    /// Kubernetes API error from kube-rs
    #[error("Kubernetes API error: {0}")]
    KubeError(#[from] kube::Error),

    /// JSON serialization/deserialization error
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Finalizer-related error during cleanup
    #[error("Finalizer error: {0}")]
    FinalizerError(String),

    /// Configuration validation error
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Node spec validation error
    #[error("Node validation error: {0}")]
    ValidationError(String),

    /// Resource not found in the cluster
    #[error("Resource not found: {kind}/{name} in namespace {namespace}")]
    NotFound {
        kind: String,
        name: String,
        namespace: String,
    },

    /// Invalid node type specified
    #[error("Invalid node type: {0}")]
    InvalidNodeType(String),

    /// Missing required field in spec
    #[error("Missing required field: {field} for node type {node_type}")]
    MissingRequiredField { field: String, node_type: String },

    /// History archive health check error
    #[error("Archive health check failed: {0}")]
    ArchiveHealthCheckError(String),

    /// HTTP request error (from reqwest)
    #[error("HTTP request error: {0}")]
    HttpError(#[from] reqwest::Error),

    /// Remediation action failed
    #[error("Remediation failed: {0}")]
    RemediationError(String),

    /// Wasm plugin error
    #[error("Plugin error: {0}")]
    PluginError(String),

    /// Webhook server error
    #[error("Webhook error: {0}")]
    WebhookError(String),

    /// Network connectivity error
    #[error("Network error: {0}")]
    NetworkError(String),

    /// Certificate generation error
    #[error("Certificate error: {0}")]
    CertificateError(#[from] rcgen::Error),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
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
            Error::KubeError(e) => format!("Kubernetes error: {}", e),
            Error::ValidationError(msg) => format!("Validation failed: {}", msg),
            Error::MissingRequiredField { field, node_type } => {
                format!("Missing {} for {} node", field, node_type)
            }
            Error::ArchiveHealthCheckError(msg) => {
                format!("Archive health check failed: {}", msg)
            }
            Error::HttpError(e) => format!("HTTP request failed: {}", e),
            Error::RemediationError(msg) => format!("Remediation failed: {}", msg),
            Error::PluginError(msg) => format!("Plugin error: {}", msg),
            Error::WebhookError(msg) => format!("Webhook error: {}", msg),
            _ => self.to_string(),
        }
    }
}

// Implement From for kube::runtime::finalizer::Error to enable ? operator
impl From<kube::runtime::finalizer::Error<Error>> for Error {
    fn from(e: kube::runtime::finalizer::Error<Error>) -> Self {
        Error::FinalizerError(e.to_string())
    }
}
