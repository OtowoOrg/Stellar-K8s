//! Observability Dashboard with Drill-Down Capabilities
//!
//! Provides a comprehensive dashboard for visualizing observability data,
//! anomalies, alerts, and incident timelines with drill-down capabilities.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use super::observability_pipeline::ObservabilityPipeline;

/// Dashboard state
pub struct DashboardState {
    pipeline: Arc<ObservabilityPipeline>,
}

impl DashboardState {
    pub fn new(pipeline: Arc<ObservabilityPipeline>) -> Self {
        Self { pipeline }
    }
}

/// Query parameters for filtering
#[derive(Debug, Deserialize)]
pub struct FilterParams {
    pub limit: Option<usize>,
    pub severity: Option<String>,
    pub category: Option<String>,
    pub namespace: Option<String>,
    pub name: Option<String>,
}

/// Dashboard summary statistics
#[derive(Debug, Serialize)]
pub struct DashboardSummary {
    pub total_events: usize,
    pub total_anomalies: usize,
    pub active_alerts: usize,
    pub resolved_alerts: usize,
    pub predictive_alerts: usize,
    pub open_incidents: usize,
    pub mttr_ms: Option<u64>,
    pub health_score: f64,
}

/// Create the dashboard router
pub fn create_dashboard_router(pipeline: Arc<ObservabilityPipeline>) -> Router {
    let state = Arc::new(DashboardState::new(pipeline));

    Router::new()
        .route("/", get(dashboard_home))
        .route("/api/summary", get(get_summary))
        .route("/api/anomalies", get(get_anomalies))
        .route("/api/alerts", get(get_alerts))
        .route("/api/alerts/:id/acknowledge", post(acknowledge_alert))
        .route("/api/alerts/:id/resolve", post(resolve_alert))
        .route("/api/predictive", get(get_predictive_alerts))
        .route("/api/correlations", get(get_correlations))
        .route("/api/incidents", get(get_incidents))
        .route("/api/incidents/:id", get(get_incident_detail))
        .route("/api/incidents/:id/rca", get(get_root_cause_analysis))
        .with_state(state)
}

/// Dashboard home page
async fn dashboard_home() -> Html<String> {
    let html = r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Stellar-K8s Observability Dashboard</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: #0f172a;
            color: #e2e8f0;
            padding: 20px;
        }
        .header {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            padding: 30px;
            border-radius: 12px;
            margin-bottom: 30px;
            box-shadow: 0 10px 30px rgba(0,0,0,0.3);
        }
        .header h1 {
            font-size: 2.5em;
            margin-bottom: 10px;
        }
        .header p {
            opacity: 0.9;
            font-size: 1.1em;
        }
        .stats-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 20px;
            margin-bottom: 30px;
        }
        .stat-card {
            background: #1e293b;
            padding: 25px;
            border-radius: 12px;
            border-left: 4px solid #667eea;
            box-shadow: 0 4px 6px rgba(0,0,0,0.1);
            transition: transform 0.2s;
        }
        .stat-card:hover {
            transform: translateY(-5px);
        }
        .stat-card h3 {
            color: #94a3b8;
            font-size: 0.9em;
            text-transform: uppercase;
            letter-spacing: 1px;
            margin-bottom: 10px;
        }
        .stat-card .value {
            font-size: 2.5em;
            font-weight: bold;
            color: #667eea;
        }
        .section {
            background: #1e293b;
            padding: 25px;
            border-radius: 12px;
            margin-bottom: 20px;
            box-shadow: 0 4px 6px rgba(0,0,0,0.1);
        }
        .section h2 {
            margin-bottom: 20px;
            color: #667eea;
            border-bottom: 2px solid #334155;
            padding-bottom: 10px;
        }
        .alert-item {
            background: #334155;
            padding: 15px;
            border-radius: 8px;
            margin-bottom: 10px;
            border-left: 4px solid #ef4444;
        }
        .alert-item.warning { border-left-color: #f59e0b; }
        .alert-item.info { border-left-color: #3b82f6; }
        .alert-item h4 {
            margin-bottom: 5px;
        }
        .alert-item p {
            color: #94a3b8;
            font-size: 0.9em;
        }
        .btn {
            background: #667eea;
            color: white;
            border: none;
            padding: 10px 20px;
            border-radius: 6px;
            cursor: pointer;
            font-size: 0.9em;
            transition: background 0.2s;
        }
        .btn:hover {
            background: #5568d3;
        }
        .loading {
            text-align: center;
            padding: 40px;
            color: #94a3b8;
        }
    </style>
</head>
<body>
    <div class="header">
        <h1>🔭 Observability Dashboard</h1>
        <p>Advanced Monitoring with Anomaly Detection & Root Cause Analysis</p>
    </div>

    <div class="stats-grid" id="stats">
        <div class="loading">Loading statistics...</div>
    </div>

    <div class="section">
        <h2>🚨 Active Alerts</h2>
        <div id="alerts">
            <div class="loading">Loading alerts...</div>
        </div>
    </div>

    <div class="section">
        <h2>📊 Recent Anomalies</h2>
        <div id="anomalies">
            <div class="loading">Loading anomalies...</div>
        </div>
    </div>

    <div class="section">
        <h2>🔮 Predictive Alerts</h2>
        <div id="predictive">
            <div class="loading">Loading predictions...</div>
        </div>
    </div>

    <script>
        async function loadDashboard() {
            try {
                const summary = await fetch('/observability/api/summary').then(r => r.json());
                displayStats(summary);

                const alerts = await fetch('/observability/api/alerts?limit=10').then(r => r.json());
                displayAlerts(alerts);

                const anomalies = await fetch('/observability/api/anomalies?limit=10').then(r => r.json());
                displayAnomalies(anomalies);

                const predictive = await fetch('/observability/api/predictive?limit=5').then(r => r.json());
                displayPredictive(predictive);
            } catch (error) {
                console.error('Error loading dashboard:', error);
            }
        }

        function displayStats(summary) {
            const statsHtml = `
                <div class="stat-card">
                    <h3>Total Events</h3>
                    <div class="value">${summary.total_events}</div>
                </div>
                <div class="stat-card">
                    <h3>Anomalies Detected</h3>
                    <div class="value">${summary.total_anomalies}</div>
                </div>
                <div class="stat-card">
                    <h3>Active Alerts</h3>
                    <div class="value">${summary.active_alerts}</div>
                </div>
                <div class="stat-card">
                    <h3>Health Score</h3>
                    <div class="value">${summary.health_score.toFixed(1)}%</div>
                </div>
            `;
            document.getElementById('stats').innerHTML = statsHtml;
        }

        function displayAlerts(alerts) {
            if (alerts.length === 0) {
                document.getElementById('alerts').innerHTML = '<p>No active alerts</p>';
                return;
            }
            const alertsHtml = alerts.map(alert => `
                <div class="alert-item ${alert.severity.toLowerCase()}">
                    <h4>${alert.title}</h4>
                    <p>${alert.description}</p>
                    <p><small>First seen: ${new Date(alert.first_seen).toLocaleString()}</small></p>
                </div>
            `).join('');
            document.getElementById('alerts').innerHTML = alertsHtml;
        }

        function displayAnomalies(anomalies) {
            if (anomalies.length === 0) {
                document.getElementById('anomalies').innerHTML = '<p>No anomalies detected</p>';
                return;
            }
            const anomaliesHtml = anomalies.map(anomaly => `
                <div class="alert-item">
                    <h4>Anomaly: ${anomaly.anomaly_type}</h4>
                    <p>${anomaly.explanation}</p>
                    <p><small>Confidence: ${(anomaly.confidence * 100).toFixed(1)}% | Z-Score: ${anomaly.deviation.zscore.toFixed(2)}</small></p>
                </div>
            `).join('');
            document.getElementById('anomalies').innerHTML = anomaliesHtml;
        }

        function displayPredictive(predictions) {
            if (predictions.length === 0) {
                document.getElementById('predictive').innerHTML = '<p>No predictive alerts</p>';
                return;
            }
            const predictiveHtml = predictions.map(pred => `
                <div class="alert-item warning">
                    <h4>⚠️ ${pred.predicted_issue}</h4>
                    <p>Probability: ${(pred.probability * 100).toFixed(1)}%</p>
                    <p><small>Recommended: ${pred.recommended_actions.join(', ')}</small></p>
                </div>
            `).join('');
            document.getElementById('predictive').innerHTML = predictiveHtml;
        }

        // Load dashboard on page load
        loadDashboard();

        // Refresh every 30 seconds
        setInterval(loadDashboard, 30000);
    </script>
</body>
</html>
    "#;

    Html(html.to_string())
}

/// Get dashboard summary
async fn get_summary(
    State(state): State<Arc<DashboardState>>,
) -> Result<Json<DashboardSummary>, StatusCode> {
    let anomalies = state.pipeline.get_anomalies(0).await;
    let alerts = state.pipeline.get_alerts(0).await;
    let predictive = state.pipeline.get_predictive_alerts(0).await;
    let incidents = state.pipeline.get_incidents(0).await;

    let active_alerts = alerts.iter().filter(|a| !a.resolved).count();
    let resolved_alerts = alerts.iter().filter(|a| a.resolved).count();

    let mttr_ms = if !incidents.is_empty() {
        let total_mttr: u64 = incidents.iter().filter_map(|i| i.mttr_ms).sum();
        Some(total_mttr / incidents.len() as u64)
    } else {
        None
    };

    let health_score = if active_alerts == 0 && anomalies.is_empty() {
        100.0
    } else {
        (100.0 - (active_alerts as f64 * 5.0 + anomalies.len() as f64 * 2.0)).max(0.0)
    };

    let summary = DashboardSummary {
        total_events: 0, // Would need to track this
        total_anomalies: anomalies.len(),
        active_alerts,
        resolved_alerts,
        predictive_alerts: predictive.len(),
        open_incidents: incidents.iter().filter(|i| i.end_time.is_none()).count(),
        mttr_ms,
        health_score,
    };

    Ok(Json(summary))
}

/// Get anomalies
async fn get_anomalies(
    State(state): State<Arc<DashboardState>>,
    Query(params): Query<FilterParams>,
) -> Result<Json<Vec<super::observability_pipeline::AnomalyDetectionResult>>, StatusCode> {
    let limit = params.limit.unwrap_or(50);
    let anomalies = state.pipeline.get_anomalies(limit).await;
    Ok(Json(anomalies))
}

/// Get alerts
async fn get_alerts(
    State(state): State<Arc<DashboardState>>,
    Query(params): Query<FilterParams>,
) -> Result<Json<Vec<super::observability_pipeline::IntelligentAlert>>, StatusCode> {
    let limit = params.limit.unwrap_or(50);
    let alerts = state.pipeline.get_alerts(limit).await;
    Ok(Json(alerts))
}

/// Acknowledge an alert
async fn acknowledge_alert(
    State(state): State<Arc<DashboardState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    state
        .pipeline
        .acknowledge_alert(&id)
        .await
        .map(|_| StatusCode::OK)
        .map_err(|_| StatusCode::NOT_FOUND)
}

/// Resolve an alert
async fn resolve_alert(
    State(state): State<Arc<DashboardState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    state
        .pipeline
        .resolve_alert(&id)
        .await
        .map(|_| StatusCode::OK)
        .map_err(|_| StatusCode::NOT_FOUND)
}

/// Get predictive alerts
async fn get_predictive_alerts(
    State(state): State<Arc<DashboardState>>,
    Query(params): Query<FilterParams>,
) -> Result<Json<Vec<super::observability_pipeline::PredictiveAlert>>, StatusCode> {
    let limit = params.limit.unwrap_or(50);
    let alerts = state.pipeline.get_predictive_alerts(limit).await;
    Ok(Json(alerts))
}

/// Get correlations
async fn get_correlations(
    State(state): State<Arc<DashboardState>>,
    Query(params): Query<FilterParams>,
) -> Result<Json<Vec<super::observability_pipeline::CorrelationResult>>, StatusCode> {
    let limit = params.limit.unwrap_or(50);
    let correlations = state.pipeline.get_correlations(limit).await;
    Ok(Json(correlations))
}

/// Get incidents
async fn get_incidents(
    State(state): State<Arc<DashboardState>>,
    Query(params): Query<FilterParams>,
) -> Result<Json<Vec<super::observability_pipeline::IncidentTimeline>>, StatusCode> {
    let limit = params.limit.unwrap_or(50);
    let incidents = state.pipeline.get_incidents(limit).await;
    Ok(Json(incidents))
}

/// Get incident detail
async fn get_incident_detail(
    State(state): State<Arc<DashboardState>>,
    Path(id): Path<String>,
) -> Result<Json<super::observability_pipeline::IncidentTimeline>, StatusCode> {
    let timeline = state
        .pipeline
        .reconstruct_incident_timeline(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(timeline))
}

/// Get root cause analysis for an incident
async fn get_root_cause_analysis(
    State(state): State<Arc<DashboardState>>,
    Path(id): Path<String>,
) -> Result<Json<super::observability_pipeline::RootCauseAnalysis>, StatusCode> {
    let rca = state
        .pipeline
        .analyze_root_cause(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(rca))
}
