//! HTTP handlers for the Dashboard API

use std::sync::Arc;

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use k8s_openapi::api::core::v1::Pod;
use crate::crd::StellarNodeSpec;
use kube::{api::Api, api::LogParams, api::Patch, api::PatchParams, ResourceExt};
use tracing::{error, info, instrument};

use crate::controller::{AdminAction, AuditEntry, ControllerState};
use crate::crd::{NodeType, StellarNetwork, StellarNode, StellarNodeSpec};
use crate::rest_api::auth::RequestIdentity;

use super::dashboard_dto::{
    CapacityPlanningResponse, ConditionDisplay, ConfigDriftResponse, ConfigImpactResponse,
    DRStatusResponse, DashboardOverview, LogAnalyticsResponse, LogPatternDto, MetricsSummary,
    NetworkBreakdown, NodeAction, NodeActionRequest, NodeActionResponse, NodeConditionsResponse,
    NodeLogsResponse, NodeTypeBreakdown, OperatorLogsResponse, SecurityPostureResponse,
    WhatIfRequest,
};
use super::dto::ErrorResponse;

/// Get DR status for a node
#[instrument(skip(state))]
pub async fn get_dr_status(
    State(state): State<Arc<ControllerState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<DRStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    let api: Api<StellarNode> = Api::namespaced(state.client.clone(), &namespace);

    match api.get(&name).await {
        Ok(node) => {
            let dr_config = node.spec.dr_config.as_ref();
            let dr_status = node.status.as_ref().and_then(|s| s.dr_status.as_ref());

            Ok(Json(DRStatusResponse {
                namespace,
                name,
                dr_enabled: dr_config.map(|c| c.enabled).unwrap_or(false),
                current_role: dr_status
                    .and_then(|s| s.current_role.as_ref().map(|r| format!("{:?}", r))),
                failover_active: dr_status.map(|s| s.failover_active).unwrap_or(false),
                last_failover_time: dr_status.and_then(|s| s.last_failover_time.clone()),
                sync_lag: dr_status.and_then(|s| s.sync_lag),
                compliance_status: None, // Would fetch from DisasterRecoveryPolicy if needed
                last_drill_result: dr_status.and_then(|s| s.last_drill_result.clone()),
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
            error!("Failed to get DR status: {:?}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("get_failed", &e.to_string())),
            ))
        }
    }
}

/// Get log analytics summary
pub async fn log_analytics(
    State(state): State<Arc<ControllerState>>,
) -> Json<LogAnalyticsResponse> {
    let top_patterns = state.analytics_engine.get_top_patterns(10);

    let patterns = top_patterns
        .into_iter()
        .map(|p| LogPatternDto {
            template: p.message_template,
            count: p.count,
            last_seen: format!("{:?}", p.last_seen), // Simplified for now
        })
        .collect();

    Json(LogAnalyticsResponse {
        top_patterns: patterns,
    })
}

/// Analyze configuration impact
pub async fn analyze_config_impact(
    State(state): State<Arc<ControllerState>>,
    Json(new_spec): Json<StellarNodeSpec>,
) -> Json<ConfigImpactResponse> {
    // For impact analysis, we'd ideally compare against the current spec.
    // Here we use a dummy old spec for demonstration.
    let old_spec = new_spec.clone(); // In reality, fetch from K8s

    let impact = crate::config_mgmt::impact::ImpactAnalyzer::analyze(&old_spec, &new_spec);
    let validation_errors = crate::config_mgmt::validation::Validator::validate(&new_spec);

    Json(ConfigImpactResponse {
        impact,
        validation_errors,
    })
}

/// Get security posture summary
pub async fn security_posture(
    State(_state): State<Arc<ControllerState>>,
) -> Json<SecurityPostureResponse> {
    // Mock posture for demonstration
    let posture = crate::security::SecurityPosture {
        overall_score: 0.95,
        findings: vec![],
        compliance_status: true,
    };

    Json(SecurityPostureResponse { posture })
}

/// Get capacity planning summary
pub async fn capacity_planning(
    State(_state): State<Arc<ControllerState>>,
) -> Json<CapacityPlanningResponse> {
    // Mock data for demonstration
    let now = chrono::Utc::now();
    let forecasts = vec![crate::capacity_planning::GrowthForecast {
        resource_type: "CPU".to_string(),
        forecast_points: vec![(now, 1.0), (now + chrono::Duration::days(30), 1.5)],
        model_used: "Linear".to_string(),
        growth_rate_pct: 50.0,
    }];

    let engine = crate::capacity_planning::recommendation::RecommendationEngine::new(1.2);
    let recommendations = engine.generate_recommendations(&forecasts);

    Json(CapacityPlanningResponse {
        recommendations,
        forecasts,
        bottlenecks: vec![
            "Storage growth exceeding 20% per month in 'stellar-mainnet'".to_string(),
        ],
    })
}

/// Run a what-if scenario analysis
pub async fn run_what_if(
    State(_state): State<Arc<ControllerState>>,
    Json(req): Json<WhatIfRequest>,
) -> Json<crate::capacity_planning::WhatIfResult> {
    let analyzer = crate::capacity_planning::analysis::ScenarioAnalyzer;
    Json(analyzer.analyze_scenario(&req.scenario_name, req.scale_factor))
}

/// Get real-time traffic shaping dashboard metrics.
pub async fn traffic_dashboard(
    State(_state): State<Arc<ControllerState>>,
) -> Json<crate::controller::traffic::TrafficDashboardSnapshot> {
    Json(crate::controller::traffic::get_traffic_dashboard_snapshot())
}

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

/// Get metrics summary for all nodes.
#[instrument(skip(state))]
pub async fn dashboard_metrics(
    State(state): State<Arc<ControllerState>>,
) -> Result<Json<Vec<MetricsSummary>>, (StatusCode, Json<ErrorResponse>)> {
    let api: Api<StellarNode> = Api::all(state.client.clone());

    match api.list(&Default::default()).await {
        Ok(nodes) => {
            let mut summaries = Vec::with_capacity(nodes.items.len());
            for node in nodes.items {
                let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
                let name = node.name_any();
                let status = node.status.as_ref();
                summaries.push(MetricsSummary {
                    namespace,
                    name,
                    ledger_sequence: status.and_then(|s| s.ledger_sequence),
                    ready_replicas: status.map(|s| s.ready_replicas).unwrap_or(0),
                    replicas: status.map(|s| s.replicas).unwrap_or(0),
                    quorum_fragility: status.and_then(|s| s.quorum_fragility),
                });
            }
            Ok(Json(summaries))
        }
        Err(e) => {
            error!("Failed to list nodes for dashboard metrics: {e:?}");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "dashboard_metrics_failed",
                    &format!("Failed to fetch dashboard metrics: {e}"),
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
    Extension(identity): Extension<RequestIdentity>,
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
        Ok(message) => {
            let action = match request.action {
                NodeAction::Restart => AdminAction::Other("node_restart".to_string()),
                NodeAction::Snapshot => AdminAction::ForensicSnapshot,
                NodeAction::Suspend => AdminAction::NodeSuspend,
                NodeAction::Resume => AdminAction::NodeResume,
                NodeAction::MaintenanceMode => AdminAction::TriggerMaintenance,
                NodeAction::Prune => AdminAction::Other("prune".to_string()),
            };

            state
                .audit_recorder
                .record(
                    AuditEntry::new(
                        action,
                        identity.subject.clone(),
                        &name,
                        namespace.clone(),
                        Some(&format!("Action {:?}", request.action)),
                    )
                    .with_metadata(serde_json::json!({
                        "authType": identity.auth_type,
                        "groups": identity.groups,
                    })),
                )
                .await;

            Ok(Json(NodeActionResponse {
                success: true,
                message,
                action: request.action,
            }))
        }
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

/// Get dashboard metrics summary
#[instrument(skip(state))]
pub async fn dashboard_metrics(
    State(state): State<Arc<ControllerState>>,
) -> Result<Json<MetricsSummary>, (StatusCode, Json<ErrorResponse>)> {
    // Return aggregated metrics across all nodes
    let api: Api<StellarNode> = Api::all(state.client.clone());

    match api.list(&Default::default()).await {
        Ok(nodes) => {
            let mut total_replicas = 0;
            let mut total_ready_replicas = 0;
            let mut latest_ledger = None;
            let mut avg_quorum_fragility = 0.0;
            let mut fragility_count = 0;

            for node in &nodes.items {
                if let Some(status) = &node.status {
                    total_replicas += status.replicas;
                    total_ready_replicas += status.ready_replicas;
                    
                    if let Some(ledger) = status.ledger_sequence {
                        latest_ledger = Some(latest_ledger.map_or(ledger, |l: u64| l.max(ledger)));
                    }
                    
                    if let Some(fragility) = status.quorum_fragility {
                        avg_quorum_fragility += fragility;
                        fragility_count += 1;
                    }
                }
            }

            if fragility_count > 0 {
                avg_quorum_fragility /= fragility_count as f64;
            }

            Ok(Json(MetricsSummary {
                namespace: "all".to_string(),
                name: "cluster".to_string(),
                ledger_sequence: latest_ledger,
                ready_replicas: total_ready_replicas,
                replicas: total_replicas,
                quorum_fragility: if fragility_count > 0 { Some(avg_quorum_fragility) } else { None },
            }))
        }
        Err(e) => {
            error!("Failed to get dashboard metrics: {:?}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("metrics_failed", &e.to_string())),
            ))
        }
    }
}
