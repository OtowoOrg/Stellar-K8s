//! Capacity Recommendation Engine
//!
//! Generates actionable recommendations based on forecasted resource needs.

use crate::capacity_planning::{ResourceUsage, CapacityRecommendation, GrowthForecast};
use chrono::{Duration, Utc};

pub struct RecommendationEngine {
    pub safety_margin: f64,
}

impl RecommendationEngine {
    pub fn new(safety_margin: f64) -> Self {
        Self { safety_margin }
    }

    /// Generates recommendations based on growth forecasts
    pub fn generate_recommendations(&self, forecasts: &[GrowthForecast]) -> Vec<CapacityRecommendation> {
        let mut recommendations = Vec::new();

        for forecast in forecasts {
            if let Some((target_time, predicted_val)) = forecast.forecast_points.last() {
                let recommended = predicted_val * self.safety_margin;
                
                // If growth is significant (> 10%), recommend expansion
                if forecast.growth_rate_pct > 10.0 {
                    recommendations.push(CapacityRecommendation {
                        target_resource: forecast.resource_type.clone(),
                        current_capacity: forecast.forecast_points[0].1, // Simplified
                        recommended_capacity: recommended,
                        confidence_score: 0.85,
                        rationale: format!(
                            "Forecasted growth of {:.1}% detected using {} model.",
                            forecast.growth_rate_pct, forecast.model_used
                        ),
                        estimated_cost_change: (recommended - forecast.forecast_points[0].1) * 20.0, // Mock cost
                        deadline: *target_time - Duration::days(7), // Recommend 1 week before
                    });
                }
            }
        }

        recommendations
    }

    /// Identifies potential bottlenecks before they happen
    pub fn identify_bottlenecks(&self, history: &[ResourceUsage], thresholds: &ResourceUsage) -> Vec<String> {
        let mut bottlenecks = Vec::new();
        
        if let Some(last) = history.last() {
            if last.cpu_cores > thresholds.cpu_cores * 0.8 {
                bottlenecks.push("CPU usage is approaching 80% of current capacity".to_string());
            }
            if last.memory_bytes > (thresholds.memory_bytes as f64 * 0.8) as i64 {
                bottlenecks.push("Memory usage is approaching 80% of current capacity".to_string());
            }
        }

        bottlenecks
    }
}
