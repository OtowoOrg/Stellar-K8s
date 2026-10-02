// Copyright 2024 Stellar-K8s Contributors
// SPDX-License-Identifier: Apache-2.0
//! WASM Bytecode Optimizer — Mutating Admission Webhook (issue #325)
//!
//! This module provides the [`wasm_mutate_handler`] axum handler and the
//! [`WasmMutatorState`] that are registered on the stellar-webhook server's
//! router at `POST /mutate/wasm`.
//!
//! The handler intercepts StellarNode CREATE/UPDATE operations that carry a
//! `spec.wasmBinary` field (base64-encoded WASM), delegates optimisation to
//! the wasm-opt HTTP sidecar service, and returns a JSON Patch that replaces
//! the raw binary with the optimised version before the object is persisted to
//! etcd.
//!
//! # Environment variables
//!
//! | Variable                | Default        | Description                                    |
//! |-------------------------|----------------|------------------------------------------------|
//! | `WASM_OPT_SIDECAR_URL`  | *(none)*       | URL of the wasm-opt HTTP sidecar (e.g. `http://wasm-opt-sidecar:9080`) |
//! | `WASM_OPT_LEVEL`        | `3`            | Optimization level passed to `wasm-opt -O<n>` |
//! | `WASM_OPT_BIN`          | `wasm-opt`     | Path to `wasm-opt` binary (subprocess fallback) |
//!
//! # Fail-open guarantee
//!
//! If optimisation fails for any reason (timeout, sidecar unavailable, wasm-opt
//! error) the original binary is allowed through with an advisory warning
//! annotation.  The deployment is **never blocked** by the optimizer.
//!
//! # Idempotency
//!
//! Objects already carrying `stellar.io/wasm-optimized: "true"` are passed
//! through without re-optimisation.
//!
//! # Route registration
//!
//! Registered in [`crate::webhook::server::WebhookServer::into_router`]:
//! ```text
//! POST /mutate/wasm  →  wasm_mutate_handler
//! ```
//! The corresponding [`MutatingWebhookConfiguration`] is defined in
//! `charts/stellar-operator/templates/wasm-optimizer.yaml`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use base64::Engine as _;
use serde_json::Value;
use tracing::{debug, error, info, instrument, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Hard budget for the entire mutation step.  Must be < 10 s (K8s deadline).
const WEBHOOK_BUDGET: Duration = Duration::from_secs(9);

/// Default optimizer timeout (≤ WEBHOOK_BUDGET).
const DEFAULT_OPTIMIZER_TIMEOUT: Duration = Duration::from_secs(8);

/// WASM binary magic bytes: `\0asm`.
const WASM_MAGIC: &[u8; 4] = b"\0asm";

/// Annotation set after a successful optimisation run (idempotency guard).
const ANN_OPTIMIZED: &str = "stellar.io/wasm-optimized";
const ANN_ORIGINAL_SIZE: &str = "stellar.io/wasm-original-size";
const ANN_OPTIMIZED_SIZE: &str = "stellar.io/wasm-optimized-size";
const ANN_REDUCTION_PCT: &str = "stellar.io/wasm-reduction-pct";

/// JSON Pointer to the base64 WASM binary field inside a StellarNode spec.
const SPEC_WASM_FIELD: &str = "/spec/wasmBinary";

/// Remote HTTP path on the wasm-opt sidecar service.
const SIDECAR_OPTIMIZE_PATH: &str = "/optimize";

// ─────────────────────────────────────────────────────────────────────────────
// Optimizer
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for the WASM bytecode optimizer.
#[derive(Debug, Clone)]
pub struct OptimizerConfig {
    /// Hard timeout for the optimisation step.
    pub timeout: Duration,
    /// URL of the remote wasm-opt HTTP sidecar.
    /// When `None`, falls back to a local `wasm-opt` subprocess.
    pub sidecar_url: Option<String>,
    /// Optimisation level (`"0"`–`"4"` or `"z"`).
    pub opt_level: String,
    /// Path to the local `wasm-opt` binary.
    pub wasm_opt_bin: PathBuf,
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_OPTIMIZER_TIMEOUT,
            sidecar_url: std::env::var("WASM_OPT_SIDECAR_URL").ok(),
            opt_level: std::env::var("WASM_OPT_LEVEL").unwrap_or_else(|_| "3".to_string()),
            wasm_opt_bin: PathBuf::from(
                std::env::var("WASM_OPT_BIN").unwrap_or_else(|_| "wasm-opt".to_string()),
            ),
        }
    }
}

/// Result of a single optimisation run.
#[derive(Debug, Clone)]
pub struct OptimizationResult {
    /// Optimised (or original) WASM bytes.
    pub bytes: Vec<u8>,
    /// Input size in bytes.
    pub original_size: usize,
    /// Output size in bytes.
    pub optimized_size: usize,
    /// Whether optimisation actually reduced the binary.
    pub was_optimized: bool,
    /// Wall-clock duration of the optimisation step.
    pub elapsed: Duration,
}

impl OptimizationResult {
    /// Size reduction as a percentage of the original.
    pub fn reduction_pct(&self) -> f64 {
        if self.original_size == 0 {
            return 0.0;
        }
        let saved = self.original_size.saturating_sub(self.optimized_size);
        saved as f64 / self.original_size as f64 * 100.0
    }

    /// Absolute bytes saved.
    pub fn bytes_saved(&self) -> usize {
        self.original_size.saturating_sub(self.optimized_size)
    }
}

/// WASM bytecode optimizer — thin wrapper around either a remote HTTP sidecar
/// or a local `wasm-opt` subprocess.
#[derive(Clone)]
pub struct WasmOptimizer {
    config: OptimizerConfig,
    http: reqwest::Client,
}

impl std::fmt::Debug for WasmOptimizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmOptimizer")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl WasmOptimizer {
    /// Create a new optimizer.
    pub fn new(config: OptimizerConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("reqwest client for wasm-opt sidecar");
        Self { config, http }
    }

    /// Optimise `wasm_bytes`, respecting the configured timeout.
    ///
    /// Returns the original bytes if optimisation produced no reduction.
    /// Returns `Err` only for hard failures (timeout, binary not found, bad WASM).
    pub async fn optimize(&self, wasm_bytes: &[u8]) -> anyhow::Result<OptimizationResult> {
        validate_wasm_magic(wasm_bytes)?;

        let original_size = wasm_bytes.len();
        let start = std::time::Instant::now();

        let optimized_bytes = tokio::time::timeout(
            self.config.timeout,
            self.run_optimization(wasm_bytes),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "wasm-opt timed out after {}ms",
                self.config.timeout.as_millis()
            )
        })??;

        let elapsed = start.elapsed();
        let optimized_size = optimized_bytes.len();

        let (final_bytes, was_optimized) = if optimized_size < original_size {
            info!(
                original_size,
                optimized_size,
                bytes_saved = original_size - optimized_size,
                elapsed_ms = elapsed.as_millis(),
                "wasm-opt: optimization successful"
            );
            (optimized_bytes, true)
        } else {
            warn!(original_size, optimized_size, "wasm-opt: no reduction");
            (wasm_bytes.to_vec(), false)
        };

        let final_size = if was_optimized { optimized_size } else { original_size };

        Ok(OptimizationResult {
            bytes: final_bytes,
            original_size,
            optimized_size: final_size,
            was_optimized,
            elapsed,
        })
    }

    async fn run_optimization(&self, wasm_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        if let Some(ref url) = self.config.sidecar_url {
            self.optimize_via_sidecar(url, wasm_bytes).await
        } else {
            self.optimize_via_subprocess(wasm_bytes).await
        }
    }

    /// POST raw WASM bytes to the sidecar HTTP service.
    async fn optimize_via_sidecar(
        &self,
        base_url: &str,
        wasm_bytes: &[u8],
    ) -> anyhow::Result<Vec<u8>> {
        let url = format!(
            "{}{SIDECAR_OPTIMIZE_PATH}?level={}",
            base_url.trim_end_matches('/'),
            self.config.opt_level,
        );
        debug!(url, bytes = wasm_bytes.len(), "sending WASM to sidecar");

        let resp = self
            .http
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/wasm")
            .body(wasm_bytes.to_vec())
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("HTTP request to wasm-opt sidecar failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("wasm-opt sidecar returned {status}: {body}");
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| anyhow::anyhow!("failed to read sidecar response body: {e}"))?;

        validate_wasm_magic(&bytes)
            .map_err(|e| anyhow::anyhow!("sidecar returned invalid WASM: {e}"))?;

        Ok(bytes.to_vec())
    }

    /// Invoke the local `wasm-opt` binary as a subprocess (fallback).
    async fn optimize_via_subprocess(&self, wasm_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        let bin = self.config.wasm_opt_bin.clone();
        let level = self.config.opt_level.clone();
        let input = wasm_bytes.to_vec();

        tokio::task::spawn_blocking(move || run_wasm_opt_subprocess(&bin, &level, &input))
            .await
            .map_err(|e| anyhow::anyhow!("wasm-opt subprocess task panicked: {e}"))?
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Subprocess helper (sync, called from spawn_blocking)
// ─────────────────────────────────────────────────────────────────────────────

fn run_wasm_opt_subprocess(
    bin: &std::path::Path,
    level: &str,
    input: &[u8],
) -> anyhow::Result<Vec<u8>> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut input_tmp = tempfile::Builder::new()
        .suffix(".wasm")
        .tempfile()
        .map_err(|e| anyhow::anyhow!("temp input file: {e}"))?;
    input_tmp
        .write_all(input)
        .map_err(|e| anyhow::anyhow!("write temp input: {e}"))?;
    input_tmp.flush().map_err(|e| anyhow::anyhow!("flush temp input: {e}"))?;

    let output_tmp = tempfile::Builder::new()
        .suffix(".wasm")
        .tempfile()
        .map_err(|e| anyhow::anyhow!("temp output file: {e}"))?;

    let input_path = input_tmp.path().to_owned();
    let output_path = output_tmp.path().to_owned();

    let out = Command::new(bin)
        .arg(&input_path)
        .arg(format!("-O{level}"))
        .arg("--dce")
        .arg("--memory-packing")
        .arg("--remove-unused-module-elements")
        .arg("--duplicate-function-elimination")
        .arg("--output")
        .arg(&output_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| anyhow::anyhow!("failed to spawn wasm-opt ({}): {e}", bin.display()))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("wasm-opt exited {}: {}", out.status, stderr.trim());
    }

    let optimized = std::fs::read(&output_path)
        .map_err(|e| anyhow::anyhow!("read wasm-opt output: {e}"))?;

    validate_wasm_magic(&optimized)?;
    Ok(optimized)
}

/// Validate WASM magic bytes `\0asm`.
pub fn validate_wasm_magic(bytes: &[u8]) -> anyhow::Result<()> {
    if bytes.len() < 4 {
        anyhow::bail!("WASM binary too short ({} bytes)", bytes.len());
    }
    if &bytes[..4] != WASM_MAGIC {
        anyhow::bail!(
            "invalid WASM magic: expected {:?}, got {:?}",
            WASM_MAGIC,
            &bytes[..4]
        );
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared state injected into the axum handler
// ─────────────────────────────────────────────────────────────────────────────

/// State shared across requests for the WASM mutator handler.
#[derive(Clone, Debug)]
pub struct WasmMutatorState {
    pub optimizer: WasmOptimizer,
}

impl WasmMutatorState {
    /// Build from environment variables (used at server startup).
    pub fn from_env() -> Self {
        Self::with_config(OptimizerConfig::default())
    }

    /// Build with an explicit config (useful in unit tests).
    pub fn with_config(mut config: OptimizerConfig) -> Self {
        if config.timeout > WEBHOOK_BUDGET {
            config.timeout = WEBHOOK_BUDGET;
        }
        Self {
            optimizer: WasmOptimizer::new(config),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Minimal AdmissionReview types (avoids pulling kube into the handler)
// ─────────────────────────────────────────────────────────────────────────────

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
// Axum handler — POST /mutate/wasm
// ─────────────────────────────────────────────────────────────────────────────

/// Axum handler for `POST /mutate/wasm`.
///
/// Intercepts StellarNode CREATE/UPDATE operations, optimises the `wasmBinary`
/// field via the wasm-opt sidecar, and returns a JSON Patch.
///
/// **Fail-open**: any error during optimisation allows the original binary
/// through with a warning annotation.
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

    // Dry-run: skip mutation entirely.
    if request.dry_run == Some(true) {
        debug!(uid = %request.uid, "dry-run: skipping WASM optimisation");
        return Json(allow_review(&request.uid, vec![])).into_response();
    }

    let object = match &request.object {
        Some(o) => o.clone(),
        // DELETE or missing object — allow through.
        None => return Json(allow_review(&request.uid, vec![])).into_response(),
    };

    // Idempotency guard.
    if object
        .pointer("/metadata/annotations")
        .and_then(|a| a.get(ANN_OPTIMIZED))
        .and_then(|v| v.as_str())
        == Some("true")
    {
        debug!(uid = %request.uid, "already optimised — skipping");
        return Json(allow_review(&request.uid, vec![])).into_response();
    }

    // Extract wasmBinary field (base64-encoded).
    let wasm_b64 = match object
        .pointer(SPEC_WASM_FIELD)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(b) => b.to_string(),
        None => {
            // Not a WASM deployment — allow through without patching.
            debug!(uid = %request.uid, "no wasmBinary field — skipping");
            return Json(allow_review(&request.uid, vec![])).into_response();
        }
    };

    // Decode base64 → raw bytes.
    let raw_bytes =
        match base64::engine::general_purpose::STANDARD.decode(&wasm_b64) {
            Ok(b) => b,
            Err(e) => {
                warn!(uid = %request.uid, err = %e, "invalid base64 in wasmBinary");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(error_review(
                        &request.uid,
                        &format!("invalid base64 in wasmBinary: {e}"),
                    )),
                )
                    .into_response();
            }
        };

    info!(
        uid = %request.uid,
        original_size = raw_bytes.len(),
        "intercepted WASM deployment — optimising"
    );

    // ── Run optimizer (bounded by WEBHOOK_BUDGET) ────────────────────────────
    let opt_result =
        match tokio::time::timeout(WEBHOOK_BUDGET, state.optimizer.optimize(&raw_bytes)).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                // Fail-open: log the error, allow original binary.
                error!(uid = %request.uid, err = %e, "optimisation failed — fail-open");
                return Json(allow_review(
                    &request.uid,
                    vec![format!("wasm-opt failed; original deployed: {e}")],
                ))
                .into_response();
            }
            Err(_elapsed) => {
                error!(
                    uid = %request.uid,
                    budget_ms = WEBHOOK_BUDGET.as_millis(),
                    "optimisation timed out — fail-open"
                );
                return Json(allow_review(
                    &request.uid,
                    vec!["wasm-opt timed out; original binary deployed".to_string()],
                ))
                .into_response();
            }
        };

    // ── Build JSON Patch ─────────────────────────────────────────────────────
    let optimised_b64 =
        base64::engine::general_purpose::STANDARD.encode(&opt_result.bytes);

    let patch = serde_json::json!([
        {
            "op": "replace",
            "path": SPEC_WASM_FIELD,
            "value": optimised_b64
        },
        {
            "op": "add",
            "path": format!("/metadata/annotations/{}", escape_ptr(ANN_OPTIMIZED)),
            "value": "true"
        },
        {
            "op": "add",
            "path": format!("/metadata/annotations/{}", escape_ptr(ANN_ORIGINAL_SIZE)),
            "value": opt_result.original_size.to_string()
        },
        {
            "op": "add",
            "path": format!("/metadata/annotations/{}", escape_ptr(ANN_OPTIMIZED_SIZE)),
            "value": opt_result.optimized_size.to_string()
        },
        {
            "op": "add",
            "path": format!("/metadata/annotations/{}", escape_ptr(ANN_REDUCTION_PCT)),
            "value": format!("{:.1}", opt_result.reduction_pct())
        }
    ]);

    let patch_str = match serde_json::to_string(&patch) {
        Ok(s) => s,
        Err(e) => {
            error!(uid = %request.uid, err = %e, "patch serialisation failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_review(&request.uid, "patch serialisation error")),
            )
                .into_response();
        }
    };

    let patch_b64 = base64::engine::general_purpose::STANDARD
        .encode(patch_str.as_bytes());

    info!(
        uid = %request.uid,
        original_size = opt_result.original_size,
        optimised_size = opt_result.optimized_size,
        bytes_saved = opt_result.bytes_saved(),
        reduction_pct = format!("{:.1}%", opt_result.reduction_pct()),
        elapsed_ms = opt_result.elapsed.as_millis(),
        was_optimized = opt_result.was_optimized,
        "WASM optimisation complete"
    );

    Json(AdmissionReview {
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
    })
    .into_response()
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions
// ─────────────────────────────────────────────────────────────────────────────

/// Escape a JSON Pointer segment (RFC 6901).
fn escape_ptr(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

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
            warnings: if warnings.is_empty() { None } else { Some(warnings) },
        }),
    }
}

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
    use axum::response::IntoResponse as _;

    fn tiny_wasm() -> Vec<u8> {
        // Minimal valid WASM: magic + version
        vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]
    }

    fn wasm_b64() -> String {
        base64::engine::general_purpose::STANDARD.encode(tiny_wasm())
    }

    fn make_review(wasm_b64: &str, dry_run: bool) -> AdmissionReview {
        AdmissionReview {
            api_version: "admission.k8s.io/v1".to_string(),
            kind: "AdmissionReview".to_string(),
            request: Some(AdmissionRequest {
                uid: "test-uid".to_string(),
                object: Some(serde_json::json!({
                    "spec": {"wasmBinary": wasm_b64},
                    "metadata": {"name": "test", "annotations": {}}
                })),
                operation: "CREATE".to_string(),
                dry_run: Some(dry_run),
            }),
            response: None,
        }
    }

    #[test]
    fn validate_magic_ok() {
        assert!(validate_wasm_magic(&tiny_wasm()).is_ok());
    }

    #[test]
    fn validate_magic_bad() {
        assert!(validate_wasm_magic(b"ELF\x02").is_err());
    }

    #[test]
    fn validate_magic_too_short() {
        assert!(validate_wasm_magic(b"\0").is_err());
    }

    #[test]
    fn escape_ptr_slash() {
        assert_eq!(
            escape_ptr("stellar.io/wasm-optimized"),
            "stellar.io~1wasm-optimized"
        );
    }

    #[test]
    fn escape_ptr_tilde() {
        assert_eq!(escape_ptr("a~b"), "a~0b");
    }

    #[test]
    fn reduction_pct_zero_denominator() {
        let r = OptimizationResult {
            bytes: vec![],
            original_size: 0,
            optimized_size: 0,
            was_optimized: false,
            elapsed: Duration::ZERO,
        };
        assert_eq!(r.reduction_pct(), 0.0);
    }

    #[test]
    fn reduction_pct_40_percent() {
        let r = OptimizationResult {
            bytes: vec![],
            original_size: 1_000_000,
            optimized_size: 600_000,
            was_optimized: true,
            elapsed: Duration::ZERO,
        };
        assert!((r.reduction_pct() - 40.0).abs() < 0.001);
    }

    #[test]
    fn allow_review_structure() {
        let r = allow_review("u1", vec![]);
        assert!(r.response.as_ref().unwrap().allowed);
        assert!(r.response.as_ref().unwrap().patch.is_none());
    }

    #[test]
    fn error_review_structure() {
        let r = error_review("u2", "oops");
        let resp = r.response.unwrap();
        assert!(!resp.allowed);
        assert_eq!(resp.status.unwrap().code, 400);
    }

    #[tokio::test]
    async fn handler_dry_run_is_200() {
        let state = Arc::new(WasmMutatorState::from_env());
        let review = make_review(&wasm_b64(), true);
        let resp = wasm_mutate_handler(State(state), Json(review))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn handler_bad_base64_is_400() {
        let state = Arc::new(WasmMutatorState::from_env());
        let review = make_review("!!!not-base64!!!", false);
        let resp = wasm_mutate_handler(State(state), Json(review))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handler_no_wasm_field_passes_through() {
        let state = Arc::new(WasmMutatorState::from_env());
        let review = AdmissionReview {
            api_version: "admission.k8s.io/v1".to_string(),
            kind: "AdmissionReview".to_string(),
            request: Some(AdmissionRequest {
                uid: "uid-no-wasm".to_string(),
                object: Some(serde_json::json!({
                    "spec": {"nodeType": "Validator"},
                    "metadata": {"name": "v1", "annotations": {}}
                })),
                operation: "CREATE".to_string(),
                dry_run: Some(false),
            }),
            response: None,
        };
        let resp = wasm_mutate_handler(State(state), Json(review))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn handler_already_optimised_skips() {
        let state = Arc::new(WasmMutatorState::from_env());
        let mut review = make_review(&wasm_b64(), false);
        if let Some(ref mut req) = review.request {
            if let Some(ref mut obj) = req.object {
                obj["metadata"]["annotations"][ANN_OPTIMIZED] = serde_json::json!("true");
            }
        }
        let resp = wasm_mutate_handler(State(state), Json(review))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Fail-open: absent wasm-opt binary must NOT block the deployment.
    #[tokio::test]
    async fn handler_fail_open_when_wasm_opt_absent() {
        let cfg = OptimizerConfig {
            wasm_opt_bin: PathBuf::from("/nonexistent/wasm-opt"),
            sidecar_url: None,
            timeout: Duration::from_secs(3),
            ..OptimizerConfig::default()
        };
        let state = Arc::new(WasmMutatorState::with_config(cfg));
        let review = make_review(&wasm_b64(), false);
        let resp = wasm_mutate_handler(State(state), Json(review))
            .await
            .into_response();
        // Fail-open: 200 OK even though wasm-opt is missing.
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn handler_missing_request_is_400() {
        let state = Arc::new(WasmMutatorState::from_env());
        let review = AdmissionReview {
            api_version: "admission.k8s.io/v1".to_string(),
            kind: "AdmissionReview".to_string(),
            request: None,
            response: None,
        };
        let resp = wasm_mutate_handler(State(state), Json(review))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
