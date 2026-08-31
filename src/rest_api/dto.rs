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

/// Standardised API Error Codes for REST Endpoints (issue #1282)
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
        }
    }

    /// HTTP status for each error code — ensures consistent mapping across all REST endpoints
    pub fn http_status(&self) -> axum::http::StatusCode {
        match self {
            Self::ErrNotFound => axum::http::StatusCode::NOT_FOUND,
            Self::ErrBadRequest => axum::http::StatusCode::BAD_REQUEST,
            Self::ErrUnauthorized => axum::http::StatusCode::UNAUTHORIZED,
            Self::ErrForbidden => axum::http::StatusCode::FORBIDDEN,
            Self::ErrInternalServerError => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Self::ErrServiceUnavailable => axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Self::ErrPartialDegradation => axum::http::StatusCode::MULTI_STATUS,
            Self::ErrReconcileStalled => axum::http::StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

/// Structured error response for all REST API endpoints
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
            error_code: "ERR_INTERNAL_SERVER_ERROR".to_string(),
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
