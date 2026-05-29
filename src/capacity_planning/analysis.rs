//! Scenario Analysis and Cost Projection
//!
//! Evaluates the impact of hypothetical changes and projects associated costs.

use crate::capacity_planning::{ResourceImpact, WhatIfResult};

pub struct ScenarioAnalyzer;

impl ScenarioAnalyzer {
    /// Evaluates a what-if scenario (e.g., "What if we double the number of Horizon nodes?")
    pub fn analyze_scenario(&self, name: &str, scale_factor: f64) -> WhatIfResult {
        let mut impacts = Vec::new();

        // Model impact on CPU
        impacts.push(ResourceImpact {
            resource: "CPU".to_string(),
            change_pct: (scale_factor - 1.0) * 100.0,
            bottleneck_risk: scale_factor > 2.0,
        });

        // Model impact on Memory
        impacts.push(ResourceImpact {
            resource: "Memory".to_string(),
            change_pct: (scale_factor - 1.0) * 100.0,
            bottleneck_risk: scale_factor > 1.8,
        });

        // Model impact on Network
        impacts.push(ResourceImpact {
            resource: "Network".to_string(),
            change_pct: (scale_factor - 1.0) * 120.0, // Non-linear network growth
            bottleneck_risk: scale_factor > 1.5,
        });

        WhatIfResult {
            scenario_name: name.to_string(),
            impacts,
            feasibility_score: if scale_factor > 3.0 { 0.4 } else { 0.9 },
            estimated_monthly_cost: 500.0 * scale_factor, // Mock base cost
        }
    }

    /// Projects future costs based on growth forecasts
    pub fn project_costs(&self, growth_rate: f64, base_cost: f64, months: u32) -> Vec<(u32, f64)> {
        let mut projections = Vec::new();
        let mut current_cost = base_cost;

        for month in 1..=months {
            current_cost *= 1.0 + (growth_rate / 100.0);
            projections.push((month, current_cost));
        }

        projections
    }
}
