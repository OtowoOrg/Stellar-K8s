//! HTTP handlers for the Dashboard API

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use k8s_openapi::api::core::v1::Pod;
use kube::{api::Api, api::LogParams, api::Patch, api::PatchParams, ResourceExt};
use tracing::{error, info, instrument};

use crate::controller::ControllerState;
use crate::crd::{NodeType, StellarNetwork, StellarNode};

use super::dashboard_dto::{
    ConditionDisplay, DashboardOverview, MetricsSummary, NetworkBreakdown, NodeAction,
    NodeActionRequest, NodeActionResponse, NodeConditionsResponse, NodeLogsResponse,
    NodeTypeBreakdown, OperatorLogsResponse,
};
use super::dto::ErrorResponse;

/// Dashboard overview endpoint
#[instrument(skip(state))]
pub async fn dashboard_overview(
    State(state): State<Arc<ControllerState>>,
) -> Result<Json<DashboardOverview>, (StatusCode, Json<ErrorResponse>)> {
    let api: Api<StellarNode> = Api::all(state.client.clone());

    match api.list(&Default::default()).await {
        Ok(nodes) => {
            let total_nodes = nodes.items.len();
            let mut healthy = 0;
            let mut syncing = 0;
            let mut unhealthy = 0;

            let mut validators = 0;
            let mut horizon = 0;
            let mut soroban = 0;

            let mut mainnet = 0;
            let mut testnet = 0;
            let mut futurenet = 0;
            let mut custom = 0;

            for node in &nodes.items {
                // Count by health status
                if let Some(status) = &node.status {
                    let conditions = &status.conditions;
                    if !conditions.is_empty() {
                        let ready = conditions
                            .iter()
                            .find(|c| c.type_ == "Ready")
                            .map(|c| c.status == "True")
                            .unwrap_or(false);
                        let synced = conditions
                            .iter()
                            .find(|c| c.type_ == "Synced")
                            .map(|c| c.status == "True")
                            .unwrap_or(false);

                        if ready && synced {
                            healthy += 1;
                        } else if ready {
                            syncing += 1;
                        } else {
                            unhealthy += 1;
                        }
                    } else {
                        unhealthy += 1;
                    }
                } else {
                    unhealthy += 1;
                }

                // Count by type
                match node.spec.node_type {
                    NodeType::Validator => validators += 1,
                    NodeType::Horizon => horizon += 1,
                    NodeType::SorobanRpc => soroban += 1,
                }

                // Count by network
                match &node.spec.network {
                    StellarNetwork::Mainnet => mainnet += 1,
                    StellarNetwork::Testnet => testnet += 1,
                    StellarNetwork::Futurenet => futurenet += 1,
                    StellarNetwork::Custom(_) => custom += 1,
                }
            }

            Ok(Json(DashboardOverview {
                total_nodes,
                healthy_nodes: healthy,
                syncing_nodes: syncing,
                unhealthy_nodes: unhealthy,
                nodes_by_type: NodeTypeBreakdown {
                    validators,
                    horizon,
                    soroban,
                },
                nodes_by_network: NetworkBreakdown {
                    mainnet,
                    testnet,
                    futurenet,
                    custom,
                },
            }))
        }
        Err(e) => {
            error!("Failed to list nodes for dashboard: {:?}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "dashboard_failed",
                    &format!("Failed to fetch dashboard data: {e}"),
                )),
            ))
        }
    }
}

/// Get node conditions formatted for UI
#[instrument(skip(state), fields(node_name = %name, namespace = %namespace, reconcile_id = "-"))]
pub async fn get_node_conditions(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<NodeConditionsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let api: Api<StellarNode> = Api::namespaced(state.client.clone(), &namespace);

    match api.get(&name).await {
        Ok(node) => {
            let conditions = node
                .status
                .as_ref()
                .map(|s| s.conditions.iter().map(ConditionDisplay::from).collect())
                .unwrap_or_default();

            Ok(Json(NodeConditionsResponse {
                namespace: namespace.clone(),
                name: name.clone(),
                conditions,
            }))
        }
        Err(kube::Error::Api(e)) if e.code == 404 => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                &format!("Node {namespace}/{name} not found"),
            )),
        )),
        Err(e) => {
            error!("Failed to get node conditions: {:?}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("get_failed", &e.to_string())),
            ))
        }
    }
}

/// Get node logs
#[instrument(skip(state), fields(node_name = %name, namespace = %namespace, reconcile_id = "-"))]
pub async fn get_node_logs(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<NodeLogsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let pod_api: Api<Pod> = Api::namespaced(state.client.clone(), &namespace);

    // Find pods for this node
    let label_selector = format!("app.kubernetes.io/instance={name}");
    let lp = kube::api::ListParams::default().labels(&label_selector);

    match pod_api.list(&lp).await {
        Ok(pods) => {
            if pods.items.is_empty() {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse::new(
                        "no_pods",
                        &format!("No pods found for node {namespace}/{name}"),
                    )),
                ));
            }

            // Get logs from the first pod
            let pod = &pods.items[0];
            let pod_name = pod.name_any();

            let log_params = LogParams {
                tail_lines: Some(500),
                ..Default::default()
            };

            match pod_api.logs(&pod_name, &log_params).await {
                Ok(logs) => Ok(Json(NodeLogsResponse {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    pod_name,
                    logs,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                })),
                Err(e) => {
                    error!("Failed to get logs for pod {}: {:?}", pod_name, e);
                    Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::new("logs_failed", &e.to_string())),
                    ))
                }
            }
        }
        Err(e) => {
            error!(
                "Failed to list pods for node {}/{}: {:?}",
                namespace, name, e
            );
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("list_pods_failed", &e.to_string())),
            ))
        }
    }
}

/// Execute node action (restart, snapshot, suspend, resume)
#[instrument(skip(state), fields(node_name = %name, namespace = %namespace, reconcile_id = "-"))]
pub async fn execute_node_action(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, name)): Path<(String, String)>,
    Json(request): Json<NodeActionRequest>,
) -> Result<Json<NodeActionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let api: Api<StellarNode> = Api::namespaced(state.client.clone(), &namespace);

    // Verify node exists
    let node = match api.get(&name).await {
        Ok(n) => n,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse::new(
                    "not_found",
                    &format!("Node {namespace}/{name} not found"),
                )),
            ))
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("get_failed", &e.to_string())),
            ))
        }
    };

    info!(
        "Executing action {:?} on node {}/{}",
        request.action, namespace, name
    );

    let result = match request.action {
        NodeAction::Restart => restart_node(&state, &api, &node).await,
        NodeAction::Snapshot => trigger_snapshot(&api, &node).await,
        NodeAction::Suspend => suspend_node(&api, &node).await,
        NodeAction::Resume => resume_node(&api, &node).await,
        NodeAction::MaintenanceMode => toggle_maintenance_mode(&api, &node).await,
        NodeAction::Prune => trigger_prune(&api, &node).await,
    };

    match result {
        Ok(message) => Ok(Json(NodeActionResponse {
            success: true,
            message,
            action: request.action,
        })),
        Err(e) => {
            error!("Action failed: {:?}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("action_failed", &e.to_string())),
            ))
        }
    }
}

/// Restart a node by deleting its pods
async fn restart_node(
    state: &ControllerState,
    _api: &Api<StellarNode>,
    node: &StellarNode,
) -> Result<String, kube::Error> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    let pod_api: Api<Pod> = Api::namespaced(state.client.clone(), &namespace);
    let label_selector = format!("app.kubernetes.io/instance={name}");
    let lp = kube::api::ListParams::default().labels(&label_selector);

    let pods = pod_api.list(&lp).await?;
    let pod_count = pods.items.len();

    for pod in pods.items {
        let pod_name = pod.name_any();
        pod_api
            .delete(&pod_name, &kube::api::DeleteParams::default())
            .await?;
        info!("Deleted pod {} for restart", pod_name);
    }

    Ok(format!("Restarted {pod_count} pod(s) for node {name}"))
}

/// Trigger a manual snapshot
async fn trigger_snapshot(
    api: &Api<StellarNode>,
    node: &StellarNode,
) -> Result<String, kube::Error> {
    let name = node.name_any();

    let patch = serde_json::json!({
        "metadata": {
            "annotations": {
                "stellar.org/request-snapshot": "true"
            }
        }
    });

    api.patch(
        &name,
        &PatchParams::apply("stellar-dashboard"),
        &Patch::Merge(&patch),
    )
    .await?;

    Ok(format!("Snapshot requested for node {name}"))
}

/// Suspend a node
async fn suspend_node(api: &Api<StellarNode>, node: &StellarNode) -> Result<String, kube::Error> {
    let name = node.name_any();

    let patch = serde_json::json!({
        "spec": {
            "suspended": true
        }
    });

    api.patch(
        &name,
        &PatchParams::apply("stellar-dashboard"),
        &Patch::Merge(&patch),
    )
    .await?;

    Ok(format!("Node {name} suspended"))
}

/// Resume a node
async fn resume_node(api: &Api<StellarNode>, node: &StellarNode) -> Result<String, kube::Error> {
    let name = node.name_any();

    let patch = serde_json::json!({
        "spec": {
            "suspended": false
        }
    });

    api.patch(
        &name,
        &PatchParams::apply("stellar-dashboard"),
        &Patch::Merge(&patch),
    )
    .await?;

    Ok(format!("Node {name} resumed"))
}

/// Toggle maintenance mode on a node
async fn toggle_maintenance_mode(
    api: &Api<StellarNode>,
    node: &StellarNode,
) -> Result<String, kube::Error> {
    let name = node.name_any();
    let current = node.spec.maintenance_mode;
    let next = !current;

    let patch = serde_json::json!({
        "spec": {
            "maintenanceMode": next
        }
    });

    api.patch(
        &name,
        &PatchParams::apply("stellar-dashboard"),
        &Patch::Merge(&patch),
    )
    .await?;

    let state = if next { "enabled" } else { "disabled" };
    Ok(format!("Maintenance mode {state} for node {name}"))
}

/// Trigger archive pruning via annotation
async fn trigger_prune(api: &Api<StellarNode>, node: &StellarNode) -> Result<String, kube::Error> {
    let name = node.name_any();

    let patch = serde_json::json!({
        "metadata": {
            "annotations": {
                "stellar.org/request-prune": chrono::Utc::now().to_rfc3339()
            }
        }
    });

    api.patch(
        &name,
        &PatchParams::apply("stellar-dashboard"),
        &Patch::Merge(&patch),
    )
    .await?;

    Ok(format!("Archive prune requested for node {name}"))
}

/// Get metrics summary for a node
#[instrument(skip(state), fields(node_name = %name, namespace = %namespace, reconcile_id = "-"))]
pub async fn get_node_metrics(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<MetricsSummary>, (StatusCode, Json<ErrorResponse>)> {
    let api: Api<StellarNode> = Api::namespaced(state.client.clone(), &namespace);

    match api.get(&name).await {
        Ok(node) => {
            let status = node.status.as_ref();

            Ok(Json(MetricsSummary {
                namespace: namespace.clone(),
                name: name.clone(),
                ledger_sequence: status.and_then(|s| s.ledger_sequence),
                ready_replicas: status.map(|s| s.ready_replicas).unwrap_or(0),
                replicas: status.map(|s| s.replicas).unwrap_or(0),
                quorum_fragility: status.and_then(|s| s.quorum_fragility),
            }))
        }
        Err(kube::Error::Api(e)) if e.code == 404 => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                &format!("Node {namespace}/{name} not found"),
            )),
        )),
        Err(e) => {
            error!("Failed to get node metrics: {:?}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("get_failed", &e.to_string())),
            ))
        }
    }
}

/// Get operator pod logs (the operator itself, identified by HOSTNAME env var)
#[instrument(skip(state), fields(node_name = "-", namespace = %state.operator_namespace, reconcile_id = "-"))]
pub async fn get_operator_logs(
    State(state): State<Arc<ControllerState>>,
) -> Result<Json<OperatorLogsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let namespace = &state.operator_namespace;
    let pod_api: Api<Pod> = Api::namespaced(state.client.clone(), namespace);

    // Identify the operator pod by the well-known label set by the Helm chart
    let lp = kube::api::ListParams::default().labels("app.kubernetes.io/name=stellar-operator");

    match pod_api.list(&lp).await {
        Ok(pods) if !pods.items.is_empty() => {
            let pod = &pods.items[0];
            let pod_name = pod.name_any();

            let log_params = LogParams {
                tail_lines: Some(200),
                ..Default::default()
            };

            match pod_api.logs(&pod_name, &log_params).await {
                Ok(raw) => {
                    let lines: Vec<String> = raw.lines().map(str::to_owned).collect();
                    Ok(Json(OperatorLogsResponse {
                        logs: lines,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    }))
                }
                Err(e) => {
                    error!("Failed to fetch operator logs from pod {pod_name}: {e:?}");
                    Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::new("logs_failed", &e.to_string())),
                    ))
                }
            }
        }
        Ok(_) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "no_operator_pod",
                "No operator pod found with label app.kubernetes.io/name=stellar-operator",
            )),
        )),
        Err(e) => {
            error!("Failed to list operator pods: {e:?}");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("list_pods_failed", &e.to_string())),
            ))
        }
    }
}
