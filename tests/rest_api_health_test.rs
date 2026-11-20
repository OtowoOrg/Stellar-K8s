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

//! Integration tests verifying that the REST API server builds its Axum 0.8
//! router without panicking, binds to a TCP listener, and returns HTTP 200 JSON
//! payloads on `/healthz`, `/health`, and `/livez` (Issue #151).

#![cfg(feature = "rest-api")]

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use stellar_k8s::controller::ControllerState;
use stellar_k8s::rest_api::metrics_store::StellarMetricsStore;
use stellar_k8s::rest_api::{build_router, run_server};

fn test_controller_state() -> Arc<ControllerState> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = kube::Config::new("http://127.0.0.1:6443".parse().expect("valid URI"));
    let client = kube::Client::try_from(config).expect("failed to build offline kube::Client");

    let env_filter = EnvFilter::new("info");
    let (_, log_reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);
    let audit_log = Arc::new(stellar_k8s::controller::audit_log::AuditLog::new());

    Arc::new(ControllerState {
        client,
        enable_mtls: false,
        operator_namespace: "stellar-system".to_string(),
        watch_namespace: None,
        mtls_config: None,
        dry_run: true,
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
        audit_log: audit_log.clone(),
        audit_recorder: Arc::new(stellar_k8s::controller::AuditRecorder::new(
            audit_log,
            vec![],
            None,
        )),
        anomaly_detector: Arc::new(stellar_k8s::controller::AnomalyDetector::new(
            Default::default(),
        )),
        plugin_registry: Arc::new(stellar_k8s::plugin_sdk::PluginRegistry::new()),
        analytics_engine: Arc::new(stellar_k8s::logging::analytics::AnalyticsEngine::new(
            Duration::from_secs(3600),
        )),
        oidc_config: None,
        metrics_store: Arc::new(StellarMetricsStore::new()),
    })
}

#[tokio::test]
async fn rest_api_server_starts_and_health_endpoints_return_200() {
    let state = test_controller_state();
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind ephemeral port");
    let addr = listener.local_addr().expect("failed to read local_addr");

    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server failed");
    });

    let http = reqwest::Client::new();

    // 1. Verify GET /healthz returns 200 OK and {"status":"ok"}
    let healthz_resp = http
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .expect("GET /healthz request failed");
    assert_eq!(healthz_resp.status(), reqwest::StatusCode::OK);
    let healthz_json: serde_json::Value = healthz_resp
        .json()
        .await
        .expect("GET /healthz body is not valid JSON");
    assert_eq!(healthz_json["status"], "ok");

    // 2. Verify GET /health returns 200 OK and {"status":"healthy","version":...}
    let health_resp = http
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("GET /health request failed");
    assert_eq!(health_resp.status(), reqwest::StatusCode::OK);
    let health_json: serde_json::Value = health_resp
        .json()
        .await
        .expect("GET /health body is not valid JSON");
    assert_eq!(health_json["status"], "healthy");
    assert_eq!(health_json["version"], env!("CARGO_PKG_VERSION"));

    // 3. Verify GET /livez returns 200 OK and {"status":"ok"}
    let livez_resp = http
        .get(format!("http://{addr}/livez"))
        .send()
        .await
        .expect("GET /livez request failed");
    assert_eq!(livez_resp.status(), reqwest::StatusCode::OK);
    let livez_json: serde_json::Value = livez_resp
        .json()
        .await
        .expect("GET /livez body is not valid JSON");
    assert_eq!(livez_json["status"], "ok");

    server_handle.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_server_binds_default_port_8080_and_serves_healthz() {
    // Check if 0.0.0.0:8080 is available in this test environment
    if tokio::net::TcpListener::bind("0.0.0.0:8080").await.is_err() {
        return;
    }

    std::env::set_var("REST_API_PORT", "8080");
    let state = test_controller_state();
    let server_handle = tokio::spawn(async move {
        let _ = run_server(state, None).await;
    });

    let http = reqwest::Client::new();
    let mut last_err = None;
    let mut succeeded = false;

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        match http.get("http://127.0.0.1:8080/healthz").send().await {
            Ok(resp) => {
                assert_eq!(resp.status(), reqwest::StatusCode::OK);
                let body: serde_json::Value = resp.json().await.expect("valid JSON");
                assert_eq!(body, serde_json::json!({"status": "ok"}));
                for endpoint in ["http://localhost:8080/healthz", "http://localhost:8080/health"] {
                    let output = tokio::process::Command::new("curl")
                        .args(["-v", "--max-time", "5", "--noproxy", "*", endpoint])
                        .output()
                        .await
                        .expect("curl must execute");
                    println!("=== curl -v {} ===", endpoint);
                    println!("{}", String::from_utf8_lossy(&output.stderr));
                    println!("{}", String::from_utf8_lossy(&output.stdout));
                    assert!(output.status.success(), "curl {} exited non-zero", endpoint);
                }
                succeeded = true;
                break;
            }
            Err(e) => last_err = Some(e),
        }
    }

    server_handle.abort();
    assert!(
        succeeded,
        "run_server on 0.0.0.0:8080 did not respond to GET /healthz: {last_err:?}"
    );
}
