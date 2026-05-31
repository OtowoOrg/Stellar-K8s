//! REST API handlers for resource optimization dashboard.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::controller::resource_optimization::{
    CapacitySimulator, ForecastEngine, ForecastModel, OptimizationController,
    ResourceOptimizationConfig, SimulationScenario, SlaMetrics, TimeSeriesPoint,
};
use crate::controller::ControllerState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationQuery {
    pub namespace: Option<String>,
    pub node_name: Option<String>,
    pub current_min_replicas: Option<i32>,
    pub max_replicas: Option<i32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationDashboardResponse {
    pub recommendations: Vec<crate::controller::resource_optimization::OptimizationRecommendation>,
    pub forecast_model: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationRequest {
    pub scenario_name: String,
    pub scale_factor: f64,
    pub current_replicas: i32,
    pub tps_per_replica: f64,
    pub history: Vec<HistoryPoint>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPoint {
    pub value: f64,
}

/// GET /api/v1/optimization/recommendations
pub async fn optimization_recommendations(
    State(state): State<Arc<ControllerState>>,
    axum::extract::Query(query): axum::extract::Query<OptimizationQuery>,
) -> Json<OptimizationDashboardResponse> {
    let namespace = query.namespace.unwrap_or_else(|| "default".to_string());
    let node_name = query.node_name.unwrap_or_else(|| "horizon".to_string());
    let current_min = query.current_min_replicas.unwrap_or(2);
    let max_replicas = query.max_replicas.unwrap_or(10);

    let config = ResourceOptimizationConfig {
        enabled: true,
        ..Default::default()
    };
    let mut controller = OptimizationController::new(config);

    // Seed with sample observations for dashboard preview
    for tps in [800.0, 900.0, 1000.0, 1100.0, 1200.0, 1300.0] {
        controller.record_observation(tps);
    }

    let metrics = SlaMetrics {
        p99_latency_ms: 120.0,
        availability_pct: 99.95,
        error_rate: 0.005,
        current_replicas: current_min,
        forecast_tps: 0.0,
    };

    let recommendations = controller
        .recommend(&namespace, &node_name, current_min, max_replicas, &metrics)
        .into_iter()
        .collect();

    let _ = state; // reserved for future cluster-wide aggregation

    Json(OptimizationDashboardResponse {
        recommendations,
        forecast_model: "ensemble".to_string(),
    })
}

/// POST /api/v1/optimization/simulate
pub async fn optimization_simulate(
    Json(req): Json<SimulationRequest>,
) -> Json<crate::controller::resource_optimization::SimulationResult> {
    let history: Vec<TimeSeriesPoint> = req
        .history
        .iter()
        .map(|h| TimeSeriesPoint {
            timestamp: chrono::Utc::now(),
            value: h.value,
        })
        .collect();

    let scenario = SimulationScenario {
        name: req.scenario_name,
        scale_factor: req.scale_factor,
        additional_replicas: 0,
        cost_per_replica_monthly: 150.0,
    };

    let sim = CapacitySimulator::new();
    Json(sim.simulate(
        &history,
        req.current_replicas,
        req.tps_per_replica,
        &scenario,
    ))
}

/// GET /api/v1/optimization/forecast
pub async fn optimization_forecast(
    axum::extract::Query(query): axum::extract::Query<OptimizationQuery>,
) -> Json<crate::controller::resource_optimization::ForecastResult> {
    let _ = query;
    let engine = ForecastEngine::new(ForecastModel::Ensemble);
    let history: Vec<TimeSeriesPoint> = (0..20)
        .map(|i| TimeSeriesPoint {
            timestamp: chrono::Utc::now(),
            value: 1000.0 + i as f64 * 50.0,
        })
        .collect();

    Json(engine.forecast(&history, 60))
}
