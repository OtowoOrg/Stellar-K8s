//! Wasm Plugin Types and Interfaces
//!
//! This module defines the types and interfaces for Wasm-based validation plugins.
//! Plugins receive StellarNode specifications and return validation results.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Plugin metadata describing a Wasm validation plugin
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PluginMetadata {
    /// Unique identifier for the plugin
    pub name: String,

    /// Semantic version of the plugin (e.g., "1.0.0")
    pub version: String,

    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Plugin author
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    /// SHA256 hash of the Wasm binary for integrity verification
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,

    /// Resource limits for plugin execution
    #[serde(default)]
    pub limits: PluginLimits,
}

/// Resource limits for plugin execution sandboxing
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PluginLimits {
    /// Maximum execution time in milliseconds (default: 1000)
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    /// Maximum memory in bytes (default: 16MB)
    #[serde(default = "default_max_memory")]
    pub max_memory_bytes: u64,

    /// Maximum fuel (Wasmtime instruction count limit, default: 1_000_000)
    #[serde(default = "default_max_fuel")]
    pub max_fuel: u64,
}

fn default_timeout_ms() -> u64 {
    1000
}

fn default_max_memory() -> u64 {
    16 * 1024 * 1024 // 16MB
}

fn default_max_fuel() -> u64 {
    1_000_000
}

impl Default for PluginLimits {
    fn default() -> Self {
        Self {
            timeout_ms: default_timeout_ms(),
            max_memory_bytes: default_max_memory(),
            max_fuel: default_max_fuel(),
        }
    }
}

/// Input provided to Wasm validation plugins
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationInput {
    /// The operation being performed (CREATE, UPDATE, DELETE)
    pub operation: Operation,

    /// The StellarNode being validated (new version for CREATE/UPDATE)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<serde_json::Value>,

    /// The old StellarNode (for UPDATE operations)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_object: Option<serde_json::Value>,

    /// Namespace of the resource
    pub namespace: String,

    /// Name of the resource
    pub name: String,

    /// User information from the request
    pub user_info: UserInfo,

    /// Additional context passed to the plugin
    #[serde(default)]
    pub context: BTreeMap<String, String>,
}

/// Kubernetes operation type
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Operation {
    Create,
    Update,
    Delete,
    Connect,
}

impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Operation::Create => write!(f, "CREATE"),
            Operation::Update => write!(f, "UPDATE"),
            Operation::Delete => write!(f, "DELETE"),
            Operation::Connect => write!(f, "CONNECT"),
        }
    }
}

/// User information from the Kubernetes API request
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInfo {
    /// Username of the requester
    pub username: String,

    /// UID of the user
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,

    /// Groups the user belongs to
    #[serde(default)]
    pub groups: Vec<String>,

    /// Extra information (key-value pairs)
    #[serde(default)]
    pub extra: BTreeMap<String, Vec<String>>,
}

/// Output returned by Wasm validation plugins
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationOutput {
    /// Whether the validation passed
    pub allowed: bool,

    /// Human-readable status message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Machine-readable reason code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Detailed validation errors
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ValidationError>,

    /// Warnings (non-blocking issues)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,

    /// Audit annotations to add to the audit log
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub audit_annotations: BTreeMap<String, String>,
}

impl ValidationOutput {
    /// Create an allowed response
    pub fn allowed() -> Self {
        Self {
            allowed: true,
            message: None,
            reason: None,
            errors: Vec::new(),
            warnings: Vec::new(),
            audit_annotations: BTreeMap::new(),
        }
    }

    /// Create an allowed response with warnings
    pub fn allowed_with_warnings(warnings: Vec<String>) -> Self {
        Self {
            allowed: true,
            message: None,
            reason: None,
            errors: Vec::new(),
            warnings,
            audit_annotations: BTreeMap::new(),
        }
    }

    /// Create a denied response
    pub fn denied(message: impl Into<String>) -> Self {
        Self {
            allowed: false,
            message: Some(message.into()),
            reason: Some("ValidationFailed".to_string()),
            errors: Vec::new(),
            warnings: Vec::new(),
            audit_annotations: BTreeMap::new(),
        }
    }

    /// Create a denied response with detailed errors
    pub fn denied_with_errors(errors: Vec<ValidationError>) -> Self {
        let message = errors
            .iter()
            .map(|e| e.message.clone())
            .collect::<Vec<_>>()
            .join("; ");

        Self {
            allowed: false,
            message: Some(message),
            reason: Some("ValidationFailed".to_string()),
            errors,
            warnings: Vec::new(),
            audit_annotations: BTreeMap::new(),
        }
    }

    /// Create an error response for plugin failures
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            allowed: false,
            message: Some(message.into()),
            reason: Some("PluginError".to_string()),
            errors: Vec::new(),
            warnings: Vec::new(),
            audit_annotations: BTreeMap::new(),
        }
    }
}

impl Default for ValidationOutput {
    fn default() -> Self {
        Self::allowed()
    }
}

/// Detailed validation error
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationError {
    /// JSON path to the invalid field (e.g., "spec.replicas")
    pub field: String,

    /// Error message describing the issue
    pub message: String,

    /// Error type/code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<ValidationErrorType>,

    /// Invalid value (for debugging)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_value: Option<serde_json::Value>,
}

impl ValidationError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            error_type: None,
            invalid_value: None,
        }
    }

    pub fn with_type(mut self, error_type: ValidationErrorType) -> Self {
        self.error_type = Some(error_type);
        self
    }

    pub fn with_value(mut self, value: serde_json::Value) -> Self {
        self.invalid_value = Some(value);
        self
    }
}

/// Types of validation errors
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ValidationErrorType {
    /// Required field is missing
    Required,
    /// Value is out of allowed range
    Invalid,
    /// Value exceeds maximum
    TooLarge,
    /// Value is below minimum
    TooSmall,
    /// Value doesn't match pattern
    InvalidPattern,
    /// Value is not in allowed set
    NotSupported,
    /// Duplicate value
    Duplicate,
    /// Field is immutable
    Immutable,
    /// Custom constraint violation
    ConstraintViolation,
}

/// Plugin configuration stored in ConfigMap or CRD
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PluginConfig {
    /// Plugin metadata
    pub metadata: PluginMetadata,

    /// Base64-encoded Wasm binary
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasm_binary: Option<String>,

    /// Reference to a ConfigMap containing the Wasm binary
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_map_ref: Option<ConfigMapRef>,

    /// Reference to a Secret containing the Wasm binary
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<SecretRef>,

    /// URL to download the Wasm binary from
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Operations this plugin validates
    #[serde(default = "default_operations")]
    pub operations: Vec<Operation>,

    /// Whether this plugin is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Fail-open behavior (allow if plugin fails)
    #[serde(default)]
    pub fail_open: bool,

    /// Custom configuration passed to the plugin
    #[serde(default)]
    pub plugin_config: BTreeMap<String, serde_json::Value>,
}

fn default_operations() -> Vec<Operation> {
    vec![Operation::Create, Operation::Update]
}

fn default_true() -> bool {
    true
}

/// Reference to a ConfigMap
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfigMapRef {
    /// Name of the ConfigMap
    pub name: String,
    /// Key containing the Wasm binary
    pub key: String,
    /// Namespace (defaults to webhook namespace)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// Reference to a Secret
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    /// Name of the Secret
    pub name: String,
    /// Key containing the Wasm binary
    pub key: String,
    /// Namespace (defaults to webhook namespace)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// Result of plugin execution
#[derive(Clone, Debug)]
pub struct PluginExecutionResult {
    /// Plugin name
    pub plugin_name: String,

    /// Validation output
    pub output: ValidationOutput,

    /// Execution time in milliseconds
    pub execution_time_ms: u64,

    /// Memory used in bytes
    pub memory_used_bytes: u64,

    /// Fuel consumed
    pub fuel_consumed: u64,
}

/// Aggregated results from all plugins
#[derive(Clone, Debug)]
pub struct AggregatedValidationResult {
    /// Whether all plugins allowed the request
    pub allowed: bool,

    /// Combined message
    pub message: Option<String>,

    /// All errors from all plugins
    pub errors: Vec<ValidationError>,

    /// All warnings from all plugins
    pub warnings: Vec<String>,

    /// Individual plugin results
    pub plugin_results: Vec<PluginExecutionResult>,

    /// Audit annotations from all plugins
    pub audit_annotations: BTreeMap<String, String>,

    /// Total execution time across all plugins in milliseconds
    pub total_execution_time_ms: u64,
}

impl AggregatedValidationResult {
    /// Aggregate results from multiple plugins
    pub fn aggregate(results: Vec<PluginExecutionResult>) -> Self {
        let mut allowed = true;
        let mut messages = Vec::new();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut audit_annotations = BTreeMap::new();

        for result in &results {
            if !result.output.allowed {
                allowed = false;
                if let Some(msg) = &result.output.message {
                    messages.push(format!("[{}] {}", result.plugin_name, msg));
                }
            }

            for error in &result.output.errors {
                errors.push(ValidationError {
                    field: error.field.clone(),
                    message: format!("[{}] {}", result.plugin_name, error.message),
                    error_type: error.error_type.clone(),
                    invalid_value: error.invalid_value.clone(),
                });
            }

            for warning in &result.output.warnings {
                warnings.push(format!("[{}] {}", result.plugin_name, warning));
            }

            for (k, v) in &result.output.audit_annotations {
                audit_annotations.insert(format!("{}/{}", result.plugin_name, k), v.clone());
            }
        }

        let message = if messages.is_empty() {
            None
        } else {
            Some(messages.join("; "))
        };

        let total_execution_time_ms: u64 = results.iter().map(|r| r.execution_time_ms).sum();

        Self {
            allowed,
            message,
            errors,
            warnings,
            plugin_results: results,
            audit_annotations,
            total_execution_time_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_output_allowed() {
        let output = ValidationOutput::allowed();
        assert!(output.allowed);
        assert!(output.message.is_none());
    }

    #[test]
    fn test_validation_output_denied() {
        let output = ValidationOutput::denied("test error");
        assert!(!output.allowed);
        assert_eq!(output.message, Some("test error".to_string()));
    }

    #[test]
    fn test_validation_error() {
        let error = ValidationError::new("spec.replicas", "must be positive")
            .with_type(ValidationErrorType::Invalid)
            .with_value(serde_json::json!(-1));

        assert_eq!(error.field, "spec.replicas");
        assert_eq!(error.error_type, Some(ValidationErrorType::Invalid));
    }

    #[test]
    fn test_aggregate_results() {
        let results = vec![
            PluginExecutionResult {
                plugin_name: "plugin1".to_string(),
                output: ValidationOutput::allowed_with_warnings(vec!["warning1".to_string()]),
                execution_time_ms: 10,
                memory_used_bytes: 1000,
                fuel_consumed: 100,
            },
            PluginExecutionResult {
                plugin_name: "plugin2".to_string(),
                output: ValidationOutput::denied("error from plugin2"),
                execution_time_ms: 20,
                memory_used_bytes: 2000,
                fuel_consumed: 200,
            },
        ];

        let aggregated = AggregatedValidationResult::aggregate(results);
        assert!(!aggregated.allowed);
        assert!(aggregated.message.unwrap().contains("plugin2"));
        assert_eq!(aggregated.warnings.len(), 1);
    }
}
