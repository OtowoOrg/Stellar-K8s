//! Data Transfer Objects for the Dashboard API

use serde::{Deserialize, Serialize};

use crate::crd::Condition;

/// Dashboard overview response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardOverview {
    pub total_nodes: usize,
    pub healthy_nodes: usize,
    pub syncing_nodes: usize,
    pub unhealthy_nodes: usize,
    pub nodes_by_type: NodeTypeBreakdown,
    pub nodes_by_network: NetworkBreakdown,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeTypeBreakdown {
    pub validators: usize,
    pub horizon: usize,
    pub soroban: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkBreakdown {
    pub mainnet: usize,
    pub testnet: usize,
    pub futurenet: usize,
    pub custom: usize,
}

/// Node logs response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeLogsResponse {
    pub namespace: String,
    pub name: String,
    pub pod_name: String,
    pub logs: String,
    pub timestamp: String,
}

/// Node action request
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeActionRequest {
    pub action: NodeAction,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeAction {
    Restart,
    Snapshot,
    Suspend,
    Resume,
    /// Toggle maintenance mode on the node
    MaintenanceMode,
    /// Prune old history archives for the node
    Prune,
}

/// Node action response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeActionResponse {
    pub success: bool,
    pub message: String,
    pub action: NodeAction,
}

/// Node conditions response (formatted for UI)
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeConditionsResponse {
    pub namespace: String,
    pub name: String,
    pub conditions: Vec<ConditionDisplay>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionDisplay {
    pub condition_type: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub last_transition_time: Option<String>,
    pub severity: ConditionSeverity,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConditionSeverity {
    Success,
    Warning,
    Error,
    Info,
}

impl From<&Condition> for ConditionDisplay {
    fn from(c: &Condition) -> Self {
        let severity = match c.type_.as_str() {
            "Ready" if c.status == "True" => ConditionSeverity::Success,
            "Ready" if c.status == "False" => ConditionSeverity::Error,
            "Synced" if c.status == "True" => ConditionSeverity::Success,
            "Synced" if c.status == "False" => ConditionSeverity::Warning,
            "ArchiveIntegrityDegraded" if c.status == "True" => ConditionSeverity::Warning,
            _ => ConditionSeverity::Info,
        };

        Self {
            condition_type: c.type_.clone(),
            status: c.status.clone(),
            reason: Some(c.reason.clone()),
            message: Some(c.message.clone()),
            last_transition_time: Some(c.last_transition_time.clone()),
            severity,
        }
    }
}

/// Operator logs response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorLogsResponse {
    pub logs: Vec<String>,
    pub timestamp: String,
}

/// Metrics summary for dashboard
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsSummary {
    pub namespace: String,
    pub name: String,
    pub ledger_sequence: Option<u64>,
    pub ready_replicas: i32,
    pub replicas: i32,
    pub quorum_fragility: Option<f64>,
}
