//! Example StellarNode Validation Plugin
//!
//! This is an example Wasm plugin that validates StellarNode resources.
//! Build with: `cargo build --target wasm32-wasi --release`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Host functions provided by the runtime
extern "C" {
    fn get_input_len() -> i32;
    fn read_input(ptr: *mut u8, len: i32) -> i32;
    fn write_output(ptr: *const u8, len: i32) -> i32;
    fn log_message(ptr: *const u8, len: i32);
}

/// Validation input from the webhook server
#[derive(Debug, Deserialize)]
struct ValidationInput {
    operation: String,
    object: Option<StellarNode>,
    old_object: Option<StellarNode>,
    namespace: Option<String>,
    name: String,
    user_info: Option<UserInfo>,
    #[serde(default)]
    context: HashMap<String, String>,
}

/// StellarNode spec (simplified)
#[derive(Debug, Deserialize)]
struct StellarNode {
    #[serde(rename = "apiVersion")]
    api_version: Option<String>,
    kind: Option<String>,
    metadata: Option<Metadata>,
    spec: Option<StellarNodeSpec>,
}

#[derive(Debug, Deserialize)]
struct Metadata {
    name: Option<String>,
    namespace: Option<String>,
    labels: Option<HashMap<String, String>>,
    annotations: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StellarNodeSpec {
    node_type: Option<String>,
    network: Option<String>,
    replicas: Option<i32>,
    image: Option<String>,
    resources: Option<Resources>,
    horizon_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Resources {
    requests: Option<ResourceRequirements>,
    limits: Option<ResourceRequirements>,
}

#[derive(Debug, Deserialize)]
struct ResourceRequirements {
    cpu: Option<String>,
    memory: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    username: Option<String>,
    groups: Option<Vec<String>>,
}

/// Validation output to return to the webhook server
#[derive(Debug, Serialize)]
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
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    audit_annotations: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
struct ValidationError {
    field: String,
    message: String,
    #[serde(rename = "type")]
    error_type: String,
}

impl ValidationOutput {
    fn allowed() -> Self {
        Self {
            allowed: true,
            message: None,
            reason: None,
            errors: vec![],
            warnings: vec![],
            audit_annotations: HashMap::new(),
        }
    }

    fn denied(message: &str) -> Self {
        Self {
            allowed: false,
            message: Some(message.to_string()),
            reason: None,
            errors: vec![],
            warnings: vec![],
            audit_annotations: HashMap::new(),
        }
    }

    fn with_warning(mut self, warning: &str) -> Self {
        self.warnings.push(warning.to_string());
        self
    }

    fn with_error(mut self, field: &str, message: &str, error_type: &str) -> Self {
        self.errors.push(ValidationError {
            field: field.to_string(),
            message: message.to_string(),
            error_type: error_type.to_string(),
        });
        self
    }
}

/// Helper function to log messages
fn log(msg: &str) {
    unsafe {
        log_message(msg.as_ptr(), msg.len() as i32);
    }
}

/// Read input from the host
fn read_validation_input() -> Result<ValidationInput, String> {
    let len = unsafe { get_input_len() };
    if len <= 0 {
        return Err("No input provided".to_string());
    }

    let mut buffer = vec![0u8; len as usize];
    let read = unsafe { read_input(buffer.as_mut_ptr(), len) };
    if read < 0 {
        return Err("Failed to read input".to_string());
    }

    serde_json::from_slice(&buffer[..read as usize])
        .map_err(|e| format!("Failed to parse input: {}", e))
}

/// Write output to the host
fn write_validation_output(output: &ValidationOutput) -> Result<(), String> {
    let json = serde_json::to_vec(output)
        .map_err(|e| format!("Failed to serialize output: {}", e))?;

    let result = unsafe { write_output(json.as_ptr(), json.len() as i32) };
    if result < 0 {
        return Err("Failed to write output".to_string());
    }

    Ok(())
}

/// Main validation logic
fn validate_stellar_node(input: &ValidationInput) -> ValidationOutput {
    log(&format!("Validating {} operation for {}", input.operation, input.name));

    let node = match &input.object {
        Some(n) => n,
        None => return ValidationOutput::allowed(),
    };

    let spec = match &node.spec {
        Some(s) => s,
        None => return ValidationOutput::denied("StellarNode spec is required"),
    };

    let mut output = ValidationOutput::allowed();
    let mut has_errors = false;

    // Validation Rule 1: Node type must be valid
    if let Some(node_type) = &spec.node_type {
        let valid_types = ["core", "horizon", "soroban-rpc"];
        if !valid_types.contains(&node_type.as_str()) {
            output = output.with_error(
                "spec.nodeType",
                &format!("Invalid node type: {}. Must be one of: {:?}", node_type, valid_types),
                "FieldValueInvalid",
            );
            has_errors = true;
        }
    } else {
        output = output.with_error(
            "spec.nodeType",
            "nodeType is required",
            "FieldRequired",
        );
        has_errors = true;
    }

    // Validation Rule 2: Network must be valid
    if let Some(network) = &spec.network {
        let valid_networks = ["testnet", "mainnet", "futurenet", "standalone"];
        if !valid_networks.contains(&network.as_str()) {
            output = output.with_error(
                "spec.network",
                &format!("Invalid network: {}. Must be one of: {:?}", network, valid_networks),
                "FieldValueInvalid",
            );
            has_errors = true;
        }
    }

    // Validation Rule 3: Replicas must be positive
    if let Some(replicas) = spec.replicas {
        if replicas < 1 {
            output = output.with_error(
                "spec.replicas",
                "Replicas must be at least 1",
                "FieldValueInvalid",
            );
            has_errors = true;
        }
        if replicas > 10 {
            output = output.with_warning("High replica count (>10) may require additional resources");
        }
    }

    // Validation Rule 4: Horizon nodes require horizon_url for soroban-rpc
    if let Some(node_type) = &spec.node_type {
        if node_type == "soroban-rpc" && spec.horizon_url.is_none() {
            output = output.with_error(
                "spec.horizonUrl",
                "horizonUrl is required for soroban-rpc nodes",
                "FieldRequired",
            );
            has_errors = true;
        }
    }

    // Validation Rule 5: Resource requests should not exceed limits
    if let Some(resources) = &spec.resources {
        if let (Some(requests), Some(limits)) = (&resources.requests, &resources.limits) {
            // This is a simplified check - in production, you'd parse the quantities
            if let (Some(req_mem), Some(lim_mem)) = (&requests.memory, &limits.memory) {
                if parse_memory(req_mem) > parse_memory(lim_mem) {
                    output = output.with_error(
                        "spec.resources",
                        "Memory request cannot exceed limit",
                        "FieldValueInvalid",
                    );
                    has_errors = true;
                }
            }
        }
    }

    // Validation Rule 6: Mainnet nodes should have resource limits
    if let Some(network) = &spec.network {
        if network == "mainnet" {
            if spec.resources.is_none() {
                output = output.with_warning("Mainnet nodes should have resource limits defined");
            }
        }
    }

    // Set final allowed status
    if has_errors {
        output.allowed = false;
        output.message = Some("Validation failed".to_string());
    }

    // Add audit annotation
    output.audit_annotations.insert(
        "stellar.io/validated-by".to_string(),
        "example-validator-plugin".to_string(),
    );

    output
}

/// Simple memory parser (returns bytes)
fn parse_memory(mem: &str) -> u64 {
    let mem = mem.trim();
    if mem.ends_with("Gi") {
        mem.trim_end_matches("Gi").parse::<u64>().unwrap_or(0) * 1024 * 1024 * 1024
    } else if mem.ends_with("Mi") {
        mem.trim_end_matches("Mi").parse::<u64>().unwrap_or(0) * 1024 * 1024
    } else if mem.ends_with("Ki") {
        mem.trim_end_matches("Ki").parse::<u64>().unwrap_or(0) * 1024
    } else if mem.ends_with("G") {
        mem.trim_end_matches("G").parse::<u64>().unwrap_or(0) * 1000 * 1000 * 1000
    } else if mem.ends_with("M") {
        mem.trim_end_matches("M").parse::<u64>().unwrap_or(0) * 1000 * 1000
    } else if mem.ends_with("K") {
        mem.trim_end_matches("K").parse::<u64>().unwrap_or(0) * 1000
    } else {
        mem.parse::<u64>().unwrap_or(0)
    }
}

/// Entry point called by the Wasm runtime
#[no_mangle]
pub extern "C" fn validate() -> i32 {
    // Read input
    let input = match read_validation_input() {
        Ok(input) => input,
        Err(e) => {
            log(&format!("Error reading input: {}", e));
            let output = ValidationOutput::denied(&e);
            let _ = write_validation_output(&output);
            return 1;
        }
    };

    // Run validation
    let output = validate_stellar_node(&input);
    let result = if output.allowed { 0 } else { 1 };

    // Write output
    if let Err(e) = write_validation_output(&output) {
        log(&format!("Error writing output: {}", e));
        return 2;
    }

    result
}

// Export memory for the runtime
#[no_mangle]
pub static mut MEMORY: [u8; 65536] = [0; 65536];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_node() {
        let input = ValidationInput {
            operation: "CREATE".to_string(),
            object: Some(StellarNode {
                api_version: Some("stellar.io/v1alpha1".to_string()),
                kind: Some("StellarNode".to_string()),
                metadata: Some(Metadata {
                    name: Some("test-node".to_string()),
                    namespace: Some("default".to_string()),
                    labels: None,
                    annotations: None,
                }),
                spec: Some(StellarNodeSpec {
                    node_type: Some("horizon".to_string()),
                    network: Some("testnet".to_string()),
                    replicas: Some(1),
                    image: None,
                    resources: None,
                    horizon_url: None,
                }),
            }),
            old_object: None,
            namespace: Some("default".to_string()),
            name: "test-node".to_string(),
            user_info: None,
            context: HashMap::new(),
        };

        let output = validate_stellar_node(&input);
        assert!(output.allowed);
    }

    #[test]
    fn test_invalid_node_type() {
        let input = ValidationInput {
            operation: "CREATE".to_string(),
            object: Some(StellarNode {
                api_version: Some("stellar.io/v1alpha1".to_string()),
                kind: Some("StellarNode".to_string()),
                metadata: None,
                spec: Some(StellarNodeSpec {
                    node_type: Some("invalid".to_string()),
                    network: Some("testnet".to_string()),
                    replicas: Some(1),
                    image: None,
                    resources: None,
                    horizon_url: None,
                }),
            }),
            old_object: None,
            namespace: Some("default".to_string()),
            name: "test-node".to_string(),
            user_info: None,
            context: HashMap::new(),
        };

        let output = validate_stellar_node(&input);
        assert!(!output.allowed);
        assert!(!output.errors.is_empty());
    }

    #[test]
    fn test_parse_memory() {
        assert_eq!(parse_memory("1Gi"), 1024 * 1024 * 1024);
        assert_eq!(parse_memory("512Mi"), 512 * 1024 * 1024);
        assert_eq!(parse_memory("1G"), 1000 * 1000 * 1000);
    }
}
