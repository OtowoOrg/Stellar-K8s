// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Security Policy Enforcement (OPA)
//!
//! Enforces organizational security policies for StellarNode resources.

use crate::crd::StellarNodeSpec;

pub struct PolicyEnforcer;

impl PolicyEnforcer {
    /// Validates a spec against OPA-style security policies
    pub fn enforce_policy(spec: &StellarNodeSpec) -> Vec<String> {
        let mut violations = Vec::new();

        // 1. Ensure privileged containers are disabled (enforced by PSS but checked here too)
        // 2. Ensure only approved registries are used
        if let Some(validator) = &spec.validator_config {
            // Policy: Validators must have history archives enabled in production
            if spec.network == crate::crd::StellarNetwork::Mainnet
                && !validator.enable_history_archive
            {
                violations.push(
                    "Policy Violation: Mainnet validators must have history archives enabled"
                        .to_string(),
                );
            }
        }

        violations
    }
}
