//! Stellar-Native Autoscaler for Horizon
//!
//! Scales Horizon pods based on the frequency of HTTP 429 (Too Many Requests)
//! responses exported from Prometheus. Supports predictive scaling.

use crate::controller::predictive_scaling::fit_holt_winters;
use crate::crd::StellarNode;
use crate::error::Result;
use tracing::{debug, info};

pub struct HorizonRateLimitScaler {
    prometheus_url: String,
}

impl HorizonRateLimitScaler {
    pub fn new(prometheus_url: String) -> Self {
        Self { prometheus_url }
    }

    /// Fetch 429 error rates from Prometheus
    pub async fn fetch_429_rate(&self, node_name: &str) -> Result<f64> {
        // Simulated Prometheus query for 429 error rates
        // Example: rate(stellar_horizon_http_responses_total{status="429", node="..."}[5m])
        debug!(
            "Fetching 429 rate for node {} from {}",
            node_name, self.prometheus_url
        );

        // Return a simulated value for now
        Ok(0.5) // 0.5 requests per second hitting 429
    }

    /// Compute desired replicas based on 429 frequency
    pub fn compute_replicas(&self, current_replicas: i32, rate_429: f64, threshold: f64) -> i32 {
        if rate_429 > threshold {
            // Scale up: increase by 50% or at least 1
            ((current_replicas as f64 * 1.5).ceil() as i32).max(current_replicas + 1)
        } else if rate_429 < (threshold * 0.1) && current_replicas > 2 {
            // Scale down: decrease by 1
            current_replicas - 1
        } else {
            current_replicas
        }
    }

    /// Implement predictive scaling based on historical 429 spikes
    pub fn predict_future_429_rate(&self, history: &[f64]) -> Option<f64> {
        let alpha = 0.3;
        let beta = 0.1;
        let state = fit_holt_winters(history, alpha, beta)?;
        Some(state.forecast(12)) // Forecast 1 hour (assuming 5m intervals)
    }

    /// Main reconciliation logic for rate-limit based scaling
    pub async fn reconcile_scaling(
        &self,
        node: &StellarNode,
        current_replicas: i32,
    ) -> Result<i32> {
        let node_name = node.metadata.name.as_ref().unwrap();
        let rate_429 = self.fetch_429_rate(node_name).await?;

        let threshold = 1.0; // 1 request per second hitting 429
        let target_replicas = self.compute_replicas(current_replicas, rate_429, threshold);

        // If predictive scaling is enabled, adjust target
        if let Some(ref _autoscaling) = node.spec.autoscaling {
            // This is just a placeholder for future integration with PredictiveScalingConfig
            info!(
                "Rate-limit 기반 predictive scaling evaluation for {}",
                node_name
            );
        }

        if target_replicas != current_replicas {
            info!(
                "Scaling node {} from {} to {} based on 429 rate ({})",
                node_name, current_replicas, target_replicas, rate_429
            );
        }

        Ok(target_replicas)
    }
}
