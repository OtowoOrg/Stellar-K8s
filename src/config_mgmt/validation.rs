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
        let req = &spec.resources.requests;
        let lim = &spec.resources.limits;
        if let (Some(req_cpu), Some(lim_cpu)) =
            (parse_cpu_cores(&req.cpu), parse_cpu_cores(&lim.cpu))
        {
            if req_cpu > lim_cpu {
                errors.push("CPU request cannot be greater than limit".to_string());
            }
        }

        // 2. Version safety check
        if spec.version.is_empty() {
            errors.push("Software version must be specified".to_string());
        }

        // 3. Network-specific validation
        if matches!(spec.network, crate::crd::StellarNetwork::Custom(_))
            && spec.custom_network_passphrase.is_none()
        {
            errors.push("Custom network requires a network passphrase".to_string());
        }

        // 4. Node type specific validation
        if spec.node_type == crate::crd::NodeType::Validator && spec.validator_config.is_none() {
            errors.push("Validator nodes require validator_config".to_string());
        }

        errors
    }
}

fn parse_cpu_cores(cpu: &str) -> Option<f64> {
    let trimmed = cpu.trim();
    if let Some(milli) = trimmed.strip_suffix('m') {
        milli.parse::<f64>().ok().map(|v| v / 1000.0)
    } else {
        trimmed.parse::<f64>().ok()
    }
}
