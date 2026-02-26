//! Image Registry Validator Plugin
//!
//! This Wasm plugin validates that StellarNode resources only use
//! approved container image registries.
//!
//! # Example Policy
//!
//! Only allow images from:
//! - docker.io/stellar/*
//! - ghcr.io/stellar/*
//! - gcr.io/stellar-project/*

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Validation input from the webhook
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidationInput {
    operation: String,
    object: Option<serde_json::Value>,
    #[allow(dead_code)]
    old_object: Option<serde_json::Value>,
    #[allow(dead_code)]
    namespace: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    user_info: UserInfo,
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

/// Validation output returned to the webhook
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationOutput {
    allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    errors: Vec<ValidationError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    audit_annotations: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationError {
    field: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_type: Option<String>,
}

/// Approved image registries
const APPROVED_REGISTRIES: &[&str] = &[
    "docker.io/stellar/",
    "ghcr.io/stellar/",
    "gcr.io/stellar-project/",
];

/// Host functions provided by the webhook runtime
extern "C" {
    fn get_input_len() -> i32;
    fn read_input(ptr: *mut u8, len: i32) -> i32;
    fn write_output(ptr: *const u8, len: i32) -> i32;
    fn log_message(ptr: *const u8, len: i32);
}

/// Main validation entry point
#[no_mangle]
pub extern "C" fn validate() -> i32 {
    // Read input from host
    let input = match read_validation_input() {
        Ok(input) => input,
        Err(e) => {
            log(&format!("Failed to read input: {}", e));
            write_validation_output(&ValidationOutput {
                allowed: false,
                message: Some(format!("Plugin error: {}", e)),
                reason: Some("PluginError".to_string()),
                errors: vec![],
                warnings: vec![],
                audit_annotations: BTreeMap::new(),
            });
            return 1;
        }
    };

    // Perform validation
    let output = validate_stellar_node(&input);

    // Write output to host
    write_validation_output(&output);

    if output.allowed {
        0
    } else {
        1
    }
}

/// Read validation input from the host
fn read_validation_input() -> Result<ValidationInput, String> {
    unsafe {
        let len = get_input_len();
        if len <= 0 {
            return Err("Invalid input length".to_string());
        }

        let mut buffer = vec![0u8; len as usize];
        let read = read_input(buffer.as_mut_ptr(), len);
        if read != len {
            return Err(format!("Failed to read input: expected {}, got {}", len, read));
        }

        serde_json::from_slice(&buffer).map_err(|e| format!("Failed to parse input: {}", e))
    }
}

/// Write validation output to the host
fn write_validation_output(output: &ValidationOutput) {
    let json = match serde_json::to_vec(output) {
        Ok(json) => json,
        Err(e) => {
            log(&format!("Failed to serialize output: {}", e));
            return;
        }
    };

    unsafe {
        write_output(json.as_ptr(), json.len() as i32);
    }
}

/// Log a message to the host
fn log(message: &str) {
    unsafe {
        log_message(message.as_ptr(), message.len() as i32);
    }
}

/// Validate a StellarNode resource
fn validate_stellar_node(input: &ValidationInput) -> ValidationOutput {
    log(&format!("Validating {} operation", input.operation));

    // Only validate CREATE and UPDATE operations
    if input.operation != "CREATE" && input.operation != "UPDATE" {
        return ValidationOutput {
            allowed: true,
            message: Some(format!("Skipping {} operation", input.operation)),
            reason: None,
            errors: vec![],
            warnings: vec![],
            audit_annotations: BTreeMap::new(),
        };
    }

    let Some(object) = &input.object else {
        return ValidationOutput {
            allowed: false,
            message: Some("No object provided".to_string()),
            reason: Some("InvalidInput".to_string()),
            errors: vec![],
            warnings: vec![],
            audit_annotations: BTreeMap::new(),
        };
    };

    // Extract spec.version (which contains the image tag)
    let version = object
        .get("spec")
        .and_then(|spec| spec.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    log(&format!("Checking version: {}", version));

    // For this example, we'll check if the version looks like it could be from an approved registry
    // In a real implementation, you'd parse the full image reference
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // Check if version contains a registry prefix
    if version.contains('/') {
        let is_approved = APPROVED_REGISTRIES
            .iter()
            .any(|registry| version.starts_with(registry));

        if !is_approved {
            errors.push(ValidationError {
                field: "spec.version".to_string(),
                message: format!(
                    "Image '{}' is not from an approved registry. Approved registries: {}",
                    version,
                    APPROVED_REGISTRIES.join(", ")
                ),
                error_type: Some("InvalidRegistry".to_string()),
            });
        } else {
            log(&format!("Image '{}' is from an approved registry", version));
        }
    } else {
        // No registry specified, assume docker.io
        warnings.push(format!(
            "No registry specified in version '{}', assuming docker.io",
            version
        ));
    }

    // Check resource limits (example of additional validation)
    if let Some(spec) = object.get("spec") {
        if let Some(resources) = spec.get("resources") {
            if let Some(limits) = resources.get("limits") {
                if let Some(memory) = limits.get("memory").and_then(|m| m.as_str()) {
                    // Parse memory limit (simple check for demonstration)
                    if memory.ends_with("Mi") || memory.ends_with("Gi") {
                        let value: String = memory.chars().take_while(|c| c.is_numeric()).collect();
                        if let Ok(mem_value) = value.parse::<i32>() {
                            if memory.ends_with("Mi") && mem_value < 512 {
                                warnings.push(format!(
                                    "Memory limit {} is quite low, consider at least 512Mi",
                                    memory
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    let allowed = errors.is_empty();
    let message = if !allowed {
        Some(
            errors
                .iter()
                .map(|e| e.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
        )
    } else {
        Some("Validation passed".to_string())
    };

    let mut audit_annotations = BTreeMap::new();
    audit_annotations.insert(
        "image-registry-validator.stellar.org/checked".to_string(),
        "true".to_string(),
    );
    if !version.is_empty() {
        audit_annotations.insert(
            "image-registry-validator.stellar.org/version".to_string(),
            version.to_string(),
        );
    }

    ValidationOutput {
        allowed,
        message,
        reason: if allowed {
            None
        } else {
            Some("PolicyViolation".to_string())
        },
        errors,
        warnings,
        audit_annotations,
    }
}
