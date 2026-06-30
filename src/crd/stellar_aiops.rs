//! StellarAIOps Custom Resource Definition
//!
//! The StellarAIOps CRD enables AI-powered incident management including
//! anomaly detection, root cause analysis, and automated remediation.

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::Condition;

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarAIOps",
    namespaced,
    status = "StellarAIOpsStatus",
    shortname = "sao",
    printcolumn = r#"{"name":"Enabled","type":"boolean","jsonPath":".spec.enabled"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"Incidents","type":"integer","jsonPath":".status.incidentCount"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarAIOpsSpec {
    /// Enable AI-powered incident management
    #[serde(default)]
    pub enabled: bool,

    /// Anomaly detection configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anomaly_detection: Option<AnomalyDetectionConfig>,

    /// Root cause analysis configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_cause_analysis: Option<RootCauseAnalysisConfig>,

    /// Automated remediation configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automated_remediation: Option<AutomatedRemediationConfig>,

    /// Capacity planning configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity_planning: Option<CapacityPlanningConfig>,

    /// Predictive maintenance configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predictive_maintenance: Option<PredictiveMaintenanceConfig>,

    /// ChatOps integration configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chatops_config: Option<ChatOpsConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyDetectionConfig {
    /// Enable anomaly detection
    #[serde(default)]
    pub enabled: bool,

    /// Confidence threshold (0-1)
    #[serde(default = "default_anomaly_threshold")]
    pub threshold: f64,

    /// Detection model type
    pub model_type: Option<String>,

    /// Training data window size
    pub training_window_hours: Option<u32>,
}

fn default_anomaly_threshold() -> f64 {
    0.85
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RootCauseAnalysisConfig {
    /// Enable RCA
    #[serde(default)]
    pub enabled: bool,

    /// Confidence threshold (0-1)
    #[serde(default = "default_rca_threshold")]
    pub confidence_threshold: f64,

    /// Causal inference model enabled
    #[serde(default)]
    pub causal_inference_enabled: bool,
}

fn default_rca_threshold() -> f64 {
    0.7
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutomatedRemediationConfig {
    /// Enable automated remediation
    #[serde(default)]
    pub enabled: bool,

    /// Maximum runbook executions per incident
    #[serde(default = "default_max_runbooks")]
    pub max_runbook_executions: u32,

    /// Runbooks directory
    pub runbooks_dir: Option<String>,
}

fn default_max_runbooks() -> u32 {
    5
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapacityPlanningConfig {
    /// Enable capacity planning
    #[serde(default)]
    pub enabled: bool,

    /// Forecast horizon in days
    #[serde(default = "default_forecast_days")]
    pub forecast_horizon_days: u32,

    /// Recommendation engine enabled
    #[serde(default)]
    pub recommendation_engine: bool,
}

fn default_forecast_days() -> u32 {
    90
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PredictiveMaintenanceConfig {
    /// Enable predictive maintenance
    #[serde(default)]
    pub enabled: bool,

    /// Metrics to monitor
    #[serde(default)]
    pub monitored_metrics: Vec<String>,

    /// Failure prediction threshold
    pub prediction_threshold: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatOpsConfig {
    /// Enable ChatOps integration
    #[serde(default)]
    pub enabled: bool,

    /// Slack integration
    pub slack: Option<SlackIntegration>,

    /// Teams integration
    pub teams: Option<TeamsIntegration>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlackIntegration {
    /// Slack webhook URL secret reference
    pub webhook_secret: String,

    /// Channel to send alerts
    pub channel: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TeamsIntegration {
    /// Teams webhook URL secret reference
    pub webhook_secret: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarAIOpsStatus {
    /// Conditions of this resource
    #[serde(default)]
    pub conditions: Vec<Condition>,

    /// Current incident count
    #[serde(default)]
    pub incident_count: u32,

    /// Last analysis time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_analysis_time: Option<String>,

    /// Operational status
    pub operational_status: Option<OperationalStatus>,

    /// Average incident resolution time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_mttr: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum OperationalStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

impl Default for StellarAIOpsStatus {
    fn default() -> Self {
        Self {
            conditions: Vec::new(),
            incident_count: 0,
            last_analysis_time: None,
            operational_status: Some(OperationalStatus::Healthy),
            average_mttr: None,
        }
    }
}
