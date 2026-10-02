// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
//! Mutating Admission Webhook — WASM Bytecode Optimizer
//!
//! This module implements the Kubernetes MutatingAdmissionWebhook handler that
//! intercepts `StellarNode` WASM deployment payloads, streams the raw bytecode
//! to the wasm-opt sidecar service, and transparently replaces the payload with
//! the optimized binary before the object is persisted to etcd.
//!
//! # How it fits in
//!
//! ```text
//!  kubectl apply -f deploy.yaml
//!       │
//!       ▼
//!  Kubernetes API server
//!       │  MutatingWebhookConfiguration matches
//!       │  StellarNode CREATE/UPDATE ops
//!       ▼
//!  stellar-webhook Pod  ◄── /mutate/wasm  (this module)
//!       │
//!       │  POST /optimize  (raw WASM bytes)
//!       ▼
//!  wasm-opt-sidecar Pod  (Alpine + Binaryen)
//!       │
//!       │  optimized WASM bytes
//!       ▼
//!  stellar-webhook Pod
//!       │  JSON Patch: replace spec.wasmBinary with optimized base64
//!       ▼
//!  Kubernetes API server  →  etcd
//! ```
//!
//! # Timeout guarantee
//!
//! The total time from webhook entry to response is bounded by
//! [`WEBHOOK_BUDGET`] (default 9 s), always leaving at least 1 s of
//! headroom before the hard Kubernetes 10-second admission deadline.
//!
//! # Idempotency
//!
//! If the incoming `StellarNode` spec carries the annotation
//! `stellar.io/wasm-optimized: "true"`, the webhook skips re-optimization and
//! returns an empty (allow) patch immediately.
//!
//! # Metrics annotation
//!
//! After a successful optimization the webhook injects the following
//! annotations so operators can observe bytecode savings in dashboards:
//!
//! - `stellar.io/wasm-original-size`  — original size in bytes
//! - `stellar.io/wasm-optimized-size` — optimized size in bytes
//! - `stellar.io/wasm-reduction-pct`  — reduction percentage (1 decimal)
//! - `stellar.io/wasm-optimized`      — `"true"` (idempotency guard)

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use base64::Engine as _;
use serde_json::{json, Value};
use tracing::{debug, error, info, instrument, warn};

use crate::deployment::optimizer::{OptimizationResult, OptimizerConfig, WasmOptimizer};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Total time budget for the mutation step (webhook entry → response).
/// Must be < 10 s (K8s hard deadline).  We leave 1 s of headroom.
const WEBHOOK_BUDGET: Duration = Duration::from_secs(9);

/// Annotation set on a StellarNode after a successful optimization run.
/// Used as an idempotency guard: a second admission round-trip is a no-op.
const ANN_OPTIMIZED: &str = "stellar.io/wasm-optimized";
const ANN_ORIGINAL_SIZE: &str = "stellar.io/wasm-original-size";
const ANN_OPTIMIZED_SIZE: &str = "stellar.io/wasm-optimized-size";
const ANN_REDUCTION_PCT: &str = "stellar.io/wasm-reduction-pct";

/// JSON pointer into a StellarNode spec that holds the base64-encoded WASM.
/// Adjust this path if the CRD schema moves the field.
const SPEC_WASM_FIELD: &str = "/spec/wasmBinary";

// ─────────────────────────────────────────────────────────────────────────────
// Shared state
// ─────────────────────────────────────────────────────────────────────────────

/// Shared state injected into the axum handler via `State<Arc<WasmMutatorState>>`.
#[derive(Clone)]
pub struct WasmMutatorState {
    pub optimizer: WasmOptimizer,
}

impl WasmMutatorState {
    /// Construct from environment — reads `WASM_OPT_SIDECAR_URL` etc.
    pub fn from_env() -> Self {
        Self::with_config(OptimizerConfig::default())
    }

    /// Construct with an explicit configuration (useful in tests).
    pub fn with_config(mut config: OptimizerConfig) -> Self {
        // Clamp the optimizer timeout to the webhook budget.
        if config.timeout > WEBHOOK_BUDGET {
            config.timeout = WEBHOOK_BUDGET;
        }
        Self {
            optimizer: WasmOptimizer::new(config),
        }
    }
}

impl std::fmt::Debug for WasmMutatorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmMutatorState").finish_non_exhaustive()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AdmissionReview types (minimal — avoids pulling in kube from controller crate)
// ─────────────────────────────────────────────────────────────────────────────

/// Minimal AdmissionReview wrapper for JSON deserialization.
///
/// We define our own here rather than depending on `kube` from the controller
/// crate so the controller remains lightweight.  The actual handler in
/// `src/webhook/server.rs` (stellar-k8s crate) uses the full kube types.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionReview {
    pub api_version: String,
    pub kind: String,
    pub request: Option<AdmissionRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<AdmissionResponse>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionRequest {
    pub uid: String,
    #[serde(default)]
    pub object: Option<Value>,
    #[serde(default)]
    pub old_object: Option<Value>,
    #[serde(default)]
    pub operation: String,
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionResponse {
    pub uid: String,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AdmissionStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct AdmissionStatus {
    pub code: u16,
    pub message: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Axum handler
// ─────────────────────────────────────────────────────────────────────────────

/// Axum handler for `POST /mutate/wasm`.
///
/// Registered on the webhook server's router as:
/// ```ignore
/// router.route("/mutate/wasm", post(wasm_mutate_handler))
/// ```
#[instrument(skip(state, body), fields(uid = tracing::field::Empty))]
pub async fn wasm_mutate_handler(
    State(state): State<Arc<WasmMutatorState>>,
    Json(review): Json<AdmissionReview>,
) -> impl IntoResponse {
    let request = match &review.request {
        Some(r) => r.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(error_review("", "missing request in AdmissionReview")),
            )
                .into_response();
        }
    };

    tracing::Span::current().record("uid", &request.uid);

    // Dry-run: allow immediately without modification.
    if request.dry_run == Some(true) {
        debug!(uid = %request.uid, "dry-run: skipping WASM optimization");
        return Json(allow_review(&request.uid, vec![])).into_response();
    }

    // Extract the StellarNode object.
    let object = match &request.object {
        Some(o) => o.clone(),
        None => {
            // DELETE operations have no object — allow them through.
            return Json(allow_review(&request.uid, vec![])).into_response();
        }
    };

    // Idempotency check: skip re-optimization if already annotated.
    if is_already_optimized(&object) {
        debug!(
            uid = %request.uid,
            "StellarNode already carries {ANN_OPTIMIZED}=true — skipping"
        );
        return Json(allow_review(&request.uid, vec![])).into_response();
    }

    // Extract the wasmBinary field (base64-encoded in the spec).
    let wasm_b64 = match extract_wasm_binary(&object) {
        Some(b64) => b64,
        None => {
            // No WASM binary present — not a WASM deployment, allow through.
            debug!(uid = %request.uid, "no wasmBinary field — skipping optimization");
            return Json(allow_review(&request.uid, vec![])).into_response();
        }
    };

    // Decode base64 → raw bytes.
    let raw_bytes = match base64::engine::general_purpose::STANDARD.decode(&wasm_b64) {
        Ok(b) => b,
        Err(e) => {
            warn!(uid = %request.uid, err = %e, "failed to base64-decode wasmBinary");
            return (
                StatusCode::BAD_REQUEST,
                Json(error_review(&request.uid, &format!("invalid base64 in wasmBinary: {e}"))),
            )
                .into_response();
        }
    };

    info!(
        uid = %request.uid,
        original_size = raw_bytes.len(),
        "intercepted WASM deployment payload — optimizing"
    );

    // ── Run the optimizer (with budget timeout) ──────────────────────────────
    let opt_result = match tokio::time::timeout(
        WEBHOOK_BUDGET,
        state.optimizer.optimize(&raw_bytes),
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            error!(uid = %request.uid, err = %e, "WASM optimization failed — allowing original");
            // Fail-open: allow the deployment with the original binary.
            return Json(allow_review(
                &request.uid,
                vec![format!("wasm-opt failed, deploying original: {e}")],
            ))
            .into_response();
        }
        Err(_) => {
            error!(
                uid = %request.uid,
                budget_ms = WEBHOOK_BUDGET.as_millis(),
                "WASM optimization timed out — allowing original"
            );
            return Json(allow_review(
                &request.uid,
                vec!["wasm-opt timed out; deploying original binary".to_string()],
            ))
            .into_response();
        }
    };

    // Build JSON Patch and return the mutated review.
    let patch = build_patch(&opt_result);
    let patch_json = match serde_json::to_string(&patch) {
        Ok(s) => s,
        Err(e) => {
            error!(uid = %request.uid, err = %e, "failed to serialize JSON patch");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_review(&request.uid, &format!("patch serialization error: {e}"))),
            )
                .into_response();
        }
    };

    let patch_b64 =
        base64::engine::general_purpose::STANDARD.encode(patch_json.as_bytes());

    info!(
        uid = %request.uid,
        original_size = opt_result.original_size,
        optimized_size = opt_result.optimized_size,
        bytes_saved = opt_result.bytes_saved(),
        reduction_pct = format!("{:.1}%", opt_result.reduction_pct()),
        elapsed_ms = opt_result.elapsed.as_millis(),
        was_optimized = opt_result.was_optimized,
        "WASM optimization complete"
    );

    let response = AdmissionReview {
        api_version: "admission.k8s.io/v1".to_string(),
        kind: "AdmissionReview".to_string(),
        request: None,
        response: Some(AdmissionResponse {
            uid: request.uid.clone(),
            allowed: true,
            patch: Some(patch_b64),
            patch_type: Some("JSONPatch".to_string()),
            status: None,
            warnings: if opt_result.was_optimized {
                None
            } else {
                Some(vec![
                    "wasm-opt produced no reduction; original binary deployed".to_string(),
                ])
            },
        }),
    };

    Json(response).into_response()
}

// ─────────────────────────────────────────────────────────────────────────────
// Patch builder
// ─────────────────────────────────────────────────────────────────────────────

/// Build the JSON Patch that replaces the wasmBinary field and injects metric
/// annotations into the StellarNode manifest.
fn build_patch(result: &OptimizationResult) -> Value {
    let optimized_b64 =
        base64::engine::general_purpose::STANDARD.encode(&result.bytes);

    json!([
        // Replace the WASM binary with the optimized version.
        {
            "op": "replace",
            "path": SPEC_WASM_FIELD,
            "value": optimized_b64
        },
        // Inject metrics annotations.
        {
            "op": "add",
            "path": format!("/metadata/annotations/{}", escape_json_pointer(ANN_OPTIMIZED)),
            "value": "true"
        },
        {
            "op": "add",
            "path": format!("/metadata/annotations/{}", escape_json_pointer(ANN_ORIGINAL_SIZE)),
            "value": result.original_size.to_string()
        },
        {
            "op": "add",
            "path": format!("/metadata/annotations/{}", escape_json_pointer(ANN_OPTIMIZED_SIZE)),
            "value": result.optimized_size.to_string()
        },
        {
            "op": "add",
            "path": format!("/metadata/annotations/{}", escape_json_pointer(ANN_REDUCTION_PCT)),
            "value": format!("{:.1}", result.reduction_pct())
        }
    ])
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions
// ─────────────────────────────────────────────────────────────────────────────

/// Return `true` if the object already carries `stellar.io/wasm-optimized: "true"`.
fn is_already_optimized(object: &Value) -> bool {
    object
        .pointer("/metadata/annotations")
        .and_then(|a| a.get(ANN_OPTIMIZED))
        .and_then(|v| v.as_str())
        == Some("true")
}

/// Extract the base64-encoded wasmBinary field from a StellarNode spec.
fn extract_wasm_binary(object: &Value) -> Option<String> {
    object
        .pointer(SPEC_WASM_FIELD)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Escape a JSON Pointer segment (RFC 6901: `/` → `~1`, `~` → `~0`).
fn escape_json_pointer(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

/// Build a minimal allow `AdmissionReview` with optional warnings.
fn allow_review(uid: &str, warnings: Vec<String>) -> AdmissionReview {
    AdmissionReview {
        api_version: "admission.k8s.io/v1".to_string(),
        kind: "AdmissionReview".to_string(),
        request: None,
        response: Some(AdmissionResponse {
            uid: uid.to_string(),
            allowed: true,
            patch: None,
            patch_type: None,
            status: None,
            warnings: if warnings.is_empty() {
                None
            } else {
                Some(warnings)
            },
        }),
    }
}

/// Build a deny `AdmissionReview` with an error message.
fn error_review(uid: &str, message: &str) -> AdmissionReview {
    AdmissionReview {
        api_version: "admission.k8s.io/v1".to_string(),
        kind: "AdmissionReview".to_string(),
        request: None,
        response: Some(AdmissionResponse {
            uid: uid.to_string(),
            allowed: false,
            patch: None,
            patch_type: None,
            status: Some(AdmissionStatus {
                code: 400,
                message: message.to_string(),
            }),
            warnings: None,
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_review_with_wasm(wasm_b64: &str) -> AdmissionReview {
        AdmissionReview {
            api_version: "admission.k8s.io/v1".to_string(),
            kind: "AdmissionReview".to_string(),
            request: Some(AdmissionRequest {
                uid: "test-uid-1234".to_string(),
                object: Some(json!({
                    "apiVersion": "stellar.org/v1alpha1",
                    "kind": "StellarNode",
                    "metadata": {
                        "name": "test-node",
                        "annotations": {}
                    },
                    "spec": {
                        "wasmBinary": wasm_b64,
                        "nodeType": "SorobanRpc"
                    }
                })),
                old_object: None,
                operation: "CREATE".to_string(),
                dry_run: Some(false),
            }),
            response: None,
        }
    }

    fn make_review_no_wasm() -> AdmissionReview {
        AdmissionReview {
            api_version: "admission.k8s.io/v1".to_string(),
            kind: "AdmissionReview".to_string(),
            request: Some(AdmissionRequest {
                uid: "test-uid-5678".to_string(),
                object: Some(json!({
                    "apiVersion": "stellar.org/v1alpha1",
                    "kind": "StellarNode",
                    "metadata": {"name": "validator"},
                    "spec": {"nodeType": "Validator"}
                })),
                old_object: None,
                operation: "CREATE".to_string(),
                dry_run: Some(false),
            }),
            response: None,
        }
    }

    #[test]
    fn test_escape_json_pointer() {
        assert_eq!(escape_json_pointer("stellar.io/wasm-optimized"), "stellar.io~1wasm-optimized");
        assert_eq!(escape_json_pointer("a~b/c"), "a~0b~1c");
    }

    #[test]
    fn test_is_already_optimized_true() {
        let obj = json!({
            "metadata": {
                "annotations": {
                    "stellar.io/wasm-optimized": "true"
                }
            }
        });
        assert!(is_already_optimized(&obj));
    }

    #[test]
    fn test_is_already_optimized_false() {
        let obj = json!({"metadata": {"annotations": {}}});
        assert!(!is_already_optimized(&obj));
    }

    #[test]
    fn test_extract_wasm_binary_present() {
        let obj = json!({
            "spec": {"wasmBinary": "AGFzbQ=="}
        });
        assert_eq!(extract_wasm_binary(&obj), Some("AGFzbQ==".to_string()));
    }

    #[test]
    fn test_extract_wasm_binary_absent() {
        let obj = json!({"spec": {}});
        assert_eq!(extract_wasm_binary(&obj), None);
    }

    #[test]
    fn test_extract_wasm_binary_empty_string() {
        let obj = json!({"spec": {"wasmBinary": ""}});
        assert_eq!(extract_wasm_binary(&obj), None);
    }

    #[test]
    fn test_allow_review_structure() {
        let r = allow_review("uid-abc", vec![]);
        assert_eq!(r.response.as_ref().unwrap().uid, "uid-abc");
        assert!(r.response.as_ref().unwrap().allowed);
        assert!(r.response.as_ref().unwrap().patch.is_none());
    }

    #[test]
    fn test_allow_review_with_warnings() {
        let r = allow_review("uid-x", vec!["warn1".to_string()]);
        let resp = r.response.unwrap();
        assert!(resp.warnings.unwrap().contains(&"warn1".to_string()));
    }

    #[test]
    fn test_error_review_structure() {
        let r = error_review("uid-err", "something broke");
        let resp = r.response.unwrap();
        assert!(!resp.allowed);
        assert_eq!(resp.status.unwrap().message, "something broke");
    }

    #[test]
    fn test_build_patch_structure() {
        let result = OptimizationResult {
            bytes: vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
            original_size: 1_000_000,
            optimized_size: 500_000,
            was_optimized: true,
            elapsed: std::time::Duration::from_millis(500),
        };
        let patch = build_patch(&result);
        let arr = patch.as_array().unwrap();
        // Replace wasmBinary op
        assert_eq!(arr[0]["op"], "replace");
        assert_eq!(arr[0]["path"], "/spec/wasmBinary");
        // Annotations
        assert_eq!(arr[1]["op"], "add");
        // Reduction pct annotation value
        assert_eq!(arr[4]["value"], "50.0");
    }

    #[test]
    fn test_no_wasm_binary_skips_optimization() {
        let review = make_review_no_wasm();
        let req = review.request.unwrap();
        let obj = req.object.unwrap();
        assert!(extract_wasm_binary(&obj).is_none());
    }

    /// Test the full handler with a valid WASM binary that cannot be reduced by
    /// a missing wasm-opt.  The fail-open path must return `allowed: true`.
    #[tokio::test]
    async fn handler_fail_open_on_missing_wasm_opt() {
        use std::path::PathBuf;
        use std::sync::Arc;

        let cfg = OptimizerConfig {
            wasm_opt_bin: PathBuf::from("/nonexistent/wasm-opt"),
            sidecar_url: None,
            timeout: Duration::from_secs(5),
            ..OptimizerConfig::default()
        };
        let state = Arc::new(WasmMutatorState::with_config(cfg));

        // Valid WASM magic bytes, base64-encoded.
        let tiny_wasm = vec![0x00u8, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        let b64 = base64::engine::general_purpose::STANDARD.encode(&tiny_wasm);

        let review = make_review_with_wasm(&b64);

        let resp = wasm_mutate_handler(
            State(state),
            Json(review),
        )
        .await;

        // Convert to response and check status code.
        use axum::response::IntoResponse as _;
        let http_resp = resp.into_response();
        assert_eq!(http_resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn handler_dry_run_skips_optimization() {
        use std::sync::Arc;

        let state = Arc::new(WasmMutatorState::from_env());
        let mut review = make_review_with_wasm("AGFzbQ==");
        review.request.as_mut().unwrap().dry_run = Some(true);

        let resp = wasm_mutate_handler(State(state), Json(review)).await;
        use axum::response::IntoResponse as _;
        let http_resp = resp.into_response();
        assert_eq!(http_resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn handler_idempotent_skip() {
        use std::sync::Arc;

        let state = Arc::new(WasmMutatorState::from_env());
        let mut review = make_review_with_wasm("AGFzbQ==");
        if let Some(ref mut req) = review.request {
            if let Some(ref mut obj) = req.object {
                obj["metadata"]["annotations"][ANN_OPTIMIZED] = json!("true");
            }
        }

        let resp = wasm_mutate_handler(State(state), Json(review)).await;
        use axum::response::IntoResponse as _;
        let http_resp = resp.into_response();
        assert_eq!(http_resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn handler_bad_base64_returns_400() {
        use std::sync::Arc;

        let state = Arc::new(WasmMutatorState::from_env());
        let review = make_review_with_wasm("not-valid-base64!!!");

        let resp = wasm_mutate_handler(State(state), Json(review)).await;
        use axum::response::IntoResponse as _;
        let http_resp = resp.into_response();
        assert_eq!(http_resp.status(), StatusCode::BAD_REQUEST);
    }
}
