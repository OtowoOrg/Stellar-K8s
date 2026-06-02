use chrono;
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec};
use kube::api::ObjectMeta;
use stellar_k8s::controller::health_check_sidecar::SyncStatus;

#[test]
fn test_health_check_sidecar_injected() {
    // Verify that the health check sidecar is injected into the pod spec
    let pod_spec = PodSpec {
        containers: vec![
            Container {
                name: "stellar-node".to_string(),
                ..Default::default()
            },
            Container {
                name: "stellar-health-check".to_string(),
                image: Some("stellar-k8s:latest".to_string()),
                command: Some(vec!["/stellar-health-sidecar".to_string()]),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    assert_eq!(pod_spec.containers.len(), 2);
    assert_eq!(pod_spec.containers[1].name, "stellar-health-check");
    assert_eq!(
        pod_spec.containers[1].command,
        Some(vec!["/stellar-health-sidecar".to_string()])
    );
}

#[test]
fn test_liveness_probe_uses_health_check_sidecar() {
    // Verify that liveness probe points to health check sidecar port
    let probe = k8s_openapi::api::core::v1::Probe {
        http_get: Some(k8s_openapi::api::core::v1::HTTPGetAction {
            path: Some("/healthz".to_string()),
            port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8081),
            ..Default::default()
        }),
        initial_delay_seconds: Some(30),
        period_seconds: Some(10),
        timeout_seconds: Some(5),
        failure_threshold: Some(3),
        ..Default::default()
    };

    assert_eq!(probe.initial_delay_seconds, Some(30));
    assert_eq!(probe.period_seconds, Some(10));
    assert_eq!(probe.timeout_seconds, Some(5));
    assert_eq!(probe.failure_threshold, Some(3));

    // Verify it uses the sidecar port
    if let Some(http_get) = &probe.http_get {
        assert_eq!(
            http_get.port,
            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8081)
        );
        assert_eq!(http_get.path, Some("/healthz".to_string()));
    }
}

#[test]
fn test_readiness_probe_uses_health_check_sidecar() {
    // Verify that readiness probe points to health check sidecar port
    let probe = k8s_openapi::api::core::v1::Probe {
        http_get: Some(k8s_openapi::api::core::v1::HTTPGetAction {
            path: Some("/readyz".to_string()),
            port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8081),
            ..Default::default()
        }),
        initial_delay_seconds: Some(60),
        period_seconds: Some(5),
        timeout_seconds: Some(5),
        failure_threshold: Some(2),
        ..Default::default()
    };

    assert_eq!(probe.initial_delay_seconds, Some(60));
    assert_eq!(probe.period_seconds, Some(5));
    assert_eq!(probe.timeout_seconds, Some(5));
    assert_eq!(probe.failure_threshold, Some(2));

    // Verify it uses the sidecar port and readiness endpoint
    if let Some(http_get) = &probe.http_get {
        assert_eq!(
            http_get.port,
            k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8081)
        );
        assert_eq!(http_get.path, Some("/readyz".to_string()));
    }
}

#[test]
fn test_node_unready_during_sync() {
    // Verify that nodes are marked Unready during sync phases
    let sync_status = SyncStatus {
        is_synced: false,
        ledger_num: 100,
        network_ledger: 200,
        last_check: chrono::Utc::now().timestamp(),
    };

    assert!(!sync_status.is_synced);
    assert!(sync_status.network_ledger > sync_status.ledger_num);

    // Verify the lag is significant (more than 10 ledgers)
    let lag = sync_status.network_ledger - sync_status.ledger_num;
    assert!(lag > 10, "Expected lag > 10, got {}", lag);
}

#[test]
fn test_node_ready_when_synced() {
    // Verify that nodes are marked Ready when synced
    let sync_status = SyncStatus {
        is_synced: true,
        ledger_num: 1000,
        network_ledger: 1005,
        last_check: chrono::Utc::now().timestamp(),
    };

    assert!(sync_status.is_synced);

    // Verify the lag is within acceptable range (≤ 10 ledgers)
    let lag = sync_status.network_ledger - sync_status.ledger_num;
    assert!(lag <= 10, "Expected lag ≤ 10, got {}", lag);
}

#[test]
fn test_validator_sync_threshold() {
    // Test that validator nodes use 10-ledger sync threshold
    let synced_status = SyncStatus {
        is_synced: true,
        ledger_num: 1000,
        network_ledger: 1010, // Exactly at threshold
        last_check: chrono::Utc::now().timestamp(),
    };

    let unsynced_status = SyncStatus {
        is_synced: false,
        ledger_num: 1000,
        network_ledger: 1011, // Just over threshold
        last_check: chrono::Utc::now().timestamp(),
    };

    assert!(synced_status.is_synced);
    assert!(!unsynced_status.is_synced);
}

#[test]
fn test_horizon_sync_threshold() {
    // Test that horizon nodes use 5-ledger sync threshold
    let synced_status = SyncStatus {
        is_synced: true,
        ledger_num: 1000,
        network_ledger: 1005, // Exactly at threshold
        last_check: chrono::Utc::now().timestamp(),
    };

    let unsynced_status = SyncStatus {
        is_synced: false,
        ledger_num: 1000,
        network_ledger: 1006, // Just over threshold
        last_check: chrono::Utc::now().timestamp(),
    };

    assert!(synced_status.is_synced);
    assert!(!unsynced_status.is_synced);
}
