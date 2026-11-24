// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Cost optimization dashboard spec and drill-down reporting

use serde::{Deserialize, Serialize};

use crate::scheduler::savings::SavingsAggregator;

use super::allocation::CostAllocation;
use super::anomaly::CostAnomaly;
use super::forecast::CostForecast;
use super::recommender::OptimizationRecommendation;

/// Dashboard summary rendered for the operator UI
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostDashboard {
    pub total_monthly_cost_usd: f64,
    pub total_potential_savings_usd: f64,
    pub savings_pct: f64,
    pub active_anomalies: usize,
    pub top_recommendations: Vec<String>,
    pub namespace_breakdown: Vec<NamespaceRow>,
    pub forecast_30d_usd: f64,
    pub prometheus_metrics: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamespaceRow {
    pub namespace: String,
    pub team: String,
    pub cost_usd: f64,
    pub cost_pct: f64,
}

impl CostDashboard {
    pub fn build(
        allocation: &CostAllocation,
        anomalies: &[CostAnomaly],
        recommendations: &[OptimizationRecommendation],
        forecasts: &[CostForecast],
    ) -> Self {
        Self::build_with_realized(
            allocation,
            anomalies,
            recommendations,
            forecasts,
            &SavingsAggregator::new(),
            0.0,
        )
    }

    /// Attach first-class realized savings from the placement aggregator.
    pub fn build_with_realized(
        allocation: &CostAllocation,
        anomalies: &[CostAnomaly],
        recommendations: &[OptimizationRecommendation],
        forecasts: &[CostForecast],
        realized: &SavingsAggregator,
        spot_ratio: f64,
    ) -> Self {
        let total = allocation.total();
        let savings: f64 = recommendations
            .iter()
            .map(|r| r.estimated_monthly_savings)
            .sum();
        let forecast_30d = forecasts.iter().map(|f| f.forecast_30d_usd).sum::<f64>()
            / forecasts.len().max(1) as f64;

        let namespace_breakdown = allocation
            .by_namespace()
            .iter()
            .map(|ns| NamespaceRow {
                namespace: ns.namespace.clone(),
                team: ns.team.clone(),
                cost_usd: ns.total_cost_usd,
                cost_pct: if total > 0.0 {
                    ns.total_cost_usd / total * 100.0
                } else {
                    0.0
                },
            })
            .collect();

        let top_recommendations = recommendations
            .iter()
            .take(5)
            .map(|r| r.description.clone())
            .collect();

        let mut prometheus_metrics = format!(
            "# TYPE stellar_cost_total_monthly_usd gauge\n\
             stellar_cost_total_monthly_usd {:.2}\n\
             # TYPE stellar_cost_potential_savings_usd gauge\n\
             stellar_cost_potential_savings_usd {:.2}\n\
             # TYPE stellar_cost_anomalies_active gauge\n\
             stellar_cost_anomalies_active {}\n",
            total,
            savings,
            anomalies.len(),
        );
        prometheus_metrics.push_str(&realized.prometheus_metrics(spot_ratio));

        Self {
            total_monthly_cost_usd: total,
            total_potential_savings_usd: savings,
            savings_pct: if total > 0.0 {
                savings / total * 100.0
            } else {
                0.0
            },
            active_anomalies: anomalies.len(),
            top_recommendations,
            namespace_breakdown,
            forecast_30d_usd: forecast_30d,
            prometheus_metrics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_optimization::allocation::CostAllocation;
    use crate::scheduler::capacity::{best_effort_workload, defined_test_cluster, place_workloads};
    use crate::scheduler::savings::{COST_SAVINGS_METRIC, HOURLY_SAVINGS_METRIC};
    use chrono::Utc;

    #[test]
    fn dashboard_includes_hourly_realized_savings() {
        let nodes = defined_test_cluster();
        let workloads: Vec<_> = (0..10)
            .map(|i| best_effort_workload(&format!("be-{i}")))
            .collect();
        let report = place_workloads(&workloads, &nodes, &[]);
        let mut agg = SavingsAggregator::new();
        agg.record_placement(Utc::now(), &report);
        let dash = CostDashboard::build_with_realized(
            &CostAllocation::default(),
            &[],
            &[],
            &[],
            &agg,
            report.best_effort_spot_ratio(),
        );
        assert!(dash.prometheus_metrics.contains(COST_SAVINGS_METRIC));
        assert!(dash.prometheus_metrics.contains(HOURLY_SAVINGS_METRIC));
        assert!(dash
            .prometheus_metrics
            .contains("stellar_best_effort_spot_placement_ratio"));
        assert!(agg.latest_hourly_usd() > 0.0);
    }
}
