//! Predictive autoscaling controller with ML-based workload prediction.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use super::forecasting::{ForecastEngine, ForecastModel, TimeSeriesPoint};
use super::metrics::record_optimization_metrics;
use super::sla::{SlaConstraint, SlaEvaluator, SlaMetrics};
use super::vpa_optimizer::{ResourceObservation, VpaOptimization, VpaOptimizer, VpaRecommendation};
use crate::controller::predictive_scaling::{
    compute_min_replicas, scrape_prometheus_metric, LedgerVolumeCollector, LedgerVolumePoint,
    PredictiveScalingConfig,
};

/// Resource optimization configuration embedded in StellarNode spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceOptimizationConfig {
    /// Enable ML-based resource optimization.
    #[serde(default)]
    pub enabled: bool,
    /// Predictive horizontal scaling settings.
    #[serde(default)]
    pub predictive_scaling: PredictiveScalingConfig,
    /// SLA constraints for optimization.
    #[serde(default)]
    pub sla: SlaConstraint,
    /// VPA optimization settings.
    #[serde(default)]
    pub vpa_optimization: VpaOptimization,
    /// Forecast horizon in minutes.
    #[serde(default = "default_forecast_horizon")]
    pub forecast_horizon_minutes: u32,
}

fn default_forecast_horizon() -> u32 {
    60
}

impl Default for ResourceOptimizationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            predictive_scaling: PredictiveScalingConfig::default(),
            sla: SlaConstraint::default(),
            vpa_optimization: VpaOptimization::default(),
            forecast_horizon_minutes: default_forecast_horizon(),
        }
    }
}

/// Optimization recommendation produced by the controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationRecommendation {
    pub namespace: String,
    pub node_name: String,
    pub recommended_min_replicas: i32,
    pub current_min_replicas: i32,
    pub forecast_tps: f64,
    pub prediction_confidence: f64,
    pub prediction_mape: f64,
    pub sla_compliant: bool,
    pub cost_savings_pct: f64,
    pub vpa_recommendation: Option<VpaRecommendation>,
    pub generated_at: chrono::DateTime<Utc>,
}

/// ML-based resource optimization controller.
pub struct OptimizationController {
    config: ResourceOptimizationConfig,
    collector: LedgerVolumeCollector,
    engine: ForecastEngine,
}

impl OptimizationController {
    pub fn new(config: ResourceOptimizationConfig) -> Self {
        Self {
            config,
            collector: LedgerVolumeCollector::new(120),
            engine: ForecastEngine::new(ForecastModel::Ensemble),
        }
    }

    /// Record a TPS observation into the time-series buffer.
    pub fn record_observation(&mut self, tps: f64) {
        self.collector.record(LedgerVolumePoint {
            timestamp: Utc::now(),
            tps,
        });
    }

    /// Compute optimization recommendation from collected observations.
    pub fn recommend(
        &self,
        namespace: &str,
        node_name: &str,
        current_min: i32,
        max_replicas: i32,
        sla_metrics: &SlaMetrics,
    ) -> Option<OptimizationRecommendation> {
        if !self.config.enabled || self.collector.len() < 2 {
            return None;
        }

        let history: Vec<TimeSeriesPoint> = self
            .collector
            .observations()
            .iter()
            .map(|p| TimeSeriesPoint {
                timestamp: p.timestamp,
                value: p.tps,
            })
            .collect();

        let forecast = self
            .engine
            .forecast(&history, self.config.forecast_horizon_minutes);

        let forecast_tps = forecast.points.first().map(|p| p.predicted).unwrap_or(0.0);

        let base_replicas = compute_min_replicas(
            forecast_tps,
            self.config.predictive_scaling.tps_per_replica,
            self.config.predictive_scaling.scaling_factor,
            current_min,
            max_replicas,
        );

        let mut metrics = sla_metrics.clone();
        metrics.forecast_tps = forecast_tps;

        let recommended = SlaEvaluator::adjust_replicas(
            &self.config.sla,
            &metrics,
            base_replicas,
            self.config.predictive_scaling.tps_per_replica,
            max_replicas,
        );

        let sla_compliant = SlaEvaluator::is_compliant(&self.config.sla, &metrics);

        let baseline_cost = current_min as f64 * 150.0;
        let optimized_cost = recommended as f64 * 150.0;
        let cost_savings = if baseline_cost > 0.0 && optimized_cost < baseline_cost {
            ((baseline_cost - optimized_cost) / baseline_cost) * 100.0
        } else {
            0.0
        };

        let vpa_recommendation = if self.config.vpa_optimization.enabled {
            let obs = ResourceObservation {
                cpu_millicores: forecast_tps * 0.5,
                memory_bytes: forecast_tps * 1024.0 * 1024.0,
                forecast_cpu_millicores: forecast_tps * 0.5,
                forecast_memory_bytes: forecast_tps * 1024.0 * 1024.0,
            };
            Some(VpaOptimizer::recommend(&self.config.vpa_optimization, &obs))
        } else {
            None
        };

        record_optimization_metrics(
            namespace,
            node_name,
            cost_savings,
            if sla_compliant { 100.0 } else { 0.0 },
            forecast.mape,
        );

        Some(OptimizationRecommendation {
            namespace: namespace.to_string(),
            node_name: node_name.to_string(),
            recommended_min_replicas: recommended,
            current_min_replicas: current_min,
            forecast_tps,
            prediction_confidence: forecast.confidence,
            prediction_mape: forecast.mape,
            sla_compliant,
            cost_savings_pct: cost_savings,
            vpa_recommendation,
            generated_at: Utc::now(),
        })
    }

    /// Scrape Prometheus and update observations, then return recommendation.
    pub async fn run_cycle(
        &mut self,
        namespace: &str,
        node_name: &str,
        current_min: i32,
        max_replicas: i32,
        sla_metrics: &SlaMetrics,
    ) -> Option<OptimizationRecommendation> {
        if !self.config.enabled {
            return None;
        }

        let label_filters = format!("namespace=\"{namespace}\",node=\"{node_name}\"");
        if let Some(tps) = scrape_prometheus_metric(
            &self.config.predictive_scaling.prometheus_url,
            &self.config.predictive_scaling.ledger_volume_metric,
            &label_filters,
        )
        .await
        {
            self.record_observation(tps);
            debug!(node = %node_name, tps, "Recorded optimization observation");
        }

        let rec = self.recommend(namespace, node_name, current_min, max_replicas, sla_metrics)?;

        if rec.recommended_min_replicas != current_min {
            info!(
                node = %node_name,
                current_min,
                recommended = rec.recommended_min_replicas,
                forecast_tps = rec.forecast_tps,
                confidence = rec.prediction_confidence,
                "Resource optimization recommendation generated"
            );
        }

        Some(rec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_generates_recommendation() {
        let mut ctrl = OptimizationController::new(ResourceOptimizationConfig {
            enabled: true,
            ..Default::default()
        });

        for tps in [1000.0, 1100.0, 1200.0, 1300.0, 1400.0] {
            ctrl.record_observation(tps);
        }

        let metrics = SlaMetrics {
            p99_latency_ms: 100.0,
            availability_pct: 99.99,
            error_rate: 0.001,
            current_replicas: 2,
            forecast_tps: 0.0,
        };

        let rec = ctrl
            .recommend("stellar", "horizon-1", 2, 10, &metrics)
            .expect("should produce recommendation");

        assert!(rec.recommended_min_replicas >= 2);
        assert!(rec.prediction_confidence > 0.0);
    }

    #[test]
    fn disabled_controller_returns_none() {
        let ctrl = OptimizationController::new(ResourceOptimizationConfig::default());
        let metrics = SlaMetrics::default();
        assert!(ctrl.recommend("ns", "node", 1, 5, &metrics).is_none());
    }
}
