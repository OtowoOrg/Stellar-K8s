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
//! Incident Custom Resource Definition (#1580)
//!
//! Declarative management for network partition and consensus anomaly incidents,
//! tracking automated alerting, chronological timeline events, and quorum adjustment recommendations.

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Custom Resource Definition for Network & Consensus Incidents
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "incident.stellar.org",
    version = "v1alpha1",
    kind = "Incident",
    namespaced,
    status = "IncidentStatus",
    shortname = "inc",
    printcolumn = r#"{"name":"Title","type":"string","jsonPath":".spec.title"}"#,
    printcolumn = r#"{"name":"Type","type":"string","jsonPath":".spec.incidentType"}"#,
    printcolumn = r#"{"name":"Severity","type":"string","jsonPath":".spec.severity"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"MissedCloses","type":"integer","jsonPath":".status.partitionDetails.missedLedgerCloses"}"#,
    printcolumn = r#"{"name":"DetectedAt","type":"date","jsonPath":".status.detectedAt"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct IncidentSpec {
    /// Human-readable title of the incident.
    pub title: String,

    /// Nature of incident (NetworkPartition, LedgerStall, QuorumDegradation).
    pub incident_type: IncidentType,

    /// Urgency and impact level.
    pub severity: IncidentSeverity,

    /// List of affected validator or Horizon node names.
    #[serde(default)]
    pub affected_nodes: Vec<String>,

    /// Alert dispatch targets (Slack, PagerDuty, Webhooks).
    #[serde(default)]
    pub alert_channels: Vec<AlertChannelConfig>,

    /// Whether the operator is permitted to automatically apply recommended quorum adjustments.
    #[serde(default)]
    pub auto_remediation_enabled: bool,
}

/// Incident categories
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum IncidentType {
    NetworkPartition,
    LedgerStall,
    QuorumDegradation,
    ByzantineFault,
    Custom(String),
}

/// Severity classification
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum IncidentSeverity {
    Critical,
    High,
    Medium,
    Low,
}

/// Destination channel configuration for incident notifications
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlertChannelConfig {
    pub name: String,
    pub channel_type: AlertChannelType,
    pub endpoint_url: String,
    #[serde(default)]
    pub routing_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum AlertChannelType {
    Webhook,
    Slack,
    PagerDuty,
    OpsGenie,
}

/// Current state of the incident
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "PascalCase")]
pub enum IncidentPhase {
    #[default]
    Detected,
    Escalated,
    Mitigating,
    Resolved,
    PostMortem,
}

/// Observed incident status
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IncidentStatus {
    /// Current lifecycle phase.
    pub phase: IncidentPhase,

    /// Initial detection timestamp.
    pub detected_at: DateTime<Utc>,

    /// Resolution timestamp if resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,

    /// Diagnostic metrics regarding the network partition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition_details: Option<PartitionDetails>,

    /// Automated quorum adjustment suggestion for partition tolerance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quorum_adjustment_recommendation: Option<QuorumAdjustmentRecommendation>,

    /// Chronological timeline of automated response actions and diagnostic events.
    #[serde(default)]
    pub timeline: Vec<IncidentTimelineEntry>,

    /// History of alerts sent to configured channels.
    #[serde(default)]
    pub alert_dispatches: Vec<AlertDispatchResult>,
}

/// Partition diagnostics
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PartitionDetails {
    /// Number of consecutive missed ledger closes detected.
    pub missed_ledger_closes: u32,

    /// Expected ledger close interval in seconds (typically ~5s).
    pub expected_close_time_secs: u32,

    /// Last ledger sequence number committed before stall.
    pub last_closed_ledger: u64,

    /// Duration of consensus stall in seconds.
    pub stall_duration_secs: u64,

    /// Unreachable validator peer nodes.
    #[serde(default)]
    pub unreachable_peers: Vec<String>,

    /// Peer nodes that remain reachable within current partition.
    #[serde(default)]
    pub reachable_peers: Vec<String>,

    /// Percentage of configured quorum set currently reachable.
    pub quorum_health_pct: f64,
}

/// Quorum set adjustment recommendation for partition tolerance
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuorumAdjustmentRecommendation {
    /// Safe adjusted validator set excluding partitioned nodes.
    pub suggested_quorum_set: Vec<String>,

    /// New recommended threshold to regain consensus liveness without safety compromise.
    pub new_threshold: u32,

    /// Technical explanation and safety proof rationale.
    pub rationale: String,

    /// Recommended operator steps or automated actions.
    #[serde(default)]
    pub action_plan: Vec<String>,
}

/// Individual chronological entry in the incident timeline
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IncidentTimelineEntry {
    /// Exact event timestamp.
    pub timestamp: DateTime<Utc>,

    /// Lifecycle stage or source component.
    pub stage: String,

    /// Descriptive summary of detection, alerting, or remediation action.
    pub message: String,

    /// Optional structured metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Alert delivery record
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlertDispatchResult {
    pub channel_name: String,
    pub dispatched_at: DateTime<Utc>,
    pub success: bool,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_message: Option<String>,
}
