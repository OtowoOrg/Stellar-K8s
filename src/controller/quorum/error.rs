//! Error types for quorum analysis

use thiserror::Error;

#[derive(Error, Debug)]
pub enum QuorumAnalysisError {
    /// HTTP request to Stellar Core failed
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    /// Failed to parse SCP state response
    #[error("Failed to parse SCP state: {0}")]
    ParseError(String),

    /// Quorum graph is invalid (e.g., no intersection)
    #[error("Invalid quorum topology: {0}")]
    InvalidTopology(String),

    /// Analysis timeout exceeded
    #[error("Analysis timeout exceeded")]
    Timeout,

    /// Kubernetes API error when updating status
    #[error("Kubernetes API error: {0}")]
    KubeError(#[from] kube::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, QuorumAnalysisError>;
