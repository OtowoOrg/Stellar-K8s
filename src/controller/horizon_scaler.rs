//! Stellar-Native Autoscaler for Horizon
//!
//! Scales Horizon pods based on Stellar-specific metrics:
//!
//! 1. **TPS (transactions per second)**: scale up when TPS per replica exceeds
//!    `tps_scale_up_threshold` (default: 100 TPS/replica).
//! 2. **Queue length**: scale up when the pending transaction queue exceeds
//!    `queue_scale_up_threshold` (default: 500 transactions).
//! 3. **HTTP 429 rate**: legacy path — scale up when rate-limit errors hit
//!    `threshold` (default: 1 req/s).
//!
//! The three signals are evaluated independently and the highest replica count
//! wins, so any single pressure point can trigger a scale-up.
//!
//! # HPA Integration
//!
//! This module's [`compute_replicas`] logic mirrors what the Kubernetes HPA
//! does when it reads from the custom metrics API.  It is also used directly
//! by the operator's own reconciliation loop so that the operator can apply
//! an immediate patch rather than waiting for the HPA's next sync cycle.

use crate::controller::predictive_scaling::fit_holt_winters;
use crate::crd::StellarNode;
use crate::error::Result;
use tracing::{debug, info};

/// Default TPS per-replica threshold — scale up when TPS/replica > this value.
pub const DEFAULT_TPS_SCALE_UP_THRESHOLD: f64 = 100.0;
/// Default queue-length threshold — scale up when queue > this value.
pub const DEFAULT_QUEUE_SCALE_UP_THRESHOLD: i64 = 500;
/// Default scale-down TPS hysteresis multiplier (scale down when TPS < threshold * this).
pub const DEFAULT_TPS_SCALE_DOWN_HYSTERESIS: f64 = 0.1;

/// Configuration for the Horizon rate-limit / TPS / queue-length autoscaler.
#[derive(Debug, Clone)]
pub struct HorizonScalerConfig {
    /// HTTP 429 rate threshold (requests/s) — legacy signal.
    pub rate_429_threshold: f64,
    /// TPS per-replica target — scale up when observed TPS per replica exceeds this.
    pub tps_scale_up_threshold: f64,
    /// Queue-length absolute threshold — scale up when queue exceeds this.
    pub queue_scale_up_threshold: i64,
}

impl Default for HorizonScalerConfig {
    fn default() -> Self {
        Self {
            rate_429_threshold: 1.0,
            tps_scale_up_threshold: DEFAULT_TPS_SCALE_UP_THRESHOLD,
            queue_scale_up_threshold: DEFAULT_QUEUE_SCALE_UP_THRESHOLD,
        }
    }
}

/// Decision produced by [`HorizonRateLimitScaler::compute_replicas`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalingDecision {
    /// Recommended target replica count.
    pub target_replicas: i32,
    /// Which signal drove the scale-up (for logging / events).
    pub signal: ScalingSignal,
}

/// The metric signal that drove the scaling decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalingSignal {
    /// No change warranted by any signal.
    NoChange,
    /// 429 rate exceeded the threshold.
    RateLimit429,
    /// TPS per replica exceeded the threshold.
    TransactionsPerSecond,
    /// Queue length exceeded the threshold.
    QueueLength,
    /// Scale-down: all signals below hysteresis thresholds.
    ScaleDown,
}

/// Stellar-native autoscaler for Horizon.
pub struct HorizonRateLimitScaler {
    prometheus_url: String,
    config: HorizonScalerConfig,
}

impl HorizonRateLimitScaler {
    pub fn new(prometheus_url: String) -> Self {
        Self {
            prometheus_url,
            config: HorizonScalerConfig::default(),
        }
    }

    /// Create with explicit configuration.
    pub fn with_config(prometheus_url: String, config: HorizonScalerConfig) -> Self {
        Self {
            prometheus_url,
            config,
        }
    }

    /// Fetch 429 error rates from Prometheus.
    pub async fn fetch_429_rate(&self, node_name: &str) -> Result<f64> {
        // Simulated Prometheus query for 429 error rates.
        // Production: rate(stellar_horizon_http_responses_total{status="429", node="..."}[5m])
        debug!(
            "Fetching 429 rate for node {} from {}",
            node_name, self.prometheus_url
        );
        Ok(0.5) // 0.5 requests/s hitting 429
    }

    /// Compute the recommended replica count from all three scaling signals.
    ///
    /// `current_tps` is the total observed TPS for the deployment (not per-replica).
    /// `queue_length` is the current pending transaction queue depth.
    ///
    /// Returns the highest replica count suggested by any individual signal,
    /// ensuring the most aggressive scale-up is always applied.
    pub fn compute_replicas(
        &self,
        current_replicas: i32,
        rate_429: f64,
        current_tps: i64,
        queue_length: i64,
    ) -> ScalingDecision {
        let threshold = self.config.rate_429_threshold;
        let tps_threshold = self.config.tps_scale_up_threshold;
        let queue_threshold = self.config.queue_scale_up_threshold;

        let mut target = current_replicas;
        let mut signal = ScalingSignal::NoChange;

        // --- Signal 1: HTTP 429 rate ---
        if rate_429 > threshold {
            let r = ((current_replicas as f64 * 1.5).ceil() as i32).max(current_replicas + 1);
            if r > target {
                target = r;
                signal = ScalingSignal::RateLimit429;
            }
        }

        // --- Signal 2: TPS per replica ---
        // If TPS/replica > threshold, add replicas so each would handle ~75% of threshold.
        if current_replicas > 0 {
            let tps_per_replica = current_tps as f64 / current_replicas as f64;
            if tps_per_replica > tps_threshold {
                // Target: enough replicas so each handles 75% of threshold.
                let target_tps_replicas =
                    (current_tps as f64 / (tps_threshold * 0.75)).ceil() as i32;
                if target_tps_replicas > target {
                    target = target_tps_replicas;
                    signal = ScalingSignal::TransactionsPerSecond;
                }
            }
        }

        // --- Signal 3: Absolute queue length ---
        if queue_length > queue_threshold {
            // Add one replica for every `queue_threshold` items above the threshold.
            let overage = (queue_length - queue_threshold) as f64 / queue_threshold as f64;
            let queue_replicas = current_replicas + 1 + overage.ceil() as i32;
            if queue_replicas > target {
                target = queue_replicas;
                signal = ScalingSignal::QueueLength;
            }
        }

        // --- Scale-down: all signals below hysteresis ---
        if (target == current_replicas)
            && (rate_429 < threshold * DEFAULT_TPS_SCALE_DOWN_HYSTERESIS)
            && ((current_tps as f64 / current_replicas.max(1) as f64)
                < (tps_threshold * DEFAULT_TPS_SCALE_DOWN_HYSTERESIS))
            && (queue_length < queue_threshold / 5)
            && (current_replicas > 2)
        {
            target = current_replicas - 1;
            signal = ScalingSignal::ScaleDown;
        }

        ScalingDecision {
            target_replicas: target,
            signal,
        }
    }

    /// Implement predictive scaling based on historical TPS data.
    pub fn predict_future_tps(&self, history: &[f64]) -> Option<f64> {
        let alpha = 0.3;
        let beta = 0.1;
        let state = fit_holt_winters(history, alpha, beta)?;
        Some(state.forecast(12)) // Forecast 1 hour (assuming 5-minute intervals)
    }

    /// Main reconciliation logic: reads all three signals and returns target replicas.
    ///
    /// `current_tps` and `queue_length` are fetched from the [`StellarMetricsStore`]
    /// by the caller (typically the reconciler loop).
    pub async fn reconcile_scaling(
        &self,
        node: &StellarNode,
        current_replicas: i32,
        current_tps: i64,
        queue_length: i64,
    ) -> Result<i32> {
        let node_name = node.metadata.name.as_ref().unwrap();
        let rate_429 = self.fetch_429_rate(node_name).await?;

        let decision = self.compute_replicas(current_replicas, rate_429, current_tps, queue_length);

        if let Some(ref _autoscaling) = node.spec.autoscaling {
            info!(
                "Horizon autoscaling evaluation for {} (signal: {:?})",
                node_name, decision.signal
            );
        }

        if decision.target_replicas != current_replicas {
            info!(
                "Scaling Horizon {} from {} → {} replicas (signal: {:?}, tps={}, queue={})",
                node_name,
                current_replicas,
                decision.target_replicas,
                decision.signal,
                current_tps,
                queue_length,
            );
        }

        Ok(decision.target_replicas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scaler() -> HorizonRateLimitScaler {
        HorizonRateLimitScaler::new("http://prometheus:9090".to_string())
    }

    // ---- 429-rate signal ----------------------------------------------------

    #[test]
    fn test_scale_up_on_high_429_rate() {
        let s = scaler();
        // 429 rate > threshold → scale up by 50%
        let d = s.compute_replicas(4, 2.0, 10, 0);
        assert!(d.target_replicas > 4);
        assert_eq!(d.signal, ScalingSignal::RateLimit429);
    }

    #[test]
    fn test_no_scale_on_low_429_rate() {
        let s = scaler();
        let d = s.compute_replicas(4, 0.5, 10, 0);
        assert_eq!(d.target_replicas, 4);
        assert_eq!(d.signal, ScalingSignal::NoChange);
    }

    // ---- TPS signal ---------------------------------------------------------

    #[test]
    fn test_scale_up_on_high_tps() {
        let s = scaler();
        // 2 replicas, 300 TPS → 150 TPS/replica > threshold (100)
        let d = s.compute_replicas(2, 0.0, 300, 0);
        assert!(d.target_replicas > 2, "expected scale-up, got {:?}", d);
        assert_eq!(d.signal, ScalingSignal::TransactionsPerSecond);
    }

    #[test]
    fn test_no_scale_on_low_tps() {
        let s = scaler();
        // 4 replicas, 80 TPS → 20 TPS/replica < threshold
        let d = s.compute_replicas(4, 0.0, 80, 0);
        assert_eq!(d.target_replicas, 4);
    }

    #[test]
    fn test_tps_target_formula() {
        let s = scaler();
        // 1 replica, 200 TPS → TPS/replica=200 > 100 → target = ceil(200 / 75) = 3
        let d = s.compute_replicas(1, 0.0, 200, 0);
        assert_eq!(d.target_replicas, 3);
        assert_eq!(d.signal, ScalingSignal::TransactionsPerSecond);
    }

    // ---- Queue-length signal -------------------------------------------------

    #[test]
    fn test_scale_up_on_high_queue() {
        let s = scaler();
        // queue=1000 > default threshold (500) → scale up
        let d = s.compute_replicas(3, 0.0, 0, 1000);
        assert!(d.target_replicas > 3, "expected scale-up, got {:?}", d);
        assert_eq!(d.signal, ScalingSignal::QueueLength);
    }

    #[test]
    fn test_no_scale_on_low_queue() {
        let s = scaler();
        let d = s.compute_replicas(3, 0.0, 0, 200);
        assert_eq!(d.target_replicas, 3);
    }

    // ---- Scale-down ---------------------------------------------------------

    #[test]
    fn test_scale_down_when_all_signals_low() {
        let s = scaler();
        // current_replicas=4, all signals near zero → scale down to 3
        let d = s.compute_replicas(4, 0.0, 5, 10);
        assert_eq!(d.target_replicas, 3);
        assert_eq!(d.signal, ScalingSignal::ScaleDown);
    }

    #[test]
    fn test_no_scale_down_at_minimum_replicas() {
        let s = scaler();
        // current_replicas=2 (minimum) → no scale-down
        let d = s.compute_replicas(2, 0.0, 5, 10);
        assert_eq!(d.target_replicas, 2);
    }

    // ---- Multiple signals: highest wins -------------------------------------

    #[test]
    fn test_queue_signal_wins_over_tps() {
        let s = scaler();
        // TPS and queue both suggest scale-up; queue suggests more replicas
        let d = s.compute_replicas(2, 0.0, 250, 5000);
        // queue_replicas = 2 + 1 + ceil((5000-500)/500) = 2+1+9=12
        // TPS: ceil(250/75) = 4
        assert_eq!(d.signal, ScalingSignal::QueueLength);
        assert!(d.target_replicas >= 12);
    }

    // ---- Custom config -------------------------------------------------------

    #[test]
    fn test_custom_config_tps_threshold() {
        let config = HorizonScalerConfig {
            rate_429_threshold: 5.0,
            tps_scale_up_threshold: 50.0, // lower threshold
            queue_scale_up_threshold: 1000,
        };
        let s = HorizonRateLimitScaler::with_config("http://prom:9090".to_string(), config);
        // 2 replicas, 120 TPS → 60 TPS/replica > 50 threshold → scale up
        let d = s.compute_replicas(2, 0.0, 120, 0);
        assert!(d.target_replicas > 2);
        assert_eq!(d.signal, ScalingSignal::TransactionsPerSecond);
    }
}
