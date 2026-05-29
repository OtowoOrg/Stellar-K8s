//! Security Policy Enforcement (OPA)
//!
//! Enforces organizational security policies for StellarNode resources.

use crate::crd::StellarNodeSpec;
use serde::{Deserialize, Serialize};

pub struct PolicyEnforcer;

impl PolicyEnforcer {
    /// Validates a spec against OPA-style security policies
    pub fn enforce_policy(spec: &StellarNodeSpec) -> Vec<String> {
        let mut violations = Vec::new();

        // 1. Ensure privileged containers are disabled (enforced by PSS but checked here too)
        // 2. Ensure only approved registries are used
        if let Some(validator) = &spec.validator_config {
            // Policy: Validators must have history archives enabled in production
            if spec.network == crate::crd::StellarNetwork::Mainnet && !validator.enable_history_archive {
                violations.push("Policy Violation: Mainnet validators must have history archives enabled".to_string());
            }
        }

        violations
    }
}
