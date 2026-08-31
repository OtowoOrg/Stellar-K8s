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
//! Webhook HTTP contract tests for malformed and boundary payloads (#1152)
//!
//! Conformance tests in `admission_webhook_conformance.rs` exercise
//! `WebhookServer::validate` with structured CRD objects. This suite covers
//! the **HTTP admission contract** — empty bodies, non-JSON, truncated JSON,
//! missing `request`, oversized payloads, and mutate/validate parity — using
//! `WebhookServer::into_router()` + `tower::ServiceExt::oneshot`.
//!
//! All tests are hermetic (no cluster / network listener bind required).
//!
//! ```bash
//! cargo test --test webhook_payload_contract
//! ```

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::json;
use stellar_k8s::webhook::{WasmRuntime, WebhookServer};
use tower::ServiceExt;

fn new_app() -> axum::Router {
    WebhookServer::new(WasmRuntime::new().unwrap()).into_router()
}

async fn post_json(app: axum::Router, path: &str, body: impl Into<Body>) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(body.into())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn post_raw(
    app: axum::Router,
    path: &str,
    content_type: Option<&str>,
    body: impl Into<Body>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method("POST").uri(path);
    if let Some(ct) = content_type {
        builder = builder.header("content-type", ct);
    }
    let response = app
        .oneshot(builder.body(body.into()).expect("request"))
        .await
        .expect("oneshot");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn minimal_create_review(object: serde_json::Value) -> serde_json::Value {
    json!({
        "apiVersion": "admission.k8s.io/v1",
        "kind": "AdmissionReview",
        "request": {
            "uid": "contract-test-uid",
            "kind": { "group": "stellar.org", "version": "v1alpha1", "kind": "StellarNode" },
            "resource": { "group": "stellar.org", "version": "v1alpha1", "resource": "stellarnodes" },
            "name": "contract-node",
            "namespace": "default",
            "operation": "CREATE",
            "userInfo": { "username": "contract-test" },
            "object": object
        }
    })
}

fn valid_validator_object() -> serde_json::Value {
    json!({
        "apiVersion": "stellar.org/v1alpha1",
        "kind": "StellarNode",
        "metadata": {
            "name": "contract-node",
            "namespace": "default",
            "labels": {
                "project-id": "stellar-project",
                "owner": "platform-team"
            }
        },
        "spec": {
            "nodeType": "Validator",
            "network": "testnet",
            "version": "v21.0.0",
            "replicas": 1,
            "validatorConfig": {
                "seedSecretRef": "validator-seed",
                "enableHistoryArchive": false,
                "historyArchiveUrls": []
            }
        }
    })
}

fn assert_client_error(status: StatusCode, body: &str, context: &str) {
    assert!(
        status.is_client_error(),
        "{context}: expected 4xx, got {status}; body={body}"
    );
}

// ── Malformed HTTP bodies ─────────────────────────────────────────────────────

#[tokio::test]
async fn contract_validate_empty_body_is_rejected() {
    let (status, body) = post_json(new_app(), "/validate", Body::empty()).await;
    assert_client_error(status, &body, "empty body");
}

#[tokio::test]
async fn contract_mutate_empty_body_is_rejected() {
    let (status, body) = post_json(new_app(), "/mutate", Body::empty()).await;
    assert_client_error(status, &body, "empty body on /mutate");
}

#[tokio::test]
async fn contract_validate_non_json_body_is_rejected() {
    let (status, body) = post_raw(
        new_app(),
        "/validate",
        Some("application/json"),
        Body::from("this is not json {{{"),
    )
    .await;
    assert_client_error(status, &body, "non-json body");
}

#[tokio::test]
async fn contract_validate_truncated_json_is_rejected() {
    let (status, body) = post_json(
        new_app(),
        "/validate",
        Body::from(r#"{"apiVersion":"admission.k8s.io/v1","kind":"AdmissionReview""#),
    )
    .await;
    assert_client_error(status, &body, "truncated json");
}

#[tokio::test]
async fn contract_validate_wrong_content_type_plain_text_is_rejected() {
    let (status, body) =
        post_raw(new_app(), "/validate", Some("text/plain"), Body::from("{}")).await;
    assert_client_error(status, &body, "wrong content-type");
}

// ── Boundary / malformed AdmissionReview shapes ───────────────────────────────

#[tokio::test]
async fn contract_validate_empty_object_review_is_bad_request() {
    let (status, body) = post_json(new_app(), "/validate", Body::from("{}")).await;
    // Axum Json may accept `{}` as AdmissionReview with missing request;
    // handler then returns BAD_REQUEST via try_into failure, or extractor rejects.
    assert_client_error(status, &body, "empty admission review object");
}

#[tokio::test]
async fn contract_validate_missing_request_is_bad_request() {
    let payload = json!({
        "apiVersion": "admission.k8s.io/v1",
        "kind": "AdmissionReview"
    });
    let (status, body) = post_json(new_app(), "/validate", Body::from(payload.to_string())).await;
    assert_client_error(status, &body, "missing request");
    if status == StatusCode::BAD_REQUEST {
        assert!(
            body.to_ascii_lowercase().contains("invalid")
                || body.to_ascii_lowercase().contains("request"),
            "expected invalid-request messaging, got: {body}"
        );
    }
}

#[tokio::test]
async fn contract_validate_null_request_is_bad_request() {
    let payload = json!({
        "apiVersion": "admission.k8s.io/v1",
        "kind": "AdmissionReview",
        "request": null
    });
    let (status, body) = post_json(new_app(), "/validate", Body::from(payload.to_string())).await;
    assert_client_error(status, &body, "null request");
}

#[tokio::test]
async fn contract_mutate_missing_request_is_bad_request() {
    let payload = json!({
        "apiVersion": "admission.k8s.io/v1",
        "kind": "AdmissionReview"
    });
    let (status, body) = post_json(new_app(), "/mutate", Body::from(payload.to_string())).await;
    assert_client_error(status, &body, "missing request on /mutate");
}

#[tokio::test]
async fn contract_validate_array_body_is_rejected() {
    let (status, body) = post_json(new_app(), "/validate", Body::from("[]")).await;
    assert_client_error(status, &body, "array body");
}

#[tokio::test]
async fn contract_validate_wrong_kind_payload_is_client_error_or_denied() {
    // Valid AdmissionReview envelope wrapping a Pod instead of StellarNode.
    let payload = minimal_create_review(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": "not-a-stellarnode" },
        "spec": { "containers": [] }
    }));
    let (status, body) = post_json(new_app(), "/validate", Body::from(payload.to_string())).await;
    // Either HTTP-level rejection or an AdmissionResponse with allowed=false.
    assert!(
        status.is_client_error()
            || status == StatusCode::OK && body.contains("\"allowed\":false")
            || status == StatusCode::OK && body.contains("\"allowed\": false"),
        "wrong-kind payload must not be silently admitted; status={status} body={body}"
    );
}

// ── Boundary sizes ────────────────────────────────────────────────────────────

#[tokio::test]
async fn contract_validate_oversized_body_is_rejected() {
    // Axum default body limit is 2 MiB; send ~2.5 MiB of JSON noise.
    let mut huge =
        String::from(r#"{"apiVersion":"admission.k8s.io/v1","kind":"AdmissionReview","pad":""#);
    huge.push_str(&"x".repeat(2_500_000));
    huge.push_str(r#""}"#);
    let (status, body) = post_json(new_app(), "/validate", Body::from(huge)).await;
    assert!(
        status == StatusCode::PAYLOAD_TOO_LARGE
            || status.is_client_error()
            || status == StatusCode::INTERNAL_SERVER_ERROR,
        "oversized body should not be treated as a successful admission; status={status} body_len={}",
        body.len()
    );
}

#[tokio::test]
async fn contract_validate_boundary_empty_object_field_is_handled() {
    let payload = minimal_create_review(json!({}));
    let (status, body) = post_json(new_app(), "/validate", Body::from(payload.to_string())).await;
    assert!(
        status == StatusCode::OK || status.is_client_error(),
        "empty object field must not panic; status={status}"
    );
    if status == StatusCode::OK {
        assert!(
            body.contains("\"allowed\":false") || body.contains("\"allowed\": false"),
            "empty StellarNode object must be denied; body={body}"
        );
    }
}

#[tokio::test]
async fn contract_validate_boundary_long_name_is_handled() {
    let mut object = valid_validator_object();
    let long_name = "n".repeat(253);
    object["metadata"]["name"] = json!(long_name);
    let mut payload = minimal_create_review(object);
    payload["request"]["name"] = json!("n".repeat(253));
    let (status, body) = post_json(new_app(), "/validate", Body::from(payload.to_string())).await;
    assert!(
        status == StatusCode::OK || status.is_client_error(),
        "long name must not panic; status={status} body={body}"
    );
}

// ── Happy-path contract smoke (ensures router wiring is intact) ───────────────

#[tokio::test]
async fn contract_validate_valid_review_returns_admission_response() {
    let payload = minimal_create_review(valid_validator_object());
    let (status, body) = post_json(new_app(), "/validate", Body::from(payload.to_string())).await;
    assert_eq!(status, StatusCode::OK, "valid review body={body}");
    assert!(
        body.contains("AdmissionReview") || body.contains("allowed"),
        "expected AdmissionReview response, got: {body}"
    );
}

#[tokio::test]
async fn contract_mutate_valid_review_returns_ok() {
    let payload = minimal_create_review(valid_validator_object());
    let (status, body) = post_json(new_app(), "/mutate", Body::from(payload.to_string())).await;
    assert_eq!(status, StatusCode::OK, "valid mutate body={body}");
}

#[tokio::test]
async fn contract_health_is_ok() {
    let response = new_app()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
