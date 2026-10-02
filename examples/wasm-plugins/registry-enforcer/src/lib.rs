//! Registry Enforcer — Stellar-K8s Wasm Validation Plugin
//!
//! This enterprise-grade plugin enforces an image-registry allow-list on every
//! `StellarNode` CREATE and UPDATE admission request.
//!
//! # Policy
//!
//! `spec.version` must begin with one of the prefixes defined in
//! [`APPROVED_REGISTRIES`].  Requests that reference any other registry prefix
//! are denied with a structured `ValidationError` on `spec.version`.
//!
//! # Host ABI
//!
//! The four host functions declared below are provided by the Stellar-K8s
//! Wasmtime runtime and imported from the `env` module.  See
//! `docs/plugins/wasm-api.md` for the complete specification.
//!
//! # Compile
//!
//! ```bash
//! cargo build --target wasm32-unknown-unknown --release
//! wasm-opt -Oz \
//!   -o target/wasm32-unknown-unknown/release/registry_enforcer.opt.wasm \
//!      target/wasm32-unknown-unknown/release/registry_enforcer.wasm
//! ```
//!
//! # Tests (native target)
//!
//! ```bash
//! cargo test
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Host function imports
// ---------------------------------------------------------------------------

extern "C" {
    /// Returns the byte length of the JSON input waiting in the host buffer.
    /// Returns `< 0` on host error.
    fn get_input_len() -> i32;

    /// Copies up to `len` bytes from the host input buffer into guest memory
    /// at `ptr`.  Returns the number of bytes actually copied, or `< 0` on
    /// error.
    fn read_input(ptr: *mut u8, len: i32) -> i32;

    /// Copies `len` bytes from guest memory at `ptr` into the host output
    /// buffer, replacing any previous content.  Returns `0` on success,
    /// `< 0` on error.  Call **exactly once** per `validate()` invocation.
    fn write_output(ptr: *const u8, len: i32) -> i32;

    /// Emits a UTF-8 `DEBUG`-level log line tagged `wasm_plugin` in the
    /// operator log stream.  Never blocks; never returns an error.
    fn log_message(ptr: *const u8, len: i32);
}

// ---------------------------------------------------------------------------
// Approved-registry configuration
// ---------------------------------------------------------------------------

/// Image-registry prefixes that are allowed for `spec.version`.
///
/// A `spec.version` value is accepted when it starts with **any** entry in
/// this list.  Adjust to match your organisation's approved registries before
/// deploying.
const APPROVED_REGISTRIES: &[&str] = &[
    "docker.io/stellar/",  // official Stellar Foundation images
    "ghcr.io/myorg/",      // your organisation's GitHub Container Registry
    // "artifactory.corp.example.com/stellar/",  // internal Artifactory
];

// ---------------------------------------------------------------------------
// Data types — mirrors the structs in docs/plugins/wasm-api.md
// ---------------------------------------------------------------------------

/// Incoming admission request delivered by the Stellar-K8s runtime.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidationInput {
    /// Kubernetes operation: `"CREATE"`, `"UPDATE"`, `"DELETE"`, or
    /// `"CONNECT"`.
    operation: String,

    /// The resource being admitted (new state for CREATE / UPDATE).
    object: Option<serde_json::Value>,

    /// Kubernetes namespace of the resource.
    namespace: String,

    /// Name of the resource.
    name: String,

    /// Identity of the user making the request.
    #[allow(dead_code)]
    user_info: UserInfo,

    /// Operator-injected context key/value pairs (empty by default).
    #[serde(default)]
    #[allow(dead_code)]
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

/// Decision returned to the Stellar-K8s runtime.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ValidationOutput {
    /// `true` to allow the request, `false` to deny it.
    allowed: bool,

    /// Human-readable summary shown in `kubectl` error output when denied.
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,

    /// Machine-readable reason code.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,

    /// Structured per-field validation errors.
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
    /// Dot-notation path to the offending field (e.g. `"spec.version"`).
    field: String,

    /// Human-readable description of the problem.
    message: String,

    /// One of the `errorType` codes defined in the API reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    error_type: Option<String>,

    /// The actual value that failed validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    invalid_value: Option<String>,
}

// ---------------------------------------------------------------------------
// Plugin entry point
// ---------------------------------------------------------------------------

/// Called by the Stellar-K8s runtime **once per admission request**.
///
/// Return value semantics:
/// - `0`  — validation logic ran to completion (runtime reads `allowed` from output JSON)
/// - `1`  — validation logic ran to completion (runtime reads `allowed` from output JSON)
/// - other — internal plugin error; request is denied regardless of output JSON
///
/// The return code is a secondary signal; the runtime always reads the JSON
/// written by [`write_output`].  Always call [`write_output_struct`] before
/// returning.
#[no_mangle]
pub extern "C" fn validate() -> i32 {
    // Step 1 — read and deserialise the JSON input from the host buffer.
    let input = match read_validation_input() {
        Ok(v) => v,
        Err(msg) => {
            log(&format!("registry-enforcer: failed to read input: {msg}"));
            write_denied(&format!("plugin error: {msg}"), "PluginError");
            return 1;
        }
    };

    log(&format!(
        "registry-enforcer: {} on {}/{}",
        input.operation, input.namespace, input.name
    ));

    // Step 2 — pass DELETE and CONNECT straight through without inspection.
    if input.operation != "CREATE" && input.operation != "UPDATE" {
        write_allowed("non-mutating operation; skipped by registry-enforcer");
        return 0;
    }

    // Step 3 — a CREATE / UPDATE must carry an object; deny if absent.
    let object = match &input.object {
        Some(o) => o,
        None => {
            log("registry-enforcer: no object in request");
            write_denied("no object in request", "InvalidInput");
            return 1;
        }
    };

    // Step 4 — apply the registry allow-list policy.
    let output = enforce_registry_policy(object);

    // Step 5 — serialise the decision and hand it to the runtime.
    let rc = if output.allowed { 0 } else { 1 };
    write_output_struct(&output);
    rc
}

// ---------------------------------------------------------------------------
// Policy logic
// ---------------------------------------------------------------------------

/// Check `spec.version` against [`APPROVED_REGISTRIES`] and return the
/// admission decision.
fn enforce_registry_policy(object: &serde_json::Value) -> ValidationOutput {
    let mut errors: Vec<ValidationError> = Vec::new();
    let mut audit: BTreeMap<String, String> = BTreeMap::new();

    // Always stamp the audit log so operators know this plugin ran.
    audit.insert(
        "registry-enforcer.stellar.org/checked".into(),
        "true".into(),
    );

    // Extract spec.version; treat a missing or empty value as a violation.
    let version = object
        .pointer("/spec/version")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if version.is_empty() {
        errors.push(ValidationError {
            field: "spec.version".into(),
            message: "spec.version is required".into(),
            error_type: Some("Required".into()),
            invalid_value: None,
        });
    } else {
        // Record the version string for audit trail.
        audit.insert(
            "registry-enforcer.stellar.org/version".into(),
            version.to_string(),
        );

        let approved = APPROVED_REGISTRIES
            .iter()
            .any(|prefix| version.starts_with(prefix));

        if !approved {
            let approved_list = APPROVED_REGISTRIES.join(", ");
            errors.push(ValidationError {
                field: "spec.version".into(),
                message: format!(
                    "image registry is not in the approved list; \
                     approved prefixes: {approved_list}"
                ),
                error_type: Some("NotSupported".into()),
                invalid_value: Some(version.to_string()),
            });

            log(&format!(
                "registry-enforcer: denied — unapproved registry in version '{version}'"
            ));
        } else {
            log(&format!(
                "registry-enforcer: allowed — version '{version}' matches approved registry"
            ));
        }
    }

    let allowed = errors.is_empty();
    ValidationOutput {
        allowed,
        message: if allowed {
            Some("registry-enforcer: approved registry".into())
        } else {
            Some(
                errors
                    .iter()
                    .map(|e| e.message.clone())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        },
        reason: if allowed {
            None
        } else {
            Some("PolicyViolation".into())
        },
        errors,
        warnings: Vec::new(),
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

/// Serialise `output` and write it into the host output buffer.
fn write_output_struct(output: &ValidationOutput) {
    match serde_json::to_vec(output) {
        Ok(json) => unsafe {
            write_output(json.as_ptr(), json.len() as i32);
        },
        Err(e) => log(&format!(
            "registry-enforcer: failed to serialise output: {e}"
        )),
    }
}

/// Shorthand: write a simple allowed response with no errors.
fn write_allowed(message: &str) {
    write_output_struct(&ValidationOutput {
        allowed: true,
        message: Some(message.into()),
        ..Default::default()
    });
}

/// Shorthand: write a simple denied response.
fn write_denied(message: &str, reason: &str) {
    write_output_struct(&ValidationOutput {
        allowed: false,
        message: Some(message.into()),
        reason: Some(reason.into()),
        ..Default::default()
    });
}

/// Emit a debug log line via the host runtime.
fn log(msg: &str) {
    unsafe { log_message(msg.as_ptr(), msg.len() as i32) }
}

// ---------------------------------------------------------------------------
// Unit tests (run on the native target, not inside Wasm)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- helpers ------------------------------------------------------------

    fn node_with_version(version: &str) -> serde_json::Value {
        json!({
            "apiVersion": "stellar.org/v1alpha1",
            "kind": "StellarNode",
            "metadata": {
                "name": "test-node",
                "namespace": "stellar",
                "labels": { "cost-center": "platform" }
            },
            "spec": {
                "nodeType": "Validator",
                "network": "Testnet",
                "version": version
            }
        })
    }

    fn node_without_version() -> serde_json::Value {
        json!({
            "apiVersion": "stellar.org/v1alpha1",
            "kind": "StellarNode",
            "metadata": { "name": "test-node", "namespace": "stellar" },
            "spec": { "nodeType": "Validator", "network": "Testnet" }
        })
    }

    // --- tests --------------------------------------------------------------

    #[test]
    fn approved_official_registry_is_allowed() {
        let obj = node_with_version("docker.io/stellar/stellar-core:v21.3.0");
        let out = enforce_registry_policy(&obj);
        assert!(
            out.allowed,
            "expected allowed for official registry, got: {:?}",
            out.message
        );
        assert!(out.errors.is_empty());
        assert_eq!(
            out.audit_annotations
                .get("registry-enforcer.stellar.org/checked"),
            Some(&"true".to_string())
        );
    }

    #[test]
    fn approved_org_registry_is_allowed() {
        let obj = node_with_version("ghcr.io/myorg/stellar-core:latest");
        let out = enforce_registry_policy(&obj);
        assert!(out.allowed, "expected allowed for org registry");
        assert!(out.errors.is_empty());
    }

    #[test]
    fn unapproved_registry_is_denied() {
        let obj = node_with_version("quay.io/someone/stellar-core:v21.3.0");
        let out = enforce_registry_policy(&obj);
        assert!(!out.allowed, "expected denied for unapproved registry");
        assert_eq!(out.reason.as_deref(), Some("PolicyViolation"));
        let err = &out.errors[0];
        assert_eq!(err.field, "spec.version");
        assert_eq!(err.error_type.as_deref(), Some("NotSupported"));
        assert_eq!(
            err.invalid_value.as_deref(),
            Some("quay.io/someone/stellar-core:v21.3.0")
        );
    }

    #[test]
    fn unapproved_registry_docker_hub_without_stellar_path_is_denied() {
        // "docker.io/notstellar/…" must NOT match "docker.io/stellar/"
        let obj = node_with_version("docker.io/notstellar/stellar-core:v21.3.0");
        let out = enforce_registry_policy(&obj);
        assert!(!out.allowed);
    }

    #[test]
    fn missing_version_is_denied_with_required_error() {
        let obj = node_without_version();
        let out = enforce_registry_policy(&obj);
        assert!(!out.allowed);
        let err = &out.errors[0];
        assert_eq!(err.field, "spec.version");
        assert_eq!(err.error_type.as_deref(), Some("Required"));
    }

    #[test]
    fn empty_string_version_is_denied() {
        let obj = node_with_version("");
        let out = enforce_registry_policy(&obj);
        assert!(!out.allowed);
        assert!(out.errors.iter().any(|e| e.field == "spec.version"));
    }

    #[test]
    fn allowed_output_carries_audit_annotation() {
        let obj = node_with_version("docker.io/stellar/stellar-core:v21.3.0");
        let out = enforce_registry_policy(&obj);
        assert!(out.audit_annotations.contains_key(
            "registry-enforcer.stellar.org/checked"
        ));
        assert!(out.audit_annotations.contains_key(
            "registry-enforcer.stellar.org/version"
        ));
    }

    #[test]
    fn denied_output_carries_checked_audit_annotation() {
        let obj = node_with_version("quay.io/bad/stellar-core:v1.0.0");
        let out = enforce_registry_policy(&obj);
        assert!(!out.allowed);
        assert!(out.audit_annotations.contains_key(
            "registry-enforcer.stellar.org/checked"
        ));
    }
}
