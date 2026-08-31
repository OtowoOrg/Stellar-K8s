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
//! StellarObservability CRD for comprehensive observability platform

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Comprehensive observability platform configuration
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarObservability",
    namespaced,
    status = "StellarObservabilityStatus",
    shortname = "sobs"
)]
#[serde(rename_all = "camelCase")]
pub struct StellarObservabilitySpec {
    /// Target StellarNode name
    pub target_node: String,
    /// Distributed tracing configuration
    pub tracing: TracingConfig,
    /// Log aggregation setup
    pub logging: LoggingConfig,
    /// Alerting rules
    pub alerting: AlertingConfig,
    /// Anomaly detection
    pub anomaly_detection: AnomalyDetectionConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TracingConfig {
    pub enabled: bool,
    pub backend: TracingBackend,
    pub sample_rate: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum TracingBackend {
    Jaeger,
    Zipkin,
    OpenTelemetry,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoggingConfig {
    pub enabled: bool,
    pub backend: LoggingBackend,
    pub log_level: String,
    pub retention_days: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum LoggingBackend {
    Loki,
    Elasticsearch,
    Stackdriver,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AlertingConfig {
    pub enabled: bool,
    pub alert_rules: Vec<AlertRule>,
    pub fatigue_reduction: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AlertRule {
    pub name: String,
    pub condition: String,
    pub threshold: f64,
    pub duration_secs: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyDetectionConfig {
    pub enabled: bool,
    pub model_type: AnomalyModel,
    pub sensitivity: AnomalySensitivity,
    pub baseline_learning_days: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum AnomalyModel {
    IsolationForest,
    LocalOutlierFactor,
    AutoEncoder,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum AnomalySensitivity {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct StellarObservabilityStatus {
    pub phase: String,
    pub tracing_enabled: bool,
    pub logging_enabled: bool,
    pub alerts_configured: u32,
    pub anomalies_detected: u32,
    pub dashboard_ready: bool,
}
