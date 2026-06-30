//! StellarAutoscaler CRD for advanced predictive autoscaling

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Advanced autoscaling with custom Stellar metrics
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarAutoscaler",
    namespaced,
    status = "StellarAutoscalerStatus",
    shortname = "sas"
)]
#[serde(rename_all = "camelCase")]
pub struct StellarAutoscalerSpec {
    /// Target StellarNode name
    pub target_node: String,
    /// Autoscaling policy
    pub policy: ScalingPolicy,
    /// Custom Stellar metrics
    pub custom_metrics: Vec<StellarMetric>,
    /// Predictive scaling configuration
    pub predictive_scaling: Option<PredictiveScalingConfig>,
    /// Cost-aware scaling
    pub cost_aware: Option<CostAwareConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScalingPolicy {
    pub name: String,
    pub min_replicas: i32,
    pub max_replicas: i32,
    pub strategy: ScalingStrategy,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ScalingStrategy {
    Aggressive,
    Balanced,
    Conservative,
    Custom,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarMetric {
    pub name: String,
    pub metric_type: MetricType,
    pub threshold: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum MetricType {
    TransactionThroughput,
    LedgerLag,
    RpcQueueDepth,
    ContractInvocations,
    HistoryArchiveBacklog,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PredictiveScalingConfig {
    pub enabled: bool,
    pub prediction_window_mins: u32,
    pub model_type: PredictionModel,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum PredictionModel {
    Prophet,
    LinearRegression,
    Arima,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CostAwareConfig {
    pub enabled: bool,
    pub budget_per_month_usd: f64,
    pub prefer_spot_instances: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct StellarAutoscalerStatus {
    pub phase: String,
    pub current_replicas: i32,
    pub desired_replicas: i32,
    pub last_scaling_time: Option<String>,
    pub predicted_load: Option<f64>,
    pub cost_optimization_applied: bool,
}
