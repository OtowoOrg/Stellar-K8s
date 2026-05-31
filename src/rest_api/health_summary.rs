//! Health Check API for Status Pages
//!
//! Exposes a public-facing (or internal-facing) Health API that provides a high-level
//! summary of the entire fleet's status, suitable for integration with StatusPage.io.
//!
//! Endpoints:
//! - GET /v1/health/summary - Fleet-wide health summary
//! - GET /v1/health/nodes - Per-node health status
//! - GET /v1/health/incidents - Active incidents

use crate::controller::ControllerState;
use crate::crd::StellarNode;
use axum::{extract::State, http::StatusCode, Json};
use kube::api::Api;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, instrument};

/// Overall health status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::Unhealthy => write!(f, "unhealthy"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Per-node health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHealthStatus {
    pub name: String,
    pub namespace: String,
    pub node_type: String,
    pub status: HealthStatus,
    pub synced: bool,
    pub ledger_sequence: Option<u64>,
    pub api_latency_ms: Option<u32>,
    pub peer_count: Option<u32>,
    pub last_check_time: i64,
}

/// Fleet-wide health summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetHealthSummary {
    pub overall_status: HealthStatus,
    pub timestamp: i64,
    pub total_nodes: u32,
    pub healthy_nodes: u32,
    pub synced_validators: u32,
    pub total_validators: u32,
    pub average_api_latency_ms: Option<u32>,
    pub active_incidents: u32,
    pub nodes: Vec<NodeHealthStatus>,
}

/// Active incident
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthIncident {
    pub id: String,
    pub severity: String,
    pub title: String,
    pub description: String,
    pub affected_nodes: Vec<String>,
    pub start_time: i64,
    pub end_time: Option<i64>,
    pub status: String,
}

/// Health incidents response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthIncidentsResponse {
    pub incidents: Vec<HealthIncident>,
    pub total_active: u32,
    pub timestamp: i64,
}

/// Get fleet-wide health summary
#[instrument(skip(state))]
pub async fn get_health_summary(
    State(state): State<Arc<ControllerState>>,
) -> Result<Json<FleetHealthSummary>, (StatusCode, String)> {
    debug!("Fetching fleet health summary");

    let api: Api<StellarNode> = Api::all(state.client.clone());
    let nodes = api
        .list(&Default::default())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut node_statuses = Vec::new();
    let mut healthy_count = 0;
    let mut synced_validators = 0;
    let mut total_validators = 0;

    for node in nodes.items {
        let node_name = node.metadata.name.clone().unwrap_or_default();
        let namespace = node
            .metadata
            .namespace
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let node_type = format!("{:?}", node.spec.node_type);

        // Determine health status based on node conditions
        let status = if let Some(status) = &node.status {
            if !status.conditions.is_empty() {
                let is_ready = status
                    .conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True");
                let is_synced = status
                    .conditions
                    .iter()
                    .any(|c| c.type_ == "Synced" && c.status == "True");

                if is_ready && is_synced {
                    healthy_count += 1;
                    HealthStatus::Healthy
                } else if is_ready {
                    HealthStatus::Degraded
                } else {
                    HealthStatus::Unhealthy
                }
            } else {
                HealthStatus::Unknown
            }
        } else {
            HealthStatus::Unknown
        };

        // Track validator sync status
        if node_type.contains("Validator") {
            total_validators += 1;
            if status == HealthStatus::Healthy {
                synced_validators += 1;
            }
        }

        // Extract metrics from status
        let synced = status == HealthStatus::Healthy;
        let ledger_sequence = node.status.as_ref().and_then(|s| s.ledger_sequence);

        node_statuses.push(NodeHealthStatus {
            name: node_name,
            namespace,
            node_type,
            status,
            synced,
            ledger_sequence,
            api_latency_ms: None,
            peer_count: None,
            last_check_time: chrono::Utc::now().timestamp(),
        });
    }

    let total_nodes = node_statuses.len() as u32;

    // Determine overall status
    let overall_status = if healthy_count == total_nodes && total_nodes > 0 {
        HealthStatus::Healthy
    } else if healthy_count > 0 {
        HealthStatus::Degraded
    } else if total_nodes > 0 {
        HealthStatus::Unhealthy
    } else {
        HealthStatus::Unknown
    };

    let summary = FleetHealthSummary {
        overall_status,
        timestamp: chrono::Utc::now().timestamp(),
        total_nodes,
        healthy_nodes: healthy_count,
        synced_validators,
        total_validators,
        average_api_latency_ms: None,
        active_incidents: 0, // Would be populated from incident tracking system
        nodes: node_statuses,
    };

    Ok(Json(summary))
}

/// Get per-node health status
#[instrument(skip(state))]
pub async fn get_node_health_status(
    State(state): State<Arc<ControllerState>>,
) -> Result<Json<Vec<NodeHealthStatus>>, (StatusCode, String)> {
    debug!("Fetching per-node health status");

    let api: Api<StellarNode> = Api::all(state.client.clone());
    let nodes = api
        .list(&Default::default())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut node_statuses = Vec::new();

    for node in nodes.items {
        let node_name = node.metadata.name.clone().unwrap_or_default();
        let namespace = node
            .metadata
            .namespace
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let node_type = format!("{:?}", node.spec.node_type);

        let status = if let Some(status) = &node.status {
            if !status.conditions.is_empty() {
                let is_ready = status
                    .conditions
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True");
                let is_synced = status
                    .conditions
                    .iter()
                    .any(|c| c.type_ == "Synced" && c.status == "True");

                if is_ready && is_synced {
                    HealthStatus::Healthy
                } else if is_ready {
                    HealthStatus::Degraded
                } else {
                    HealthStatus::Unhealthy
                }
            } else {
                HealthStatus::Unknown
            }
        } else {
            HealthStatus::Unknown
        };

        node_statuses.push(NodeHealthStatus {
            name: node_name,
            namespace,
            node_type,
            status: status.clone(),
            synced: status == HealthStatus::Healthy,
            ledger_sequence: node.status.as_ref().and_then(|s| s.ledger_sequence),
            api_latency_ms: None,
            peer_count: None,
            last_check_time: chrono::Utc::now().timestamp(),
        });
    }

    Ok(Json(node_statuses))
}

/// Get active incidents
#[instrument(skip(_state))]
pub async fn get_health_incidents(
    State(_state): State<Arc<ControllerState>>,
) -> Result<Json<HealthIncidentsResponse>, (StatusCode, String)> {
    debug!("Fetching active health incidents");

    // In a real implementation, this would query an incident tracking system
    // For now, we'll return an empty list
    let incidents = Vec::new();

    let response = HealthIncidentsResponse {
        incidents,
        total_active: 0,
        timestamp: chrono::Utc::now().timestamp(),
    };

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_status_display() {
        assert_eq!(HealthStatus::Healthy.to_string(), "healthy");
        assert_eq!(HealthStatus::Degraded.to_string(), "degraded");
        assert_eq!(HealthStatus::Unhealthy.to_string(), "unhealthy");
    }

    #[test]
    fn test_node_health_status_creation() {
        let status = NodeHealthStatus {
            name: "test-node".to_string(),
            namespace: "default".to_string(),
            node_type: "Validator".to_string(),
            status: HealthStatus::Healthy,
            synced: true,
            ledger_sequence: Some(12345),
            api_latency_ms: Some(50),
            peer_count: Some(10),
            last_check_time: chrono::Utc::now().timestamp(),
        };

        assert_eq!(status.name, "test-node");
        assert_eq!(status.status, HealthStatus::Healthy);
        assert!(status.synced);
    }

    #[test]
    fn test_fleet_health_summary_creation() {
        let summary = FleetHealthSummary {
            overall_status: HealthStatus::Healthy,
            timestamp: chrono::Utc::now().timestamp(),
            total_nodes: 5,
            healthy_nodes: 5,
            synced_validators: 3,
            total_validators: 3,
            average_api_latency_ms: Some(50),
            active_incidents: 0,
            nodes: Vec::new(),
        };

        assert_eq!(summary.overall_status, HealthStatus::Healthy);
        assert_eq!(summary.total_nodes, 5);
        assert_eq!(summary.healthy_nodes, 5);
    }
}
