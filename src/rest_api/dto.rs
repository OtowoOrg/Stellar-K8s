//! Data Transfer Objects for the REST API
//!
//! These types are used for API requests and responses.

use serde::{Deserialize, Serialize};

use crate::crd::{NodeType, StellarNetwork, StellarNodeStatus};

/// Response for listing nodes
#[derive(Debug, Serialize)]
pub struct NodeListResponse {
    pub items: Vec<NodeSummary>,
    pub total: usize,
}

/// Summary of a StellarNode for list views
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeSummary {
    pub name: String,
    pub namespace: String,
    pub node_type: NodeType,
    pub network: StellarNetwork,
    pub phase: String,
    pub replicas: i32,
    pub ready_replicas: i32,
}

/// Response for a single node
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDetailResponse {
    pub name: String,
    pub namespace: String,
    pub node_type: NodeType,
    pub network: StellarNetwork,
    pub version: String,
    pub status: StellarNodeStatus,
    pub created_at: Option<String>,
}

/// Request to create a node (simplified)
/// Reserved for future API endpoints
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateNodeRequest {
    pub name: String,
    pub namespace: String,
    pub node_type: NodeType,
    pub network: StellarNetwork,
    pub version: String,
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// Leader status response
#[derive(Debug, Serialize)]
pub struct LeaderResponse {
    pub is_leader: bool,
    pub holder_id: String,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

impl ErrorResponse {
    pub fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
        }
    }
}

/// Generic probe response used by /healthz, /readyz, /livez
#[derive(Debug, Serialize)]
pub struct ProbeResponse {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Request to change log level
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLevelRequest {
    /// New log level (e.g., "debug", "info", "warn", "error", "trace")
    pub level: String,
    /// Optional duration in minutes for which this level should apply
    pub duration_minutes: Option<u64>,
}

/// Response for log level change
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLevelResponse {
    pub current_level: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub message: String,
}
