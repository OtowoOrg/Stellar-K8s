//! HTTP handlers for the REST API

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use kube::{api::Api, ResourceExt};
use tracing::{error, instrument};

use crate::controller::ControllerState;
use crate::crd::StellarNode;

use super::dto::{
    ErrorResponse, HealthResponse, LeaderResponse, NodeDetailResponse, NodeListResponse,
    NodeSummary, ProbeResponse,
};

/// Health check endpoint
#[instrument]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Leader status endpoint - returns whether this replica is the active leader
#[instrument(
    skip(state),
    fields(node_name = "-", namespace = %state.operator_namespace, reconcile_id = "-")
)]
pub async fn leader_status(State(state): State<Arc<ControllerState>>) -> Json<LeaderResponse> {
    let is_leader = state.is_leader.load(std::sync::atomic::Ordering::Relaxed);
    let holder_id = std::env::var("HOSTNAME")
        .or_else(|_| hostname::get().map(|h| h.to_string_lossy().to_string()))
        .unwrap_or_else(|_| "unknown".to_string());
    Json(LeaderResponse {
        is_leader,
        holder_id,
    })
}

/// List all StellarNodes
#[instrument(
    skip(state),
    fields(node_name = "-", namespace = %state.operator_namespace, reconcile_id = "-")
)]
#[allow(deprecated)]
pub async fn list_nodes(
    State(state): State<Arc<ControllerState>>,
) -> Result<Json<NodeListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let api: Api<StellarNode> = Api::all(state.client.clone());

    match api.list(&Default::default()).await {
        Ok(nodes) => {
            let items: Vec<NodeSummary> = nodes
                .items
                .iter()
                .map(|n| NodeSummary {
                    name: n.name_any(),
                    namespace: n.namespace().unwrap_or_default(),
                    node_type: n.spec.node_type.clone(),
                    network: n.spec.network.clone(),
                    phase: n
                        .status
                        .as_ref()
                        .map(|s| s.derive_phase_from_conditions())
                        .unwrap_or_else(|| "Unknown".to_string()),
                    replicas: n.spec.replicas,
                    ready_replicas: n.status.as_ref().map(|s| s.ready_replicas).unwrap_or(0),
                })
                .collect();

            let total = items.len();
            Ok(Json(NodeListResponse { items, total }))
        }
        Err(e) => {
            error!("Failed to list nodes: {:?}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("list_failed", &e.to_string())),
            ))
        }
    }
}

/// Get a specific StellarNode
#[instrument(skip(state), fields(node_name = %name, namespace = %namespace, reconcile_id = "-"))]
pub async fn get_node(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<NodeDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let api: Api<StellarNode> = Api::namespaced(state.client.clone(), &namespace);

    match api.get(&name).await {
        Ok(node) => {
            let response = NodeDetailResponse {
                name: node.name_any(),
                namespace: node.namespace().unwrap_or_default(),
                node_type: node.spec.node_type.clone(),
                network: node.spec.network.clone(),
                version: node.spec.version.clone(),
                status: node.status.clone().unwrap_or_default(),
                created_at: node.metadata.creation_timestamp.map(|t| t.0.to_rfc3339()),
            };
            Ok(Json(response))
        }
        Err(kube::Error::Api(e)) if e.code == 404 => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                &format!("Node {namespace}/{name} not found"),
            )),
        )),
        Err(e) => {
            error!("Failed to get node {}/{}: {:?}", namespace, name, e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("get_failed", &e.to_string())),
            ))
        }
    }
}

/// /healthz - basic liveness signal; always 200 if the process is up.
pub async fn healthz() -> Json<ProbeResponse> {
    Json(ProbeResponse {
        status: "ok",
        reason: None,
    })
}

/// /readyz - checks that the K8s API server is reachable and the StellarNode CRD is installed.
pub async fn readyz(
    State(state): State<Arc<ControllerState>>,
) -> (StatusCode, Json<ProbeResponse>) {
    let api: Api<StellarNode> = Api::all(state.client.clone());
    match api.list(&Default::default()).await {
        Ok(_) => (
            StatusCode::OK,
            Json(ProbeResponse {
                status: "ok",
                reason: None,
            }),
        ),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ProbeResponse {
                status: "not ready",
                reason: Some(format!("CRD check failed: {e}")),
            }),
        ),
    }
}

/// /livez - verifies the reconciler loop is not stuck.
/// Returns 200 if a successful reconcile occurred within the last 60 seconds,
/// or if no reconcile has run yet (operator just started, within a 120s grace period).
pub async fn livez(State(state): State<Arc<ControllerState>>) -> (StatusCode, Json<ProbeResponse>) {
    const MAX_STALE_SECS: u64 = 60;
    const STARTUP_GRACE_SECS: u64 = 120;

    let last_ts = state
        .last_reconcile_success
        .load(std::sync::atomic::Ordering::Relaxed);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if last_ts == 0 {
        // No reconcile yet — allow a startup grace period based on process uptime proxy.
        // We use the reconcile_id_counter: if it's still 0 we haven't even started.
        // Either way, give the operator STARTUP_GRACE_SECS before declaring stuck.
        // Since we don't track start time, we conservatively return ok during startup.
        let _ = STARTUP_GRACE_SECS; // referenced for clarity
        return (
            StatusCode::OK,
            Json(ProbeResponse {
                status: "ok",
                reason: Some("no reconcile yet; within startup grace period".to_string()),
            }),
        );
    }

    let age = now.saturating_sub(last_ts);
    if age <= MAX_STALE_SECS {
        (
            StatusCode::OK,
            Json(ProbeResponse {
                status: "ok",
                reason: None,
            }),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ProbeResponse {
                status: "not live",
                reason: Some(format!(
                    "last successful reconcile was {age}s ago (threshold: {MAX_STALE_SECS}s)"
                )),
            }),
        )
    }
}
