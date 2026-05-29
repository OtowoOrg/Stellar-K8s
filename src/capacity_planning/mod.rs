//! Intelligent Capacity Planning and Growth Forecasting
//!
//! Provides proactive resource recommendations based on historical usage analysis,
//! growth forecasting, and what-if scenario modeling.

pub mod analysis;
pub mod forecasting;
pub mod metrics;
pub mod recommendation;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Core resource metrics for capacity planning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub timestamp: DateTime<Utc>,
    pub cpu_cores: f64,
    pub memory_bytes: i64,
    pub storage_bytes: i64,
    pub network_ingress_bytes: i64,
    pub network_egress_bytes: i64,
}

/// A capacity recommendation for a specific node or cluster
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityRecommendation {
    pub target_resource: String,
    pub current_capacity: f64,
    pub recommended_capacity: f64,
    pub confidence_score: f32,
    pub rationale: String,
    pub estimated_cost_change: f64,
    pub deadline: DateTime<Utc>,
}

/// Growth forecast for a specific resource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrowthForecast {
    pub resource_type: String,
    pub forecast_points: Vec<(DateTime<Utc>, f64)>,
    pub model_used: String,
    pub growth_rate_pct: f64,
}

/// Result of a what-if scenario analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatIfResult {
    pub scenario_name: String,
    pub impacts: Vec<ResourceImpact>,
    pub feasibility_score: f32,
    pub estimated_monthly_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceImpact {
    pub resource: String,
    pub change_pct: f64,
    pub bottleneck_risk: bool,
}
