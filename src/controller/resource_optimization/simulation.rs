//! Capacity planning what-if simulation tooling.

use serde::{Deserialize, Serialize};

use super::forecasting::{ForecastEngine, ForecastModel, TimeSeriesPoint};

/// A what-if scenario for capacity simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationScenario {
    pub name: String,
    /// Scale factor applied to current workload (1.0 = baseline).
    pub scale_factor: f64,
    /// Additional replicas to add.
    #[serde(default)]
    pub additional_replicas: i32,
    /// Cost per replica per month (USD).
    #[serde(default = "default_cost_per_replica")]
    pub cost_per_replica_monthly: f64,
}

fn default_cost_per_replica() -> f64 {
    150.0
}

/// Result of a what-if simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationResult {
    pub scenario_name: String,
    pub projected_replicas: i32,
    pub projected_monthly_cost: f64,
    pub baseline_monthly_cost: f64,
    pub cost_savings_pct: f64,
    pub sla_compliant: bool,
    pub feasibility_score: f32,
    pub bottlenecks: Vec<String>,
    pub forecast_tps: f64,
}

/// Capacity simulator for what-if analysis.
pub struct CapacitySimulator {
    engine: ForecastEngine,
}

impl Default for CapacitySimulator {
    fn default() -> Self {
        Self {
            engine: ForecastEngine::new(ForecastModel::Ensemble),
        }
    }
}

impl CapacitySimulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run a what-if simulation against historical TPS data.
    pub fn simulate(
        &self,
        history: &[TimeSeriesPoint],
        current_replicas: i32,
        tps_per_replica: f64,
        scenario: &SimulationScenario,
    ) -> SimulationResult {
        let forecast = self.engine.forecast(history, 1);
        let baseline_tps = forecast
            .points
            .first()
            .map(|p| p.predicted)
            .unwrap_or_else(|| history.last().map(|p| p.value).unwrap_or(0.0));

        let projected_tps = baseline_tps * scenario.scale_factor;
        let needed_replicas = if tps_per_replica > 0.0 {
            ((projected_tps * 1.2) / tps_per_replica).ceil() as i32
        } else {
            current_replicas
        };

        let projected_replicas = (needed_replicas + scenario.additional_replicas).max(1);
        let baseline_cost = current_replicas as f64 * scenario.cost_per_replica_monthly;
        let projected_cost = projected_replicas as f64 * scenario.cost_per_replica_monthly;
        let savings = if baseline_cost > 0.0 {
            ((baseline_cost - projected_cost) / baseline_cost) * 100.0
        } else {
            0.0
        };

        let mut bottlenecks = Vec::new();
        if scenario.scale_factor > 2.0 {
            bottlenecks.push("Network bandwidth may become a bottleneck".to_string());
        }
        if projected_replicas > current_replicas * 3 {
            bottlenecks.push("Cluster capacity may be insufficient".to_string());
        }

        let feasibility = if bottlenecks.is_empty() && scenario.scale_factor <= 3.0 {
            0.9
        } else if scenario.scale_factor <= 5.0 {
            0.6
        } else {
            0.3
        };

        let sla_compliant = projected_replicas >= needed_replicas && scenario.scale_factor <= 4.0;

        SimulationResult {
            scenario_name: scenario.name.clone(),
            projected_replicas,
            projected_monthly_cost: projected_cost,
            baseline_monthly_cost: baseline_cost,
            cost_savings_pct: savings,
            sla_compliant,
            feasibility_score: feasibility,
            bottlenecks,
            forecast_tps: projected_tps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_history() -> Vec<TimeSeriesPoint> {
        (0..10)
            .map(|i| TimeSeriesPoint {
                timestamp: Utc::now(),
                value: 1000.0 + i as f64 * 100.0,
            })
            .collect()
    }

    #[test]
    fn simulate_double_traffic() {
        let sim = CapacitySimulator::new();
        let scenario = SimulationScenario {
            name: "2x traffic".to_string(),
            scale_factor: 2.0,
            additional_replicas: 0,
            cost_per_replica_monthly: 100.0,
        };
        let result = sim.simulate(&sample_history(), 3, 1000.0, &scenario);
        assert!(result.projected_replicas >= 3);
        assert!(result.forecast_tps > 0.0);
    }
}
