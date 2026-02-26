//! Sustainability Dashboard API endpoints
//!
//! Provides REST API endpoints for monitoring CO2 footprint and carbon intensity
//! of managed Stellar infrastructure.

use crate::carbon_aware::{CarbonAwareScheduler, CarbonIntensityAPI};
use crate::error::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, Router},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

/// Sustainability dashboard state
#[derive(Clone)]
#[allow(dead_code)]
pub struct SustainabilityState {
    pub carbon_scheduler: Arc<CarbonAwareScheduler>,
    pub carbon_api: Arc<CarbonIntensityAPI>,
}

/// Sustainability metrics response
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct SustainabilityMetrics {
    /// Current timestamp
    pub timestamp: DateTime<Utc>,
    /// Total number of regions monitored
    pub regions_count: usize,
    /// Average carbon intensity across all regions (gCO2/kWh)
    pub average_intensity: f64,
    /// Best region (lowest carbon intensity)
    pub best_region: Option<RegionInfo>,
    /// Worst region (highest carbon intensity)
    pub worst_region: Option<RegionInfo>,
    /// Carbon intensity data by region
    pub regions: Vec<RegionInfo>,
    /// Data freshness status
    pub data_status: DataStatus,
    /// CO2 footprint of managed nodes
    pub node_footprint: Vec<NodeFootprint>,
}

/// Region carbon information
#[derive(Clone, Serialize, Debug)]
#[allow(dead_code)]
pub struct RegionInfo {
    /// Region identifier
    pub region: String,
    /// Current carbon intensity (gCO2/kWh)
    pub carbon_intensity: f64,
    /// Renewable energy percentage
    pub renewable_percentage: Option<f64>,
    /// Data source
    pub source: String,
    /// Last update timestamp
    pub last_updated: DateTime<Utc>,
    /// Relative ranking (1=best)
    pub ranking: Option<usize>,
}

/// Data status information
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct DataStatus {
    /// Last successful update
    pub last_updated: DateTime<Utc>,
    /// Whether data is stale
    pub is_stale: bool,
    /// Data age in minutes
    pub age_minutes: i64,
}

/// Node CO2 footprint information
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct NodeFootprint {
    /// Node name
    pub node_name: String,
    /// Node type
    pub node_type: String,
    /// Region where node is deployed
    pub region: String,
    /// Current carbon intensity (gCO2/kWh)
    pub carbon_intensity: f64,
    /// Estimated hourly CO2 emissions (gCO2/hour)
    pub hourly_emissions: f64,
    /// Estimated daily CO2 emissions (gCO2/day)
    pub daily_emissions: f64,
    /// Power consumption estimate (Watts)
    pub power_consumption: f64,
}

/// Carbon intensity history request
#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct CarbonHistoryRequest {
    /// Region to query
    pub region: String,
    /// Time range in hours (default: 24)
    pub hours: Option<u32>,
}

/// Carbon intensity forecast response
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct CarbonForecastResponse {
    /// Region identifier
    pub region: String,
    /// Forecast data points
    pub forecast: Vec<ForecastPoint>,
    /// Forecast generated at
    pub generated_at: DateTime<Utc>,
}

/// Single forecast data point
#[derive(Serialize, Debug)]
#[allow(dead_code)]
pub struct ForecastPoint {
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Predicted carbon intensity (gCO2/kWh)
    pub carbon_intensity: f64,
    /// Confidence level (0-1)
    pub confidence: f64,
}

/// Create sustainability dashboard router
#[allow(dead_code)]
pub fn sustainability_router() -> Router<SustainabilityState> {
    Router::new()
        .route("/metrics", get(get_sustainability_metrics))
        .route("/regions", get(get_region_data))
        .route("/regions/:region", get(get_region_details))
        .route("/forecast/:region", get(get_carbon_forecast))
        .route("/nodes", get(get_node_footprints))
        .route("/health", get(get_carbon_api_health))
}

/// Get overall sustainability metrics
#[allow(dead_code)]
pub async fn get_sustainability_metrics(
    State(state): State<SustainabilityState>,
) -> Result<Json<SustainabilityMetrics>, StatusCode> {
    info!("Fetching sustainability metrics");

    // Get carbon statistics from scheduler
    let carbon_stats: crate::carbon_aware::scheduler::CarbonStats = state
        .carbon_scheduler
        .get_carbon_stats()
        .await
        .map_err(|e| {
            debug!("Failed to get carbon stats: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Get detailed region data
    let carbon_data: crate::carbon_aware::types::RegionCarbonData =
        state.carbon_api.fetch_all_regions().await.map_err(|e| {
            debug!("Failed to fetch carbon data: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut regions: Vec<RegionInfo> = carbon_data
        .regions
        .values()
        .map(|data| RegionInfo {
            region: data.region.clone(),
            carbon_intensity: data.carbon_intensity,
            renewable_percentage: data.renewable_percentage,
            source: data.source.clone(),
            last_updated: data.timestamp,
            ranking: None,
        })
        .collect();

    // Sort by carbon intensity and add rankings
    regions.sort_by(|a, b| a.carbon_intensity.partial_cmp(&b.carbon_intensity).unwrap());
    for (i, region) in regions.iter_mut().enumerate() {
        region.ranking = Some(i + 1);
    }

    let best_region = regions.first().cloned();
    let worst_region = regions.last().cloned();

    // Calculate data status
    let now = Utc::now();
    let age_minutes = now
        .signed_duration_since(carbon_data.last_updated)
        .num_minutes();
    let data_status = DataStatus {
        last_updated: carbon_data.last_updated,
        is_stale: age_minutes > 15, // Consider stale after 15 minutes
        age_minutes,
    };

    // Mock node footprint data (in real implementation, this would come from actual node metrics)
    let node_footprint = generate_mock_node_footprints(&regions).await;

    let metrics = SustainabilityMetrics {
        timestamp: now,
        regions_count: carbon_stats.regions_count,
        average_intensity: carbon_stats.average_intensity,
        best_region,
        worst_region,
        regions,
        data_status,
        node_footprint,
    };

    Ok(Json(metrics))
}

/// Get region-specific carbon data
#[allow(dead_code)]
pub async fn get_region_data(
    State(state): State<SustainabilityState>,
) -> Result<Json<Vec<RegionInfo>>, StatusCode> {
    let carbon_data: crate::carbon_aware::types::RegionCarbonData = state
        .carbon_api
        .fetch_all_regions()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let regions: Vec<RegionInfo> = carbon_data
        .regions
        .values()
        .map(|data| RegionInfo {
            region: data.region.clone(),
            carbon_intensity: data.carbon_intensity,
            renewable_percentage: data.renewable_percentage,
            source: data.source.clone(),
            last_updated: data.timestamp,
            ranking: None,
        })
        .collect();

    Ok(Json(regions))
}

/// Get detailed information for a specific region
#[allow(dead_code)]
pub async fn get_region_details(
    State(state): State<SustainabilityState>,
    axum::extract::Path(region): axum::extract::Path<String>,
) -> Result<Json<RegionInfo>, StatusCode> {
    let carbon_data: Option<crate::carbon_aware::types::CarbonIntensityData> = state
        .carbon_api
        .fetch_region(&region)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match carbon_data {
        Some(data) => {
            let region_info = RegionInfo {
                region: data.region.clone(),
                carbon_intensity: data.carbon_intensity,
                renewable_percentage: data.renewable_percentage,
                source: data.source.clone(),
                last_updated: data.timestamp,
                ranking: None,
            };
            Ok(Json(region_info))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// Get carbon intensity forecast for a region
#[allow(dead_code)]
pub async fn get_carbon_forecast(
    State(_state): State<SustainabilityState>,
    axum::extract::Path(region): axum::extract::Path<String>,
) -> Result<Json<CarbonForecastResponse>, StatusCode> {
    // For now, return mock forecast data
    // In real implementation, this would call the carbon API's forecast endpoint
    let forecast = generate_mock_forecast(&region);

    Ok(Json(CarbonForecastResponse {
        region,
        forecast,
        generated_at: Utc::now(),
    }))
}

/// Get CO2 footprint information for managed nodes
#[allow(dead_code)]
pub async fn get_node_footprints(
    State(_state): State<SustainabilityState>,
) -> Result<Json<Vec<NodeFootprint>>, StatusCode> {
    // Mock node footprint data
    let footprints = generate_mock_node_footprints(&[]).await;
    Ok(Json(footprints))
}

/// Check carbon API health
#[allow(dead_code)]
pub async fn get_carbon_api_health(
    State(state): State<SustainabilityState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_healthy: bool = state
        .carbon_api
        .health_check()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let health = serde_json::json!({
        "healthy": is_healthy,
        "timestamp": Utc::now(),
        "api": "carbon_intensity"
    });

    Ok(Json(health))
}

/// Generate mock node footprint data
#[allow(dead_code)]
async fn generate_mock_node_footprints(regions: &[RegionInfo]) -> Vec<NodeFootprint> {
    // Mock node data with realistic power consumption
    let mock_nodes = vec![
        ("stellar-validator-1", "Validator", "us-west-2", 100.0),
        ("stellar-horizon-1", "Horizon", "us-east-1", 150.0),
        ("stellar-read-0", "ReadReplica", "eu-west-1", 50.0),
        ("stellar-read-1", "ReadReplica", "eu-west-1", 50.0),
        ("stellar-soroban-1", "SorobanRpc", "ap-southeast-1", 120.0),
    ];

    let region_intensity_map: HashMap<String, f64> = regions
        .iter()
        .map(|r| (r.region.clone(), r.carbon_intensity))
        .collect();

    mock_nodes
        .into_iter()
        .map(|(name, node_type, region, power_watts)| {
            let carbon_intensity = region_intensity_map.get(region).copied().unwrap_or(400.0); // Default if not found

            let hourly_emissions = (power_watts / 1000.0) * carbon_intensity; // kWh * gCO2/kWh
            let daily_emissions = hourly_emissions * 24.0;

            NodeFootprint {
                node_name: name.to_string(),
                node_type: node_type.to_string(),
                region: region.to_string(),
                carbon_intensity,
                hourly_emissions,
                daily_emissions,
                power_consumption: power_watts,
            }
        })
        .collect()
}

/// Generate mock forecast data
#[allow(dead_code)]
fn generate_mock_forecast(region: &str) -> Vec<ForecastPoint> {
    let mut forecast = Vec::new();
    let now = Utc::now();

    // Generate 24-hour forecast with some variation
    let base_intensity = match region {
        "us-west-2" => 150.0,
        "us-east-1" => 400.0,
        "eu-west-1" => 300.0,
        "eu-central-1" => 450.0,
        "ap-southeast-1" => 600.0,
        _ => 350.0,
    };

    for hour in 0..24 {
        let timestamp = now + chrono::Duration::hours(hour);
        // Add some daily variation (lower during "daylight" hours)
        let variation = if (hour % 24) >= 8 && (hour % 24) <= 18 {
            -20.0 // Daytime - more solar
        } else {
            30.0 // Nighttime - less solar
        };

        let carbon_intensity = (base_intensity + variation + (hour as f64 * 2.0)).max(50.0);
        let confidence = 0.8 - (hour as f64 * 0.02); // Decreasing confidence over time

        forecast.push(ForecastPoint {
            timestamp,
            carbon_intensity,
            confidence: confidence.max(0.5),
        });
    }

    forecast
}
