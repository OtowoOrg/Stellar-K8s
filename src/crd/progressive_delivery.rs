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
//! Progressive Delivery CRD for canary-based deployments with SLO gates

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Progressive delivery configuration with automated canary metric gates
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "ProgressiveDelivery",
    namespaced,
    status = "ProgressiveDeliveryStatus",
    shortname = "pd"
)]
#[serde(rename_all = "camelCase")]
pub struct ProgressiveDeliverySpec {
    /// Target workload name (Deployment or StatefulSet)
    pub target_workload: String,
    /// Target workload kind: Deployment or StatefulSet
    pub workload_kind: String,
    /// Target image or revision for progressive rollout
    pub target_revision: String,
    /// Weight progression strategy
    pub weight_progression: WeightProgression,
    /// SLO metric gates for canary promotion
    pub slo_gates: Vec<SloGate>,
    /// Analysis window duration in seconds
    pub analysis_window_secs: u32,
    /// Prometheus query address
    pub prometheus_address: Option<String>,
    /// Traffic splitting configuration
    pub traffic_split: Option<TrafficSplit>,
}

/// Weight progression strategy for canary promotion
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeightProgression {
    /// Linear progression: increment by fixed amount each step
    Linear { initial_weight: u32, step_size: u32 },
    /// Exponential progression: multiply weight by factor each step
    Exponential { initial_weight: u32, multiplier: f64 },
    /// Stepwise progression: jump to predefined weights
    Stepwise { steps: Vec<u32> },
}

/// SLO gate for canary promotion decision
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SloGate {
    /// Gate type: latency, error_rate, saturation, custom
    pub gate_type: String,
    /// Prometheus query for metric
    pub metric_query: String,
    /// Threshold value
    pub threshold: f64,
    /// Comparison operator: "lt", "le", "gt", "ge", "eq"
    pub operator: String,
    /// Number of consecutive violations before rollback
    pub violation_threshold: u32,
}

/// Traffic splitting configuration for canary
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrafficSplit {
    /// Traffic split mode: "service_mesh" or "service"
    pub mode: String,
    /// Service mesh type if applicable (e.g., "istio")
    pub mesh_type: Option<String>,
    /// VirtualService name for Istio
    pub virtual_service: Option<String>,
}

/// Progressive delivery status
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProgressiveDeliveryStatus {
    /// Current phase of rollout
    pub phase: PromotionPhase,
    /// Current canary weight (0-100)
    pub current_weight: u32,
    /// Current step index
    pub current_step: u32,
    /// Time of last weight update
    pub last_update_time: Option<String>,
    /// Analysis results for current step
    pub gate_results: Vec<GateResult>,
    /// Whether rollback is in progress
    pub rollback_in_progress: bool,
    /// Message describing current status
    pub message: Option<String>,
}

/// Result of a single SLO gate evaluation
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GateResult {
    /// Gate type being evaluated
    pub gate_type: String,
    /// Whether gate passed
    pub passed: bool,
    /// Current metric value
    pub metric_value: f64,
    /// Consecutive violation count
    pub violation_count: u32,
    /// Evaluation timestamp
    pub evaluated_at: Option<String>,
}

/// Promotion phase of canary rollout
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum PromotionPhase {
    #[default]
    Pending,
    AnalyzingBaseline,
    CanaryActive,
    WaitingForAnalysis,
    ReadyToPromote,
    Promoting,
    Completed,
    RollingBack,
    RolledBack,
    Failed,
}
