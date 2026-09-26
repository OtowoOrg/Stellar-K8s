// Copyright 2026 Stellar-K8s Contributors
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
//! Validator Performance Scoring and Leaderboard CRDs (#1579)
//!
//! Provides automated hourly performance scoring (uptime, ledger close participation,
//! and archive completeness) and network-wide leaderboard aggregation across federation members.

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Custom Resource for per-validator performance scoring.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "ValidatorScore",
    namespaced,
    status = "ValidatorScoreStatus",
    shortname = "vscore",
    printcolumn = r#"{"name":"Validator","type":"string","jsonPath":".spec.validatorRef"}"#,
    printcolumn = r#"{"name":"Grade","type":"string","jsonPath":".status.grade"}"#,
    printcolumn = r#"{"name":"CompositeScore","type":"number","jsonPath":".status.compositeScore"}"#,
    printcolumn = r#"{"name":"UptimeScore","type":"number","jsonPath":".status.uptimeScore.score"}"#,
    printcolumn = r#"{"name":"ConsensusRate","type":"number","jsonPath":".status.consensusScore.score"}"#,
    printcolumn = r#"{"name":"ArchiveScore","type":"number","jsonPath":".status.archiveScore.score"}"#,
    printcolumn = r#"{"name":"LastEvaluated","type":"date","jsonPath":".status.lastEvaluatedAt"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorScoreSpec {
    /// Name of the target StellarNode validator.
    pub validator_ref: String,

    /// How frequently to recalculate scores in hours (default: 1 hour).
    #[serde(default = "default_eval_hours")]
    pub evaluation_interval_hours: u32,

    /// Weights assigned to scoring components.
    #[serde(default)]
    pub weights: ScoringWeights,

    /// Rolling evaluation window in hours (default: 24).
    #[serde(default = "default_window_hours")]
    pub history_window_hours: u32,

    /// Optional federation cluster identifier if part of a multi-cluster federation.
    #[serde(default)]
    pub federation_member: Option<String>,
}

fn default_eval_hours() -> u32 {
    1
}

fn default_window_hours() -> u32 {
    24
}

/// Relative weights for composite score computation (must sum to ~1.0).
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScoringWeights {
    /// Weight for /info uptime availability (default: 0.40).
    #[serde(default = "default_uptime_weight")]
    pub uptime_weight: f32,

    /// Weight for consensus participation from SCP metrics (default: 0.40).
    #[serde(default = "default_consensus_weight")]
    pub consensus_weight: f32,

    /// Weight for history archive completeness (default: 0.20).
    #[serde(default = "default_archive_weight")]
    pub archive_weight: f32,
}

fn default_uptime_weight() -> f32 {
    0.40
}
fn default_consensus_weight() -> f32 {
    0.40
}
fn default_archive_weight() -> f32 {
    0.20
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            uptime_weight: default_uptime_weight(),
            consensus_weight: default_consensus_weight(),
            archive_weight: default_archive_weight(),
        }
    }
}

/// Performance letter grade based on composite scoring.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
pub enum PerformanceGrade {
    #[default]
    Pending,
    #[serde(rename = "A+")]
    APlus,
    A,
    B,
    C,
    D,
    F,
}

impl PerformanceGrade {
    pub fn from_score(score: f64) -> Self {
        if score >= 98.0 {
            PerformanceGrade::APlus
        } else if score >= 90.0 {
            PerformanceGrade::A
        } else if score >= 80.0 {
            PerformanceGrade::B
        } else if score >= 70.0 {
            PerformanceGrade::C
        } else if score >= 60.0 {
            PerformanceGrade::D
        } else {
            PerformanceGrade::F
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PerformanceGrade::Pending => "Pending",
            PerformanceGrade::APlus => "A+",
            PerformanceGrade::A => "A",
            PerformanceGrade::B => "B",
            PerformanceGrade::C => "C",
            PerformanceGrade::D => "D",
            PerformanceGrade::F => "F",
        }
    }
}

/// Detailed score for an individual component.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComponentScore {
    /// Normalized score (0.0 to 100.0).
    pub score: f64,

    /// Raw measured metric value (e.g. 99.98% uptime).
    pub raw_value: f64,

    /// Component grade letter (e.g., "A", "B").
    pub grade: String,

    /// Contextual explanation or description.
    pub details: String,
}

/// Individual sample in hourly score history.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HourlyScoreSample {
    pub timestamp: DateTime<Utc>,
    pub composite_score: f64,
    pub uptime_pct: f64,
    pub consensus_rate: f64,
    pub archive_completeness_pct: f64,
}

/// Observed status of a validator's performance score.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorScoreStatus {
    /// Timestamp of most recent evaluation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_evaluated_at: Option<DateTime<Utc>>,

    /// Final weighted composite score (0.0 to 100.0).
    pub composite_score: f64,

    /// Letter grade (A+, A, B, C, D, F).
    pub grade: String,

    /// Score derived from /info endpoint availability (>99% = A).
    pub uptime_score: ComponentScore,

    /// Score derived from SCP consensus nomination & close participation.
    pub consensus_score: ComponentScore,

    /// Score derived from history archive completeness and catchup lag.
    pub archive_score: ComponentScore,

    /// Federation member identifier if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub federation_member: Option<String>,

    /// Hourly rolling history samples (up to 24 or 168 hours).
    #[serde(default)]
    pub hourly_history: Vec<HourlyScoreSample>,
}

/// Custom Resource for aggregating validator scores across federation members.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "ValidatorLeaderboard",
    namespaced,
    status = "ValidatorLeaderboardStatus",
    shortname = "vboard",
    printcolumn = r#"{"name":"TotalValidators","type":"integer","jsonPath":".status.totalValidators"}"#,
    printcolumn = r#"{"name":"MedianScore","type":"number","jsonPath":".status.medianScore"}"#,
    printcolumn = r#"{"name":"NetworkHealth","type":"number","jsonPath":".status.networkHealthIndex"}"#,
    printcolumn = r#"{"name":"LastRefreshed","type":"date","jsonPath":".status.lastAggregatedAt"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorLeaderboardSpec {
    /// Optional federation CR reference to pull multi-region members from.
    #[serde(default)]
    pub federation_ref: Option<String>,

    /// Leaderboard refresh interval in seconds (default: 3600 = 1 hour).
    #[serde(default = "default_leaderboard_refresh")]
    pub refresh_interval_seconds: u32,

    /// Maximum number of ranked validators to retain in status (default: 100).
    #[serde(default = "default_top_n")]
    pub top_n: usize,
}

fn default_leaderboard_refresh() -> u32 {
    3600
}

fn default_top_n() -> usize {
    100
}

/// Ranked leaderboard entry for a single validator.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LeaderboardEntry {
    /// Rank in leaderboard (1 = highest score).
    pub rank: usize,

    /// Validator name or node identity.
    pub validator_name: String,

    /// Namespace where validator runs.
    pub namespace: String,

    /// Weighted composite score (0.0 to 100.0).
    pub composite_score: f64,

    /// Grade letter (A+, A, B, etc.).
    pub grade: String,

    /// Uptime percentage.
    pub uptime_pct: f64,

    /// SCP consensus participation rate percentage.
    pub consensus_rate: f64,

    /// History archive completeness percentage.
    pub archive_completeness_pct: f64,

    /// Region or cluster name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

/// Observed leaderboard status.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorLeaderboardStatus {
    /// Timestamp of most recent aggregation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_aggregated_at: Option<DateTime<Utc>>,

    /// Total number of evaluated validators.
    pub total_validators: usize,

    /// Median composite score across all active validators.
    pub median_score: f64,

    /// Network-wide consensus health index (0.0 to 100.0).
    pub network_health_index: f64,

    /// Ranked validator entries.
    #[serde(default)]
    pub entries: Vec<LeaderboardEntry>,
}
