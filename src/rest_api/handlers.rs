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
    ErrorResponse, HealthResponse, NodeDetailResponse, NodeListResponse, NodeSummary,
};

/// Health check endpoint
#[instrument]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// List all StellarNodes
#[instrument(skip(state))]
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
#[instrument(skip(state), fields(name = %name, namespace = %namespace))]
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
                &format!("Node {}/{} not found", namespace, name),
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
