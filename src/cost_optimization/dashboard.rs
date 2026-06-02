//! Cost optimization dashboard spec and drill-down reporting

use serde::{Deserialize, Serialize};

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
        let total = allocation.total();
        let savings: f64 = recommendations.iter().map(|r| r.estimated_monthly_savings).sum();
        let forecast_30d = forecasts.iter().map(|f| f.forecast_30d_usd).sum::<f64>() / forecasts.len().max(1) as f64;

        let namespace_breakdown = allocation
            .by_namespace()
            .iter()
            .map(|ns| NamespaceRow {
                namespace: ns.namespace.clone(),
                team: ns.team.clone(),
                cost_usd: ns.total_cost_usd,
                cost_pct: if total > 0.0 { ns.total_cost_usd / total * 100.0 } else { 0.0 },
            })
            .collect();

        let top_recommendations = recommendations
            .iter()
            .take(5)
            .map(|r| r.description.clone())
            .collect();

        let prometheus_metrics = format!(
            "# TYPE stellar_cost_total_monthly_usd gauge\n\
             stellar_cost_total_monthly_usd {:.2}\n\
             # TYPE stellar_cost_potential_savings_usd gauge\n\
             stellar_cost_potential_savings_usd {:.2}\n\
             # TYPE stellar_cost_anomalies_active gauge\n\
             stellar_cost_anomalies_active {}\n",
            total, savings, anomalies.len(),
        );

        Self {
            total_monthly_cost_usd: total,
            total_potential_savings_usd: savings,
            savings_pct: if total > 0.0 { savings / total * 100.0 } else { 0.0 },
            active_anomalies: anomalies.len(),
            top_recommendations,
            namespace_breakdown,
            forecast_30d_usd: forecast_30d,
            prometheus_metrics,
        }
    }
}
