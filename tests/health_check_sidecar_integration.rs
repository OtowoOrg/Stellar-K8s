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
use axum::http::StatusCode;
use std::sync::Arc;
use stellar_k8s::controller::health_check_sidecar::{create_router, HealthCheckState, SyncStatus};
use tokio::sync::RwLock;
use tower::ServiceExt;

#[tokio::test]
async fn test_health_check_sidecar_liveness() {
    let state = HealthCheckState {
        core_url: "http://localhost:11626".to_string(),
        sync_status: Arc::new(RwLock::new(SyncStatus::default())),
    };

    let app = create_router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/healthz")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return service unavailable since we can't connect to a real core
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_health_check_sidecar_readiness_unsynced() {
    let state = HealthCheckState {
        core_url: "http://localhost:11626".to_string(),
        sync_status: Arc::new(RwLock::new(SyncStatus {
            is_synced: false,
            ledger_num: 100,
            network_ledger: 200,
            last_check: chrono::Utc::now().timestamp(),
        })),
    };

    let app = create_router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/readyz")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return service unavailable when not synced
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_health_check_sidecar_readiness_synced() {
    let state = HealthCheckState {
        core_url: "http://localhost:11626".to_string(),
        sync_status: Arc::new(RwLock::new(SyncStatus {
            is_synced: true,
            ledger_num: 1000,
            network_ledger: 1005,
            last_check: chrono::Utc::now().timestamp(),
        })),
    };

    let app = create_router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/readyz")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return OK when synced
    assert_eq!(response.status(), StatusCode::OK);
}
