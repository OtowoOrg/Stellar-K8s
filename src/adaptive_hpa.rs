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

//! # Adaptive Horizontal Pod Autoscaling on Custom Business SLIs (Epic #1478)
//!
//! Extends Kubernetes autoscaling with domain-specific Stellar Service Level
//! Indicators (SLIs). Provides adaptive EWMA predictive smoothing, asymmetrical
//! hysteresis (fast scale-up, stabilized scale-down), and monthly cost caps.
//!
//! ## Monitored Business SLIs
//!
//! 1. **LedgerCloseLatencyMillis**: Consensus ledger close duration (target < 5000ms).
//! 2. **LedgerSyncLag**: Number of ledgers behind network tip (target <= 1 ledger).
//! 3. **HorizonTxQueueDepth**: Ingestion and submission queue size (target <= 200 txs).
//! 4. **SorobanInvocationLatency**: Smart contract execution p95 latency (target < 300ms).
//! 5. **FeeStatCongestionRatio**: Network fee market surge indicator (target < 0.85).
//! 6. **TransactionThroughputTps**: Workload TPS per replica (target 100 TPS/replica).

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Types of Stellar business SLIs driving autoscaling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum BusinessSliMetric {
    LedgerCloseLatencyMillis {
        p95_latency_ms: f64,
        target_max_ms: f64,
    },
    LedgerSyncLag {
        lag_ledgers: u64,
        target_max_ledgers: u64,
    },
    HorizonTxQueueDepth {
        queued_transactions: u64,
        target_max_queue: u64,
    },
    SorobanInvocationLatency {
        p95_latency_ms: f64,
        target_max_ms: f64,
    },
    FeeStatCongestionRatio {
        congestion_ratio: f64,
        target_max_ratio: f64,
    },
    TransactionThroughputTps {
        current_tps: f64,
        target_tps_per_replica: f64,
    },
}

impl BusinessSliMetric {
    pub fn name(&self) -> &'static str {
        match self {
            Self::LedgerCloseLatencyMillis { .. } => "LedgerCloseLatencyMillis",
            Self::LedgerSyncLag { .. } => "LedgerSyncLag",
            Self::HorizonTxQueueDepth { .. } => "HorizonTxQueueDepth",
            Self::SorobanInvocationLatency { .. } => "SorobanInvocationLatency",
            Self::FeeStatCongestionRatio { .. } => "FeeStatCongestionRatio",
            Self::TransactionThroughputTps { .. } => "TransactionThroughputTps",
        }
    }

    /// Compute the scaling pressure ratio: observed / target.
    /// Ratio > 1.0 implies scale-up pressure; ratio < 1.0 implies scale-down opportunity.
    pub fn scaling_ratio(&self, current_replicas: i32) -> f64 {
        let replicas = (current_replicas.max(1)) as f64;
        match self {
            Self::LedgerCloseLatencyMillis {
                p95_latency_ms,
                target_max_ms,
            } => {
                if *target_max_ms <= 0.0 {
                    1.0
                } else {
                    p95_latency_ms / target_max_ms
                }
            }
            Self::LedgerSyncLag {
                lag_ledgers,
                target_max_ledgers,
            } => {
                if *target_max_ledgers == 0 {
                    if *lag_ledgers > 0 {
                        2.0
                    } else {
                        1.0
                    }
                } else {
                    *lag_ledgers as f64 / *target_max_ledgers as f64
                }
            }
            Self::HorizonTxQueueDepth {
                queued_transactions,
                target_max_queue,
            } => {
                if *target_max_queue == 0 {
                    if *queued_transactions > 0 {
                        2.0
                    } else {
                        1.0
                    }
                } else {
                    *queued_transactions as f64 / *target_max_queue as f64
                }
            }
            Self::SorobanInvocationLatency {
                p95_latency_ms,
                target_max_ms,
            } => {
                if *target_max_ms <= 0.0 {
                    1.0
                } else {
                    p95_latency_ms / target_max_ms
                }
            }
            Self::FeeStatCongestionRatio {
                congestion_ratio,
                target_max_ratio,
            } => {
                if *target_max_ratio <= 0.0 {
                    1.0
                } else {
                    congestion_ratio / target_max_ratio
                }
            }
            Self::TransactionThroughputTps {
                current_tps,
                target_tps_per_replica,
            } => {
                let capacity = replicas * target_tps_per_replica;
                if capacity <= 0.0 {
                    1.0
                } else {
                    current_tps / capacity
                }
            }
        }
    }
}

/// Point-in-time SLI measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BusinessSliMeasurement {
    pub metric: BusinessSliMetric,
    pub timestamp: DateTime<Utc>,
}

/// Cost constraints configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CostBudgetConstraint {
    pub enabled: bool,
    pub monthly_budget_usd: f64,
    pub estimated_cost_per_replica_hour_usd: f64,
}

impl CostBudgetConstraint {
    /// Calculate max replicas affordable within monthly budget (assuming 730 hours/month).
    pub fn max_affordable_replicas(&self) -> i32 {
        if !self.enabled || self.estimated_cost_per_replica_hour_usd <= 0.0 {
            return i32::MAX;
        }
        let monthly_cost_per_replica = self.estimated_cost_per_replica_hour_usd * 730.0;
        let affordable = (self.monthly_budget_usd / monthly_cost_per_replica).floor() as i32;
        affordable.max(1)
    }
}

/// Policy for the Adaptive HPA.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdaptiveHpaConfig {
    pub min_replicas: i32,
    pub max_replicas: i32,
    /// EWMA smoothing alpha parameter [0.0 - 1.0].
    pub ewma_alpha: f64,
    /// Cooldown window for scale-up in seconds (default 15s for responsive protection).
    pub scale_up_cooldown_secs: u64,
    /// Stabilization window for scale-down in seconds (default 300s to avoid flapping).
    pub scale_down_stabilization_secs: u64,
    /// Proactive headroom multiplier on scale-up (e.g. 1.15 = 15% extra headroom).
    pub scale_up_headroom_factor: f64,
    /// Maximum replica change step per evaluation cycle.
    pub max_scale_step: i32,
    /// Optional monthly cost constraints.
    pub cost_constraint: Option<CostBudgetConstraint>,
}

impl Default for AdaptiveHpaConfig {
    fn default() -> Self {
        Self {
            min_replicas: 2,
            max_replicas: 20,
            ewma_alpha: 0.3,
            scale_up_cooldown_secs: 15,
            scale_down_stabilization_secs: 300,
            scale_up_headroom_factor: 1.15,
            max_scale_step: 4,
            cost_constraint: None,
        }
    }
}

/// Direction of scaling recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalingDirection {
    ScaleUp,
    ScaleDown,
    Steady,
}

/// Outcome decision from an evaluation cycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdaptiveScalingDecision {
    pub current_replicas: i32,
    pub desired_replicas: i32,
    pub effective_replicas: i32,
    pub direction: ScalingDirection,
    pub driving_sli: String,
    pub smoothed_ratio: f64,
    pub cost_capped: bool,
    pub reason: String,
    pub evaluated_at: DateTime<Utc>,
}

/// State of exponential moving average for one metric.
#[derive(Debug, Clone, Default)]
struct SliEwma {
    smoothed_value: f64,
    initialized: bool,
}

impl SliEwma {
    fn update(&mut self, sample: f64, alpha: f64) -> f64 {
        if !self.initialized {
            self.smoothed_value = sample;
            self.initialized = true;
        } else {
            self.smoothed_value = alpha * sample + (1.0 - alpha) * self.smoothed_value;
        }
        self.smoothed_value
    }
}

/// The Adaptive HPA Engine executing real-time evaluation.
pub struct AdaptiveHpaEngine {
    config: AdaptiveHpaConfig,
    ewma_states: HashMap<String, SliEwma>,
    last_scale_time: Option<DateTime<Utc>>,
    last_scale_direction: ScalingDirection,
}

impl AdaptiveHpaEngine {
    pub fn new(config: AdaptiveHpaConfig) -> Self {
        Self {
            config,
            ewma_states: HashMap::new(),
            last_scale_time: None,
            last_scale_direction: ScalingDirection::Steady,
        }
    }

    /// Evaluate current SLI measurements and compute target replica recommendation.
    pub fn evaluate(
        &mut self,
        current_replicas: i32,
        measurements: &[BusinessSliMeasurement],
        now: DateTime<Utc>,
    ) -> AdaptiveScalingDecision {
        let current = current_replicas.max(1);

        if measurements.is_empty() {
            return AdaptiveScalingDecision {
                current_replicas: current,
                desired_replicas: current,
                effective_replicas: current,
                direction: ScalingDirection::Steady,
                driving_sli: "none".to_string(),
                smoothed_ratio: 1.0,
                cost_capped: false,
                reason: "No SLI measurements provided".to_string(),
                evaluated_at: now,
            };
        }

        // 1. Compute smoothed ratios for all SLIs, finding the dominant bottleneck
        let mut max_smoothed_ratio = 1.0;
        let mut driving_sli = "balanced".to_string();

        for m in measurements {
            let name = m.metric.name().to_string();
            let raw_ratio = m.metric.scaling_ratio(current);
            let state = self.ewma_states.entry(name.clone()).or_default();
            let smoothed = state.update(raw_ratio, self.config.ewma_alpha);

            if smoothed > max_smoothed_ratio || (max_smoothed_ratio == 1.0 && smoothed != 1.0) {
                max_smoothed_ratio = smoothed;
                driving_sli = name;
            }
        }

        // 2. Compute raw desired replicas from dominant ratio
        let raw_desired = if max_smoothed_ratio > 1.0 {
            // Scale-up: apply proactive headroom
            ((current as f64) * max_smoothed_ratio * self.config.scale_up_headroom_factor).ceil()
                as i32
        } else if max_smoothed_ratio < 0.85 {
            // Scale-down: conservative target
            ((current as f64) * max_smoothed_ratio).floor() as i32
        } else {
            current
        };

        // 3. Stabilization & cooldown gating
        let (desired_with_cooldown, direction, reason) = if raw_desired > current {
            let in_cooldown = self.last_scale_time.map_or(false, |last| {
                (now - last).num_seconds() < self.config.scale_up_cooldown_secs as i64
                    && self.last_scale_direction == ScalingDirection::ScaleUp
            });

            if in_cooldown {
                (
                    current,
                    ScalingDirection::Steady,
                    format!("Scale-up suppressed by cooldown ({driving_sli} ratio={max_smoothed_ratio:.2})"),
                )
            } else {
                let bounded_step = (raw_desired - current).min(self.config.max_scale_step);
                let target = current + bounded_step;
                (
                    target,
                    ScalingDirection::ScaleUp,
                    format!(
                        "Scale-up driven by {driving_sli} (smoothed ratio={max_smoothed_ratio:.2})"
                    ),
                )
            }
        } else if raw_desired < current {
            let in_stabilization = self.last_scale_time.map_or(false, |last| {
                (now - last).num_seconds() < self.config.scale_down_stabilization_secs as i64
            });

            if in_stabilization {
                (
                    current,
                    ScalingDirection::Steady,
                    format!("Scale-down held during stabilization window ({driving_sli} ratio={max_smoothed_ratio:.2})"),
                )
            } else {
                let bounded_step = (current - raw_desired).min(self.config.max_scale_step);
                let target = current - bounded_step;
                (
                    target,
                    ScalingDirection::ScaleDown,
                    format!("Scale-down opportunity for {driving_sli} (smoothed ratio={max_smoothed_ratio:.2})"),
                )
            }
        } else {
            (
                current,
                ScalingDirection::Steady,
                format!("Workload SLIs optimal ({driving_sli} ratio={max_smoothed_ratio:.2})"),
            )
        };

        // 4. Clamp between configured Min and Max replicas
        let clamped = desired_with_cooldown
            .max(self.config.min_replicas)
            .min(self.config.max_replicas);

        // 5. Apply monthly cost budget constraint
        let mut cost_capped = false;
        let final_replicas = if let Some(ref cost) = self.config.cost_constraint {
            let cost_limit = cost.max_affordable_replicas();
            if clamped > cost_limit {
                cost_capped = true;
                warn!(
                    requested = clamped,
                    capped_to = cost_limit,
                    monthly_budget = cost.monthly_budget_usd,
                    "Autoscaling replica count capped by monthly cost budget"
                );
                cost_limit.max(self.config.min_replicas)
            } else {
                clamped
            }
        } else {
            clamped
        };

        if final_replicas != current {
            self.last_scale_time = Some(now);
            self.last_scale_direction = direction;
        }

        AdaptiveScalingDecision {
            current_replicas: current,
            desired_replicas: desired_with_cooldown,
            effective_replicas: final_replicas,
            direction,
            driving_sli,
            smoothed_ratio: max_smoothed_ratio,
            cost_capped,
            reason,
            evaluated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_up_on_ledger_close_latency() {
        let config = AdaptiveHpaConfig {
            min_replicas: 2,
            max_replicas: 10,
            ewma_alpha: 1.0, // Instant response for testing
            ..Default::default()
        };
        let mut engine = AdaptiveHpaEngine::new(config);
        let now = Utc::now();

        // Target is 5000ms; observed is 10000ms -> ratio = 2.0
        let measurements = vec![BusinessSliMeasurement {
            metric: BusinessSliMetric::LedgerCloseLatencyMillis {
                p95_latency_ms: 10000.0,
                target_max_ms: 5000.0,
            },
            timestamp: now,
        }];

        let decision = engine.evaluate(2, &measurements, now);
        assert_eq!(decision.direction, ScalingDirection::ScaleUp);
        assert!(decision.effective_replicas > 2);
        assert_eq!(decision.driving_sli, "LedgerCloseLatencyMillis");
    }

    #[test]
    fn test_scale_up_on_horizon_queue_depth() {
        let config = AdaptiveHpaConfig {
            min_replicas: 1,
            max_replicas: 8,
            ewma_alpha: 1.0,
            ..Default::default()
        };
        let mut engine = AdaptiveHpaEngine::new(config);
        let now = Utc::now();

        // Target queue is 200, observed 600
        let measurements = vec![BusinessSliMeasurement {
            metric: BusinessSliMetric::HorizonTxQueueDepth {
                queued_transactions: 600,
                target_max_queue: 200,
            },
            timestamp: now,
        }];

        let decision = engine.evaluate(2, &measurements, now);
        assert_eq!(decision.direction, ScalingDirection::ScaleUp);
        assert_eq!(decision.driving_sli, "HorizonTxQueueDepth");
    }

    #[test]
    fn test_cost_budget_capping() {
        let config = AdaptiveHpaConfig {
            min_replicas: 2,
            max_replicas: 50,
            ewma_alpha: 1.0,
            cost_constraint: Some(CostBudgetConstraint {
                enabled: true,
                monthly_budget_usd: 500.0,
                estimated_cost_per_replica_hour_usd: 0.20, // $146/mo per replica -> max 3 replicas
            }),
            ..Default::default()
        };
        let mut engine = AdaptiveHpaEngine::new(config);
        let now = Utc::now();

        let measurements = vec![BusinessSliMeasurement {
            metric: BusinessSliMetric::TransactionThroughputTps {
                current_tps: 5000.0,
                target_tps_per_replica: 100.0,
            },
            timestamp: now,
        }];

        let decision = engine.evaluate(2, &measurements, now);
        assert!(decision.cost_capped);
        assert_eq!(decision.effective_replicas, 3);
    }

    #[test]
    fn test_stabilization_window_prevents_scale_down_flapping() {
        let config = AdaptiveHpaConfig {
            min_replicas: 2,
            max_replicas: 10,
            ewma_alpha: 1.0,
            scale_down_stabilization_secs: 300,
            ..Default::default()
        };
        let mut engine = AdaptiveHpaEngine::new(config);
        let t0 = Utc::now();

        // 1. Initial Scale-Up
        let scale_up_measurement = vec![BusinessSliMeasurement {
            metric: BusinessSliMetric::HorizonTxQueueDepth {
                queued_transactions: 1000,
                target_max_queue: 200,
            },
            timestamp: t0,
        }];
        let d1 = engine.evaluate(2, &scale_up_measurement, t0);
        assert_eq!(d1.direction, ScalingDirection::ScaleUp);

        // 2. Load drops 30s later (should NOT scale down immediately because of 300s window)
        let t1 = t0 + chrono::Duration::seconds(30);
        let idle_measurement = vec![BusinessSliMeasurement {
            metric: BusinessSliMetric::HorizonTxQueueDepth {
                queued_transactions: 10,
                target_max_queue: 200,
            },
            timestamp: t1,
        }];
        let d2 = engine.evaluate(d1.effective_replicas, &idle_measurement, t1);
        assert_eq!(d2.direction, ScalingDirection::Steady);
        assert_eq!(d2.effective_replicas, d1.effective_replicas);

        // 3. 400s later (after stabilization window), scale down is permitted
        let t2 = t0 + chrono::Duration::seconds(400);
        let d3 = engine.evaluate(d1.effective_replicas, &idle_measurement, t2);
        assert_eq!(d3.direction, ScalingDirection::ScaleDown);
        assert!(d3.effective_replicas < d1.effective_replicas);
    }
}
