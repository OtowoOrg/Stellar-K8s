//! REST API handlers for the admin activity audit log.
//!
//! # Endpoints
//!
//! | Method | Path                                         | Description                           |
//! |--------|----------------------------------------------|---------------------------------------|
//! | GET    | `/api/v1/audit-log`                          | List all audit entries (newest first) |
//! | GET    | `/api/v1/audit-log/search`                   | Filtered audit entry search           |
//!
//! ## Query Parameters (both endpoints)
//!
//! | Parameter   | Type   | Description                           |
//! |-------------|--------|---------------------------------------|
//! | `namespace` | string | Filter by Kubernetes namespace        |
//! | `resource`  | string | Substring match on the resource name  |
//! | `actor`     | string | Exact match on the actor identity     |
//! | `limit`     | u32    | Maximum number of entries to return   |
//!
//! ## Example
//!
//! ```bash
//! # List last 20 entries
//! curl http://operator:9090/api/v1/audit-log?limit=20
//!
//! # Filter by namespace and actor
//! curl "http://operator:9090/api/v1/audit-log/search?namespace=stellar&actor=admin"
//! ```

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::controller::audit_log::AuditEntry;
use crate::controller::ControllerState;

// ─── Query Parameters ────────────────────────────────────────────────────────

/// Query parameters accepted by the audit log endpoints.
#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    /// Filter entries to this Kubernetes namespace.
    pub namespace: Option<String>,
    /// Substring filter on the resource name.
    pub resource: Option<String>,
    /// Exact actor identity filter.
    pub actor: Option<String>,
    /// Maximum entries to return. Defaults to 100. Pass 0 for all.
    pub limit: Option<usize>,
}

// ─── Response Types ──────────────────────────────────────────────────────────

/// Response envelope for audit log queries.
#[derive(Debug, Serialize)]
pub struct AuditLogResponse {
    pub items: Vec<AuditEntry>,
    pub total: usize,
}

// ─── Handlers ────────────────────────────────────────────────────────────────

/// `GET /api/v1/audit-log`
///
/// Returns audit log entries, most recent first. Supports optional filtering
/// via query parameters.
pub async fn list_audit_log(
    State(state): State<Arc<ControllerState>>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<AuditLogResponse>, (StatusCode, Json<crate::rest_api::dto::ErrorResponse>)> {
    let limit = q.limit.unwrap_or(100);
    let items = state.audit_log.list(
        q.namespace.as_deref(),
        q.resource.as_deref(),
        q.actor.as_deref(),
        limit,
    );
    let total = items.len();
    Ok(Json(AuditLogResponse { items, total }))
}

/// `GET /api/v1/audit-log/search`
///
/// Alias for `list_audit_log` — exists to provide a cleaner "search" endpoint
/// that consumers can express intent with.
pub async fn search_audit_log(
    state: State<Arc<ControllerState>>,
    query: Query<AuditQuery>,
) -> Result<Json<AuditLogResponse>, (StatusCode, Json<crate::rest_api::dto::ErrorResponse>)> {
    list_audit_log(state, query).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::audit_log::{AdminAction, AuditEntry, AuditLog};

    #[test]
    fn test_audit_query_defaults() {
        let q = AuditQuery {
            namespace: None,
            resource: None,
            actor: None,
            limit: None,
        };
        assert!(q.namespace.is_none());
        assert_eq!(q.limit.unwrap_or(100), 100);
    }

    #[test]
    fn test_audit_log_response_serialization() {
        let entry = AuditEntry::new(
            AdminAction::SetLogLevel,
            "admin",
            "operator",
            "stellar-system",
            None,
        );
        let resp = AuditLogResponse {
            total: 1,
            items: vec![entry],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("set_log_level"));
        assert!(json.contains("stellar-system"));
    }
}
