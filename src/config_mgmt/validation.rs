//! Configuration Validation Engine
//!
//! Provides rule-based validation for StellarNode specifications.

use crate::crd::StellarNodeSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRule {
    pub name: String,
    pub description: String,
    pub severity: ValidationSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ValidationSeverity {
    Error,
    Warning,
    Info,
}

pub struct Validator;

impl Validator {
    /// Validates a StellarNodeSpec against a set of advanced rules
    pub fn validate(spec: &StellarNodeSpec) -> Vec<String> {
        let mut errors = Vec::new();

        // 1. Resource consistency check
        if let (Some(req), Some(lim)) = (&spec.resources.requests, &spec.resources.limits) {
            if let (Some(req_cpu), Some(lim_cpu)) = (req.cpu.parse::<f64>().ok(), lim.cpu.parse::<f64>().ok()) {
                if req_cpu > lim_cpu {
                    errors.push("CPU request cannot be greater than limit".to_string());
                }
            }
        }

        // 2. Version safety check
        if spec.version.is_empty() {
            errors.push("Software version must be specified".to_string());
        }

        // 3. Network-specific validation
        if spec.network == crate::crd::StellarNetwork::Custom(_) && spec.custom_network_passphrase.is_none() {
            errors.push("Custom network requires a network passphrase".to_string());
        }

        // 4. Node type specific validation
        if spec.node_type == crate::crd::NodeType::Validator && spec.validator_config.is_none() {
            errors.push("Validator nodes require validator_config".to_string());
        }

        errors
    }
}
