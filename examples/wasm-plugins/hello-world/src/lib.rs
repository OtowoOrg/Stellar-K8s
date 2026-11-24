//! Hello World — Stellar-K8s Wasm Validation Plugin
//!
//! This is the tutorial plugin referenced by
//! `examples/wasm-plugins/hello-world/README.md`.
//!
//! # What it does
//!
//! For every CREATE or UPDATE on a StellarNode this plugin checks two rules:
//!
//! 1. **Mainnet replica requirement** — a StellarNode with `spec.network == "Mainnet"`
//!    must have `spec.replicas >= 3`.
//! 2. **Required label** — every StellarNode must carry a `cost-center` label.
//!
//! All other operations (DELETE, CONNECT) are passed through without inspection.
//!
//! # Host ABI
//!
//! The four host functions below are provided by the Stellar-K8s Wasmtime runtime
//! and imported from the `env` module.  See `docs/plugins/wasm-api.md` for the
//! complete specification.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Host function imports
// ---------------------------------------------------------------------------

extern "C" {
    /// Returns the byte length of the JSON input waiting in the host buffer.
    fn get_input_len() -> i32;

    /// Copies up to `len` bytes from the host input buffer into guest memory at
    /// `ptr`.  Returns the number of bytes actually copied, or -1 on error.
    fn read_input(ptr: *mut u8, len: i32) -> i32;

    /// Copies `len` bytes from guest memory at `ptr` into the host output
    /// buffer.  Returns 0 on success, -1 on error.
    fn write_output(ptr: *const u8, len: i32) -> i32;

    /// Emits a UTF-8 debug log line tagged `wasm_plugin` in the operator logs.
    fn log_message(ptr: *const u8, len: i32);
}

// ---------------------------------------------------------------------------
// Data types — mirrors the structs in src/webhook/types.rs
// ---------------------------------------------------------------------------

/// Incoming admission request delivered by the runtime.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidationInput {
    /// Kubernetes operation: "CREATE", "UPDATE", "DELETE", or "CONNECT".
    operation: String,
    /// The resource being admitted (new state for CREATE / UPDATE).
    object: Option<serde_json::Value>,
    /// Kubernetes namespace of the resource.
    #[allow(dead_code)]
    namespace: String,
    /// Name of the resource.
    #[allow(dead_code)]
    name: String,
    /// Identity of the user making the request.
    #[allow(dead_code)]
    user_info: UserInfo,
    /// Operator-injected context (unused in this plugin).
    #[allow(dead_code)]
    #[serde(default)]
    context: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct UserInfo {
    username: String,
    uid: Option<String>,
    groups: Vec<String>,
    extra: BTreeMap<String, Vec<String>>,
}

/// Decision returned to the runtime.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationOutput {
    /// `true` to allow the request, `false` to deny it.
    allowed: bool,
    /// Human-readable summary (shown in `kubectl` error output when denied).
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    /// Machine-readable reason code.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// Per-field validation errors.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<ValidationError>,
    /// Non-blocking advisory messages.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    /// Key/value pairs written to the Kubernetes audit log.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    audit_annotations: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationError {
    /// Dot-notation path to the offending field (e.g. "spec.replicas").
    field: String,
    /// Description of the problem.
    message: String,
}

// ---------------------------------------------------------------------------
// Plugin entry point
// ---------------------------------------------------------------------------

/// Called by the Stellar-K8s runtime once per admission request.
///
/// Return value:
/// - `0`  validation succeeded (runtime reads `allowed` from the JSON output)
/// - `1`  validation failed    (runtime reads `allowed` from the JSON output)
/// - other — treated as a plugin internal error; request is denied
#[no_mangle]
pub extern "C" fn validate() -> i32 {
    // Step 1 — read the JSON input from the host buffer.
    let input = match read_validation_input() {
        Ok(v) => v,
        Err(msg) => {
            log(&format!("hello-world: failed to read input: {msg}"));
            write_denied(&format!("plugin error: {msg}"), "PluginError");
            return 1;
        }
    };

    log(&format!(
        "hello-world: validating {} operation",
        input.operation
    ));

    // Step 2 — pass DELETE / CONNECT straight through.
    if input.operation != "CREATE" && input.operation != "UPDATE" {
        write_allowed("skipped non-mutating operation", &input.operation);
        return 0;
    }

    // Step 3 — require an object (should always be present for CREATE/UPDATE).
    let object = match &input.object {
        Some(o) => o,
        None => {
            write_denied("no object in request", "InvalidInput");
            return 1;
        }
    };

    // Step 4 — apply policy rules.
    let output = apply_policy(object);

    // Step 5 — serialise and write the output back to the host.
    let rc = if output.allowed { 0 } else { 1 };
    write_output_struct(&output);
    rc
}

// ---------------------------------------------------------------------------
// Policy logic
// ---------------------------------------------------------------------------

fn apply_policy(object: &serde_json::Value) -> ValidationOutput {
    let mut errors: Vec<ValidationError> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut audit = BTreeMap::new();

    // --- Rule 1: required "cost-center" label ----------------------------
    let has_cost_center = object
        .pointer("/metadata/labels/cost-center")
        .map(|v| !v.as_str().unwrap_or("").is_empty())
        .unwrap_or(false);

    if !has_cost_center {
        errors.push(ValidationError {
            field: "metadata.labels.cost-center".into(),
            message: "label \"cost-center\" is required on every StellarNode".into(),
        });
    }

    // --- Rule 2: Mainnet nodes need at least 3 replicas ------------------
    let network = object
        .pointer("/spec/network")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let replicas = object
        .pointer("/spec/replicas")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);

    log(&format!(
        "hello-world: network={network}, replicas={replicas}"
    ));

    if network == "Mainnet" && replicas < 3 {
        errors.push(ValidationError {
            field: "spec.replicas".into(),
            message: format!(
                "Mainnet StellarNodes must have spec.replicas >= 3, got {replicas}"
            ),
        });
    }

    // --- Advisory: low memory limit --------------------------------------
    let memory_str = object
        .pointer("/spec/resources/limits/memory")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if !memory_str.is_empty() {
        if let Some(mib) = parse_memory_mib(memory_str) {
            if mib < 512 {
                warnings.push(format!(
                    "spec.resources.limits.memory is {memory_str}; \
                     consider at least 512Mi for production nodes"
                ));
            }
        }
    }

    // --- Audit annotation ------------------------------------------------
    audit.insert(
        "hello-world.stellar.org/checked".into(),
        "true".into(),
    );
    audit.insert(
        "hello-world.stellar.org/network".into(),
        network.to_string(),
    );

    let allowed = errors.is_empty();
    let message = if allowed {
        Some("hello-world: all checks passed".into())
    } else {
        Some(
            errors
                .iter()
                .map(|e| e.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
        )
    };

    ValidationOutput {
        allowed,
        message,
        reason: if allowed {
            None
        } else {
            Some("PolicyViolation".into())
        },
        errors,
        warnings,
        audit_annotations: audit,
    }
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

/// Read and deserialise the JSON input from the host buffer.
fn read_validation_input() -> Result<ValidationInput, String> {
    unsafe {
        let len = get_input_len();
        if len <= 0 {
            return Err(format!("get_input_len returned {len}"));
        }

        let mut buf = vec![0u8; len as usize];
        let read = read_input(buf.as_mut_ptr(), len);
        if read != len {
            return Err(format!(
                "read_input: expected {len} bytes, got {read}"
            ));
        }

        serde_json::from_slice(&buf)
            .map_err(|e| format!("JSON parse error: {e}"))
    }
}

/// Serialise `output` and hand it to the host.
fn write_output_struct(output: &ValidationOutput) {
    match serde_json::to_vec(output) {
        Ok(json) => unsafe {
            write_output(json.as_ptr(), json.len() as i32);
        },
        Err(e) => log(&format!("hello-world: failed to serialise output: {e}")),
    }
}

/// Shorthand: write a simple allowed response.
fn write_allowed(message: &str, operation: &str) {
    let mut audit = BTreeMap::new();
    audit.insert("hello-world.stellar.org/checked".into(), "true".into());
    audit.insert(
        "hello-world.stellar.org/skipped-operation".into(),
        operation.to_string(),
    );
    write_output_struct(&ValidationOutput {
        allowed: true,
        message: Some(message.into()),
        reason: None,
        errors: vec![],
        warnings: vec![],
        audit_annotations: audit,
    });
}

/// Shorthand: write a simple denied response.
fn write_denied(message: &str, reason: &str) {
    write_output_struct(&ValidationOutput {
        allowed: false,
        message: Some(message.into()),
        reason: Some(reason.into()),
        errors: vec![],
        warnings: vec![],
        audit_annotations: BTreeMap::new(),
    });
}

/// Emit a debug log line via the host.
fn log(msg: &str) {
    unsafe { log_message(msg.as_ptr(), msg.len() as i32) }
}

// ---------------------------------------------------------------------------
// Memory parser helper
// ---------------------------------------------------------------------------

/// Parse a Kubernetes memory string (e.g. "256Mi", "2Gi") and return MiB.
fn parse_memory_mib(s: &str) -> Option<u64> {
    if let Some(n) = s.strip_suffix("Gi") {
        n.parse::<u64>().ok().map(|v| v * 1024)
    } else if let Some(n) = s.strip_suffix("Mi") {
        n.parse::<u64>().ok()
    } else if let Some(n) = s.strip_suffix("Ki") {
        n.parse::<u64>().ok().map(|v| v / 1024)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Unit tests  (run with `cargo test`, not in Wasm)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_node(network: &str, replicas: i64, cost_center: Option<&str>) -> serde_json::Value {
        let mut labels = serde_json::Map::new();
        if let Some(cc) = cost_center {
            labels.insert("cost-center".into(), json!(cc));
        }
        json!({
            "metadata": { "labels": labels },
            "spec": {
                "network": network,
                "replicas": replicas,
                "resources": { "limits": { "memory": "4Gi" } }
            }
        })
    }

    #[test]
    fn mainnet_with_3_replicas_and_label_is_allowed() {
        let obj = make_node("Mainnet", 3, Some("eng"));
        let out = apply_policy(&obj);
        assert!(out.allowed, "expected allowed, got: {:?}", out.message);
        assert!(out.errors.is_empty());
    }

    #[test]
    fn mainnet_with_1_replica_is_denied() {
        let obj = make_node("Mainnet", 1, Some("eng"));
        let out = apply_policy(&obj);
        assert!(!out.allowed);
        assert!(out.errors.iter().any(|e| e.field == "spec.replicas"));
    }

    #[test]
    fn missing_cost_center_is_denied() {
        let obj = make_node("Testnet", 1, None);
        let out = apply_policy(&obj);
        assert!(!out.allowed);
        assert!(out
            .errors
            .iter()
            .any(|e| e.field.contains("cost-center")));
    }

    #[test]
    fn multiple_violations_reported_together() {
        let obj = make_node("Mainnet", 1, None); // missing label AND too few replicas
        let out = apply_policy(&obj);
        assert!(!out.allowed);
        assert_eq!(out.errors.len(), 2);
    }

    #[test]
    fn testnet_with_1_replica_is_allowed() {
        let obj = make_node("Testnet", 1, Some("research"));
        let out = apply_policy(&obj);
        assert!(out.allowed);
    }

    #[test]
    fn low_memory_produces_warning() {
        let obj = json!({
            "metadata": { "labels": { "cost-center": "eng" } },
            "spec": {
                "network": "Testnet",
                "replicas": 1,
                "resources": { "limits": { "memory": "128Mi" } }
            }
        });
        let out = apply_policy(&obj);
        assert!(out.allowed);
        assert!(!out.warnings.is_empty());
    }

    #[test]
    fn parse_memory_mib_works() {
        assert_eq!(parse_memory_mib("1Gi"), Some(1024));
        assert_eq!(parse_memory_mib("512Mi"), Some(512));
        assert_eq!(parse_memory_mib("1024Ki"), Some(1));
        assert_eq!(parse_memory_mib("unknown"), None);
    }
}
