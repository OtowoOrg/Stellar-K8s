//! REST API handlers for the background job monitoring dashboard.
//!
//! # Endpoints
//!
//! | Method | Path                  | Description                          |
//! |--------|-----------------------|--------------------------------------|
//! | GET    | `/api/v1/jobs`        | List all job records (newest first)  |
//! | GET    | `/api/v1/jobs/stats`  | Aggregate counts by state            |
//!
//! ## Query Parameters (`GET /api/v1/jobs`)
//!
//! | Parameter | Type   | Description                                     |
//! |-----------|--------|-------------------------------------------------|
//! | `state`   | string | Filter by job state (pending/running/succeeded/failed/cancelled) |
//! | `kind`    | string | Filter by job kind (reconcile/archive_check/...) |
//!
//! ## Example
//!
//! ```bash
//! # All running jobs
//! curl http://operator:9090/api/v1/jobs?state=running
//!
//! # Stats summary
//! curl http://operator:9090/api/v1/jobs/stats
//! ```

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::controller::background_jobs::JobRecord;
use crate::controller::ControllerState;

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Query parameters for `GET /api/v1/jobs`.
#[derive(Debug, Deserialize)]
pub struct JobQuery {
    /// Filter by lifecycle state name (e.g. `running`, `failed`).
    pub state: Option<String>,
    /// Filter by job kind name (e.g. `reconcile`, `archive_check`).
    pub kind: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Response envelope for `GET /api/v1/jobs`.
#[derive(Debug, Serialize)]
pub struct JobListResponse {
    pub items: Vec<JobRecord>,
    pub total: usize,
}

/// Response for `GET /api/v1/jobs/stats`.
#[derive(Debug, Serialize)]
pub struct JobStatsResponse {
    pub pending: usize,
    pub running: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub total_registered: usize,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/jobs`
///
/// Returns job records, newest first.  Supports optional `state` and `kind`
/// query filters.
pub async fn list_jobs(
    State(state): State<Arc<ControllerState>>,
    Query(q): Query<JobQuery>,
) -> Result<Json<JobListResponse>, (StatusCode, Json<crate::rest_api::dto::ErrorResponse>)> {
    let items = state
        .job_registry
        .list(q.state.as_deref(), q.kind.as_deref());
    let total = items.len();
    Ok(Json(JobListResponse { items, total }))
}

/// `GET /api/v1/jobs/stats`
///
/// Returns aggregate counts of jobs by lifecycle state.
pub async fn job_stats(
    State(state): State<Arc<ControllerState>>,
) -> Result<Json<JobStatsResponse>, (StatusCode, Json<crate::rest_api::dto::ErrorResponse>)> {
    let (pending, running, succeeded, failed, cancelled) = state.job_registry.state_counts();
    let total_registered = state.job_registry.count();
    Ok(Json(JobStatsResponse {
        pending,
        running,
        succeeded,
        failed,
        cancelled,
        total_registered,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::background_jobs::{JobKind, JobRegistry};

    #[test]
    fn test_job_query_defaults() {
        let q = JobQuery {
            state: None,
            kind: None,
        };
        assert!(q.state.is_none());
        assert!(q.kind.is_none());
    }

    #[test]
    fn test_job_list_response_serialization() {
        let registry = Arc::new(JobRegistry::new());
        let _h = registry.register("test-job", JobKind::Reconcile, Some("default".into()));
        let items = registry.list(None, None);
        let resp = JobListResponse {
            total: items.len(),
            items,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("test-job"));
        assert!(json.contains("reconcile"));
    }

    #[test]
    fn test_job_stats_response_serialization() {
        let resp = JobStatsResponse {
            pending: 1,
            running: 2,
            succeeded: 10,
            failed: 3,
            cancelled: 0,
            total_registered: 16,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"pending\":1"));
        assert!(json.contains("\"running\":2"));
        assert!(json.contains("\"total_registered\":16"));
    }
}
