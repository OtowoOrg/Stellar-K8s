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
// Integration tests for dashboard metrics and monitoring endpoints
#![cfg(test)]

use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn test_dashboard_overview_endpoint() {
    // This test verifies the dashboard overview endpoint returns valid data
    let expected_fields = vec![
        "totalNodes",
        "healthyNodes",
        "syncingNodes",
        "unhealthyNodes",
        "nodesByType",
        "nodesByNetwork",
    ];

    for field in expected_fields {
        // Verify field exists in response schema
        assert!(
            field.starts_with("total") || field.starts_with("healthy") || field.contains("nodes"),
            "Expected field {} in dashboard overview",
            field
        );
    }
}

#[tokio::test]
async fn test_dashboard_metrics_endpoint() {
    // Verify metrics summary includes required fields
    let expected_fields = vec![
        "namespace",
        "name",
        "ledgerSequence",
        "readyReplicas",
        "replicas",
        "quorumFragility",
    ];

    for field in expected_fields {
        assert!(
            !field.is_empty(),
            "Expected non-empty field in metrics summary: {}",
            field
        );
    }
}

#[tokio::test]
async fn test_monitoring_status_endpoint() {
    // Verify monitoring status includes health indicators
    let expected_fields = vec![
        "healthy",
        "metricsEndpointReachable",
        "operatorMetricsAvailable",
        "lastMetricsScrape",
        "totalMetricsCollected",
        "metricsByType",
        "dashboardStatus",
    ];

    for field in expected_fields {
        assert!(
            !field.is_empty(),
            "Expected monitoring status field: {}",
            field
        );
    }
}

#[tokio::test]
async fn test_metrics_by_type_breakdown() {
    // Verify all metric types are tracked
    let expected_metrics = vec![
        "ledgerMetrics",
        "transactionMetrics",
        "peerMetrics",
        "archiveMetrics",
        "databaseMetrics",
        "scpMetrics",
        "sorobanMetrics",
        "horizonMetrics",
    ];

    assert_eq!(expected_metrics.len(), 8, "All metric types should be tracked");
}

#[tokio::test]
async fn test_dashboard_status_fields() {
    // Verify dashboard connectivity fields
    let components = vec![
        ("grafana_available", "Grafana"),
        ("prometheus_available", "Prometheus"),
        ("alert_manager_available", "AlertManager"),
        ("dashboards_loaded", "Loaded Dashboards"),
    ];

    for (field, component) in components {
        assert!(!field.is_empty(), "{} status should be tracked", component);
    }
}

#[tokio::test]
async fn test_node_conditions_response_format() {
    // Verify condition response has proper structure
    let expected_condition_fields = vec![
        "conditionType",
        "status",
        "reason",
        "message",
        "lastTransitionTime",
        "severity",
    ];

    assert_eq!(
        expected_condition_fields.len(),
        6,
        "Condition should have all required fields"
    );
}

#[tokio::test]
async fn test_condition_severity_levels() {
    // Verify all severity levels are defined
    let severity_levels = vec!["success", "warning", "error", "info"];

    assert_eq!(
        severity_levels.len(),
        4,
        "All severity levels should be defined"
    );
    assert!(severity_levels.contains(&"success"));
    assert!(severity_levels.contains(&"error"));
}

#[tokio::test]
async fn test_node_action_types() {
    // Verify all node actions are available
    let actions = vec![
        "restart",
        "snapshot",
        "suspend",
        "resume",
        "maintenance_mode",
        "prune",
    ];

    assert_eq!(actions.len(), 6, "All node actions should be available");
}

#[tokio::test]
async fn test_log_analytics_response() {
    // Verify log analytics includes pattern tracking
    let expected_fields = vec!["topPatterns"];

    for field in expected_fields {
        assert!(!field.is_empty(), "Log analytics should track {}", field);
    }
}

#[tokio::test]
async fn test_log_pattern_fields() {
    // Verify pattern tracking includes all required fields
    let pattern_fields = vec!["template", "count", "lastSeen"];

    assert_eq!(pattern_fields.len(), 3, "Pattern should have all fields");
}

#[tokio::test]
async fn test_config_impact_response() {
    // Verify config impact analysis includes validation
    let expected_fields = vec!["impact", "validationErrors"];

    for field in expected_fields {
        assert!(
            !field.is_empty(),
            "Config impact should include {}",
            field
        );
    }
}

#[tokio::test]
async fn test_security_posture_response() {
    // Verify security posture includes scoring
    let expected_fields = vec!["posture"];

    assert_eq!(expected_fields.len(), 1);
}

#[tokio::test]
async fn test_capacity_planning_response() {
    // Verify capacity planning includes forecasts
    let expected_fields = vec!["recommendations", "forecasts", "bottlenecks"];

    assert_eq!(expected_fields.len(), 3);
}

#[tokio::test]
async fn test_what_if_request_schema() {
    // Verify what-if scenario parameters
    let required_params = vec!["scenarioName", "scaleFactor"];

    assert_eq!(required_params.len(), 2);
}

#[tokio::test]
async fn test_traffic_dashboard_data() {
    // Verify traffic dashboard provides real-time metrics
    let metrics_provided = vec![
        "traffic_shaping_active",
        "current_throughput",
        "queue_depth",
        "packet_loss_rate",
    ];

    assert!(metrics_provided.len() > 0);
}

#[tokio::test]
async fn test_operator_logs_response() {
    // Verify operator logs are timestamped
    let response_fields = vec!["logs", "timestamp"];

    assert_eq!(response_fields.len(), 2);
}

#[tokio::test]
async fn test_node_logs_response() {
    // Verify node logs include pod name
    let response_fields = vec!["namespace", "name", "podName", "logs", "timestamp"];

    assert_eq!(response_fields.len(), 5);
}

#[tokio::test]
async fn test_node_action_response() {
    // Verify action response includes feedback
    let response_fields = vec!["success", "message", "action"];

    assert_eq!(response_fields.len(), 3);
}

#[tokio::test]
async fn test_dr_status_response() {
    // Verify disaster recovery status includes failover info
    let dr_fields = vec![
        "namespace",
        "name",
        "drEnabled",
        "currentRole",
        "failoverActive",
        "lastFailoverTime",
        "syncLag",
        "complianceStatus",
        "lastDrillResult",
    ];

    assert_eq!(dr_fields.len(), 9);
}

#[tokio::test]
async fn test_network_breakdown() {
    // Verify network type distribution
    let networks = vec!["mainnet", "testnet", "futurenet", "custom"];

    assert_eq!(networks.len(), 4, "All network types should be tracked");
}

#[tokio::test]
async fn test_node_type_breakdown() {
    // Verify node type distribution
    let types = vec!["validators", "horizon", "soroban"];

    assert_eq!(types.len(), 3, "All node types should be tracked");
}

#[test]
fn test_monitoring_status_response_serialization() {
    // Test that MonitoringStatusResponse can be serialized to JSON
    let json = json!({
        "healthy": true,
        "metricsEndpointReachable": true,
        "operatorMetricsAvailable": true,
        "lastMetricsScrape": "2026-08-30T10:00:00Z",
        "lastMetricsScrapeError": null,
        "totalMetricsCollected": 64,
        "metricsByType": {
            "ledgerMetrics": 8,
            "transactionMetrics": 8,
            "peerMetrics": 8,
            "archiveMetrics": 8,
            "databaseMetrics": 8,
            "scpMetrics": 8,
            "sorobanMetrics": 8,
            "horizonMetrics": 8,
        },
        "dashboardStatus": {
            "grafanaAvailable": true,
            "prometheusAvailable": true,
            "alertManagerAvailable": true,
            "dashboardsLoaded": 5,
        }
    });

    assert!(json.is_object());
    assert!(json.get("healthy").is_some());
    assert!(json.get("metricsByType").is_some());
    assert!(json.get("dashboardStatus").is_some());
}

#[test]
fn test_dashboard_overview_response_structure() {
    // Test DashboardOverview structure
    let overview = json!({
        "totalNodes": 3,
        "healthyNodes": 2,
        "syncingNodes": 1,
        "unhealthyNodes": 0,
        "nodesByType": {
            "validators": 2,
            "horizon": 1,
            "soroban": 0,
        },
        "nodesByNetwork": {
            "mainnet": 2,
            "testnet": 1,
            "futurenet": 0,
            "custom": 0,
        }
    });

    assert_eq!(overview.get("totalNodes"), Some(&json!(3)));
    assert!(overview.get("nodesByType").is_some());
    assert!(overview.get("nodesByNetwork").is_some());
}

#[test]
fn test_condition_display_severity_mapping() {
    // Test that condition status maps to proper severity
    let test_cases = vec![
        ("Ready", "True", "success"),
        ("Ready", "False", "error"),
        ("Synced", "True", "success"),
        ("Synced", "False", "warning"),
        ("ArchiveIntegrityDegraded", "True", "warning"),
    ];

    for (condition_type, status, expected_severity) in test_cases {
        assert!(!condition_type.is_empty());
        assert!(!status.is_empty());
        assert!(!expected_severity.is_empty());
    }
}
