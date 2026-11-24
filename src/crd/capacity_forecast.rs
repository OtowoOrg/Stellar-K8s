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
//! `CapacityRecommendationReport` CRD (#1493)
//!
//! One report is published per recommendation cycle by the capacity
//! forecasting engine ([`crate::capacity_planning::exhaustion`]). It carries
//! the forecasts (each with a confidence interval), the scaling
//! recommendations ranked by time-to-exhaustion, and the backtest that
//! justifies them, stamped with the model version that produced it.
//!
//! Automation consumes the `spec` and reports progress through `status`.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Output of a single capacity recommendation cycle.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "CapacityRecommendationReport",
    namespaced,
    status = "CapacityRecommendationReportStatus",
    shortname = "caprec",
    printcolumn = r#"{"name":"Cycle","type":"string","jsonPath":".spec.cycleId"}"#,
    printcolumn = r#"{"name":"Model","type":"string","jsonPath":".spec.modelVersion"}"#,
    printcolumn = r#"{"name":"MAPE","type":"number","jsonPath":".spec.backtest.mapePct"}"#,
    printcolumn = r#"{"name":"NextExhaustionDays","type":"integer","jsonPath":".spec.recommendations[0].timeToExhaustion.earliestDays"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct CapacityRecommendationReportSpec {
    /// Identifier of the recommendation cycle (e.g. `2026-Q4`).
    pub cycle_id: String,
    /// RFC 3339 timestamp at which the cycle ran.
    pub generated_at: String,
    /// Version of the forecasting model that produced this report.
    pub model_version: String,
    /// Forecast horizon in days.
    pub horizon_days: u32,
    /// Two-sided confidence level of every interval in this report (e.g. 0.9).
    pub confidence_level: f64,
    /// Forecast for every analysed series, whether or not it needs action.
    #[serde(default)]
    pub forecasts: Vec<CapacityForecastSummary>,
    /// Scaling recommendations, ranked by time-to-exhaustion (rank 1 first).
    #[serde(default)]
    pub recommendations: Vec<ScalingRecommendation>,
    /// Backtest of the model version published alongside the recommendations.
    pub backtest: BacktestReport,
}

/// Capacity dimension, forecast independently of the others.
#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(rename_all = "PascalCase")]
pub enum CapacityDimension {
    Cpu,
    Memory,
    Storage,
    ObjectCount,
}

/// Model used to produce a forecast.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ForecastModelKind {
    /// Theil–Sen robust linear trend.
    RobustLinear,
    /// Robust linear trend plus an additive periodic (weekly) component.
    SeasonalLinear,
}

/// Point estimate with its confidence interval.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ForecastInterval {
    pub point: f64,
    pub lower: f64,
    pub upper: f64,
}

/// Days until usage reaches capacity. `None` means not within the horizon.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimeToExhaustion {
    /// Pessimistic bound: the upper forecast band crosses capacity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub earliest_days: Option<u32>,
    /// The point forecast crosses capacity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_days: Option<u32>,
    /// Optimistic bound: the lower forecast band crosses capacity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_days: Option<u32>,
}

/// Forecast for one (cluster, dimension) series.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapacityForecastSummary {
    pub cluster: String,
    pub dimension: CapacityDimension,
    /// Object kind for `ObjectCount` series (e.g. `pods`, `secrets`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_kind: Option<String>,
    pub model: ForecastModelKind,
    pub current_usage: f64,
    pub current_capacity: f64,
    /// Forecast usage at the end of the horizon.
    pub at_horizon: ForecastInterval,
    pub time_to_exhaustion: TimeToExhaustion,
}

/// Urgency bucket derived from the pessimistic time-to-exhaustion.
#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "PascalCase")]
pub enum RecommendationPriority {
    Critical,
    High,
    Medium,
    Low,
}

/// A single ranked scaling recommendation.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScalingRecommendation {
    /// 1-based rank; lower means capacity runs out sooner.
    pub rank: u32,
    pub cluster: String,
    pub dimension: CapacityDimension,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_kind: Option<String>,
    pub priority: RecommendationPriority,
    pub current_capacity: f64,
    /// Capacity that keeps the upper forecast band under the target
    /// utilisation for the whole horizon.
    pub recommended_capacity: f64,
    pub time_to_exhaustion: TimeToExhaustion,
    pub forecast_at_horizon: ForecastInterval,
    /// RFC 3339 deadline to act while keeping the required lead time.
    pub act_by: String,
    pub model: ForecastModelKind,
    pub rationale: String,
}

/// Accuracy of the model version, published with every cycle.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BacktestReport {
    pub model_version: String,
    pub horizon_days: u32,
    /// Longest history (in days) used across evaluated series.
    pub history_days: u32,
    /// Number of rolling-origin forecasts scored at the horizon.
    pub evaluated_forecasts: u32,
    /// Pooled MAPE at the horizon; absent if nothing could be scored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mape_pct: Option<f64>,
    /// Fraction of actuals that fell inside the confidence interval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_coverage: Option<f64>,
    pub mape_target_pct: f64,
    pub mape_target_met: bool,
    pub min_lead_time_days: u32,
    /// True when every historical incident was predicted with enough lead.
    pub lead_time_target_met: bool,
    #[serde(default)]
    pub series: Vec<SeriesBacktest>,
    #[serde(default)]
    pub incidents: Vec<IncidentBacktest>,
}

/// Backtest detail for one series, including the model-selection evidence.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesBacktest {
    pub cluster: String,
    pub dimension: CapacityDimension,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_kind: Option<String>,
    pub selected_model: ForecastModelKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linear_mape_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seasonal_mape_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_coverage: Option<f64>,
    pub samples: u32,
}

/// Whether a historical capacity incident would have been predicted in time.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IncidentBacktest {
    pub cluster: String,
    pub dimension: CapacityDimension,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_kind: Option<String>,
    pub occurred_at: String,
    /// Largest lead (days before the incident) at which the forecast warned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_time_days: Option<u32>,
    pub met: bool,
}

/// Lifecycle of a report as driven by downstream automation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum RecommendationPhase {
    #[default]
    Pending,
    Acknowledged,
    Applied,
    Superseded,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapacityRecommendationReportStatus {
    #[serde(default)]
    pub phase: RecommendationPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
}
