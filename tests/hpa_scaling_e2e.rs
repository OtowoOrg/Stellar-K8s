use axum::extract::{Path, State};
use axum::response::IntoResponse;
use http::StatusCode;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use stellar_k8s::controller::{
    horizon_scaler::{HorizonRateLimitScaler, ScalingDecision, ScalingSignal},
    ControllerState,
};
use stellar_k8s::crd::StellarNode;
use stellar_k8s::rest_api::custom_metrics::{
    get_horizon_metric, get_metrics_discovery, get_pod_metric, get_stellar_node_metric,
    ApiResourceList, MetricValueList,
};
use stellar_k8s::rest_api::metrics_store::{StellarMetricsSnapshot, StellarMetricsStore};

// --- Test Setup Helper ---

fn mock_controller_state() -> Arc<ControllerState> {
    let client = kube::Client::try_default()
        .unwrap_or_else(|_| panic!("Failed to create dummy kube client for tests (requires valid KUBECONFIG or mocked API)"));

    let env_filter = EnvFilter::new("info");
    let (_, log_reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);

    let metrics_store = Arc::new(StellarMetricsStore::new());

    Arc::new(ControllerState {
        client,
        enable_mtls: false,
        operator_namespace: "stellar-system".to_string(),
        watch_namespace: None,
        mtls_config: None,
        dry_run: false,
        retry_budget_retriable_secs: 10,
        retry_budget_nonretriable_secs: 60,
        retry_budget_max_attempts: 3,
        is_leader: Arc::new(AtomicBool::new(true)),
        event_reporter: kube::runtime::events::Reporter {
            controller: "stellar-operator".to_string(),
            instance: None,
        },
        operator_config: Arc::new(Default::default()),
        reconcile_id_counter: AtomicU64::new(0),
        last_reconcile_success: Arc::new(AtomicU64::new(0)),
        log_reload_handle,
        log_level_expires_at: Arc::new(Mutex::new(None)),
        last_event_received: Arc::new(AtomicU64::new(0)),
        job_registry: Arc::new(stellar_k8s::controller::background_jobs::JobRegistry::new()),
        audit_log: Arc::new(stellar_k8s::controller::audit_log::AuditLog::new(100)),
        oidc_config: None,
        metrics_store,
    })
}

// Extract body from axum response
async fn extract_json_body<T: serde::de::DeserializeOwned>(
    response: axum::response::Response,
) -> (StatusCode, Option<T>) {
    let status = response.status();
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    if body_bytes.is_empty() {
        return (status, None);
    }

    let json = serde_json::from_slice(&body_bytes).expect("Failed to parse JSON body");
    (status, Some(json))
}

// --- E2E Tests ---

#[tokio::test]
async fn test_hpa_custom_metrics_discovery() {
    let response = get_metrics_discovery().await;
    let (status, body): (StatusCode, Option<ApiResourceList>) = extract_json_body(response).await;

    assert_eq!(status, StatusCode::OK);
    let list = body.unwrap();

    assert_eq!(list.kind, "APIResourceList");
    assert_eq!(list.group_version, "custom.metrics.k8s.io/v1beta2");

    // Ensure all target metrics are registered for all three resources (pods, stellarnodes, horizons)
    let resource_names: Vec<_> = list.resources.iter().map(|r| r.name.as_str()).collect();
    assert!(resource_names.contains(&"pods/stellar_horizon_tps"));
    assert!(resource_names.contains(&"stellarnodes.stellar.org/stellar_horizon_tps"));
    assert!(resource_names.contains(&"horizons.stellar.org/stellar_horizon_tps"));
    assert!(resource_names.contains(&"pods/stellar_horizon_queue_length"));
    assert!(resource_names.contains(&"horizons.stellar.org/stellar_horizon_queue_length"));
}

#[tokio::test]
async fn test_hpa_tps_metric_endpoint() {
    // 1. Setup operator state and metrics store
    let state = mock_controller_state();

    // 2. Simulate collector pushing TPS data
    state.metrics_store.upsert(
        "default",
        "horizon-0",
        StellarMetricsSnapshot {
            tps: 150,
            queue_length: 10,
            updated_at: chrono::Utc::now(),
            ..Default::default()
        },
    );

    // 3. Simulate HPA querying the custom metrics API for a specific pod
    let path = Path((
        "default".to_string(),
        "horizon-0".to_string(),
        "stellar_horizon_tps".to_string(),
    ));

    let response = get_pod_metric(State(state.clone()), path).await;
    let (status, body): (StatusCode, Option<MetricValueList>) = extract_json_body(response).await;

    assert_eq!(status, StatusCode::OK);
    let list = body.unwrap();
    assert_eq!(list.items.len(), 1);

    let item = &list.items[0];
    assert_eq!(item.value, "150");
    assert_eq!(item.metric.name, "stellar_horizon_tps");
    assert_eq!(item.described_object.kind, "Pod");
}

#[tokio::test]
async fn test_hpa_queue_length_metric_endpoint() {
    let state = mock_controller_state();

    state.metrics_store.upsert(
        "testnet",
        "my-horizon",
        StellarMetricsSnapshot {
            tps: 50,
            queue_length: 750,
            updated_at: chrono::Utc::now(),
            ..Default::default()
        },
    );

    // Simulate HPA querying the custom metrics API targeting the Horizon (StellarNode) resource
    let path = Path((
        "testnet".to_string(),
        "my-horizon".to_string(),
        "stellar_horizon_queue_length".to_string(),
    ));

    let response = get_horizon_metric(State(state.clone()), path).await;
    let (status, body): (StatusCode, Option<MetricValueList>) = extract_json_body(response).await;

    assert_eq!(status, StatusCode::OK);
    let list = body.unwrap();
    assert_eq!(list.items.len(), 1);

    let item = &list.items[0];
    assert_eq!(item.value, "750");
    assert_eq!(item.metric.name, "stellar_horizon_queue_length");
    assert_eq!(item.described_object.kind, "StellarNode");
}

#[tokio::test]
async fn test_hpa_scale_up_under_load() {
    let scaler = HorizonRateLimitScaler::new("http://prom".to_string());

    // Simulate high TPS load (600 TPS across 2 replicas = 300 TPS/replica)
    // Default threshold is 100 TPS/replica
    let decision = scaler.compute_replicas(
        2,   // current replicas
        0.0, // 429 rate
        600, // current TPS
        50,  // queue length
    );

    // Target = ceil(600 / (100 * 0.75)) = ceil(600 / 75) = 8
    assert_eq!(decision.target_replicas, 8);
    assert_eq!(decision.signal, ScalingSignal::TransactionsPerSecond);
}

#[tokio::test]
async fn test_hpa_scale_down_after_load() {
    let scaler = HorizonRateLimitScaler::new("http://prom".to_string());

    // Simulate load dropping (5 replicas, near 0 load)
    let decision = scaler.compute_replicas(
        5,   // current replicas
        0.0, // 429 rate
        5,   // current TPS
        2,   // queue length
    );

    // Should scale down gradually by 1
    assert_eq!(decision.target_replicas, 4);
    assert_eq!(decision.signal, ScalingSignal::ScaleDown);
}

#[tokio::test]
async fn test_hpa_stale_metrics_fallback() {
    let state = mock_controller_state();

    // Push stale metric (3 minutes old)
    state.metrics_store.upsert(
        "default",
        "horizon",
        StellarMetricsSnapshot {
            tps: 500, // Should be ignored because it's stale
            queue_length: 1000,
            updated_at: chrono::Utc::now() - chrono::Duration::seconds(180),
            ..Default::default()
        },
    );

    let path = Path((
        "default".to_string(),
        "horizon".to_string(),
        "stellar_horizon_tps".to_string(),
    ));

    let response = get_stellar_node_metric(State(state.clone()), path).await;
    let (status, body): (StatusCode, Option<MetricValueList>) = extract_json_body(response).await;

    assert_eq!(status, StatusCode::OK);
    let list = body.unwrap();

    // TPS should fallback to 0 to prevent scaling on old data
    assert_eq!(list.items[0].value, "0");
}

#[tokio::test]
async fn test_hpa_multi_horizon_namespace() {
    let state = mock_controller_state();

    state.metrics_store.upsert(
        "ns-1",
        "horizon",
        StellarMetricsSnapshot {
            tps: 100,
            updated_at: chrono::Utc::now(),
            ..Default::default()
        },
    );

    state.metrics_store.upsert(
        "ns-2",
        "horizon",
        StellarMetricsSnapshot {
            tps: 200,
            updated_at: chrono::Utc::now(),
            ..Default::default()
        },
    );

    // Verify ns-1
    let path1 = Path((
        "ns-1".to_string(),
        "horizon".to_string(),
        "stellar_horizon_tps".to_string(),
    ));
    let response1 = get_horizon_metric(State(state.clone()), path1).await;
    let (_, body1): (StatusCode, Option<MetricValueList>) = extract_json_body(response1).await;
    assert_eq!(body1.unwrap().items[0].value, "100");

    // Verify ns-2
    let path2 = Path((
        "ns-2".to_string(),
        "horizon".to_string(),
        "stellar_horizon_tps".to_string(),
    ));
    let response2 = get_horizon_metric(State(state.clone()), path2).await;
    let (_, body2): (StatusCode, Option<MetricValueList>) = extract_json_body(response2).await;
    assert_eq!(body2.unwrap().items[0].value, "200");
}
