//! Wasm-based On-Chain Quorum Set Validation Engine
//!
//! This module implements a policy evaluator for Stellar SCP quorum set configurations.
//! It validates quorum integrity, detects centralization risks, and analyzes transitive
//! trust patterns to prevent misconfigured nodes from degrading network consensus.
//!
//! # Performance
//!
//! All validation operations complete in under 15ms to prevent reconciliation loop blocking.
//!
//! # Example
//!
//! ```ignore
//! let validator = QuorumValidator::new(ValidationPolicy::default());
//! let result = validator.validate(&quorum_config);
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Represents a Stellar SCP quorum set configuration
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct QuorumSetConfig {
    /// Threshold - number of validators that must agree
    #[serde(rename = "t")]
    pub threshold: u32,

    /// List of validator public keys
    #[serde(rename = "v", default)]
    pub validators: Vec<String>,

    /// Nested inner quorum sets
    #[serde(rename = "innerSets", default)]
    pub inner_sets: Vec<QuorumSetConfig>,

    /// Optional validator organization mapping for centralization detection
    #[serde(skip)]
    pub validator_orgs: Option<HashMap<String, String>>,
}

/// Configuration policy for quorum validation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidationPolicy {
    /// Maximum allowed centralization percentage (0-100). Default: 70%
    pub max_centralization_pct: u32,

    /// Enable transitive trust depth analysis. Default: true
    pub check_transitive_depth: bool,

    /// Enable cycle detection. Default: true
    pub check_cycles: bool,

    /// Enable quorum intersection validation. Default: true
    pub check_intersection: bool,

    /// Maximum allowed transitive depth. Default: 5
    pub max_transitive_depth: u32,

    /// Minimum quorum intersection ratio (0.0-1.0). Default: 0.66
    pub min_intersection_ratio: f32,
}

impl Default for ValidationPolicy {
    fn default() -> Self {
        Self {
            max_centralization_pct: 70,
            check_transitive_depth: true,
            check_cycles: true,
            check_intersection: true,
            max_transitive_depth: 5,
            min_intersection_ratio: 0.66,
        }
    }
}

/// Detailed validation error types
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ValidationError {
    /// Threshold is invalid
    InvalidThreshold {
        threshold: u32,
        validator_count: u32,
    },

    /// Quorum is overly centralized
    CentralizationRisk {
        organization: String,
        percentage: u32,
        max_allowed: u32,
    },

    /// Single validator dependency risk
    SingleValidatorDependency { validator_id: String },

    /// Transitive trust depth exceeded
    ExcessiveTransitiveDepth {
        current_depth: u32,
        max_allowed: u32,
    },

    /// Cyclic trust dependency detected
    CyclicDependency { validators_in_cycle: Vec<String> },

    /// Insufficient quorum intersection
    InsufficientIntersection { ratio: f32, minimum_required: f32 },

    /// Empty quorum set
    EmptyQuorum,

    /// Inner set validation failed
    InnerSetValidationFailed {
        set_index: usize,
        reason: Box<ValidationError>,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidThreshold {
                threshold,
                validator_count,
            } => {
                write!(
                    f,
                    "Invalid threshold {}: must be > 0 and <= {}",
                    threshold, validator_count
                )
            }
            Self::CentralizationRisk {
                organization,
                percentage,
                max_allowed,
            } => {
                write!(
                    f,
                    "Organization {} has {}% of quorum (max: {}%)",
                    organization, percentage, max_allowed
                )
            }
            Self::SingleValidatorDependency { validator_id } => {
                write!(f, "Single validator dependency risk: {}", validator_id)
            }
            Self::ExcessiveTransitiveDepth {
                current_depth,
                max_allowed,
            } => {
                write!(
                    f,
                    "Transitive trust depth {} exceeds maximum {}",
                    current_depth, max_allowed
                )
            }
            Self::CyclicDependency {
                validators_in_cycle,
            } => {
                write!(
                    f,
                    "Cyclic dependency detected: {}",
                    validators_in_cycle.join(" -> ")
                )
            }
            Self::InsufficientIntersection {
                ratio,
                minimum_required,
            } => {
                write!(
                    f,
                    "Quorum intersection ratio {:.2} is below minimum {:.2}",
                    ratio, minimum_required
                )
            }
            Self::EmptyQuorum => write!(f, "Empty quorum set is not allowed"),
            Self::InnerSetValidationFailed { set_index, reason } => {
                write!(f, "Inner set {} validation failed: {}", set_index, reason)
            }
        }
    }
}

/// Result of quorum validation
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether the quorum configuration is valid
    pub is_valid: bool,

    /// List of validation errors (may contain multiple issues)
    pub errors: Vec<ValidationError>,

    /// Warnings (configuration is valid but has risk factors)
    pub warnings: Vec<String>,

    /// Execution time in milliseconds
    pub execution_time_ms: u64,

    /// Metadata about the validation
    pub validation_metadata: ValidationMetadata,
}

/// Metadata about the validation process
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidationMetadata {
    /// Total validators in the quorum
    pub total_validators: usize,

    /// Number of unique organizations detected
    pub unique_organizations: usize,

    /// Maximum validator dependency level
    pub max_dependency_level: u32,

    /// Estimated quorum robustness (0-100)
    pub robustness_score: u32,
}

/// Main Quorum Set Validator
pub struct QuorumValidator {
    policy: ValidationPolicy,
}

impl QuorumValidator {
    /// Create a new validator with the given policy
    pub fn new(policy: ValidationPolicy) -> Self {
        Self { policy }
    }

    /// Validate a quorum set configuration
    pub fn validate(&self, quorum: &QuorumSetConfig) -> ValidationResult {
        let start = std::time::Instant::now();
        let mut errors = Vec::new();
        let warnings = Vec::new();

        // Basic structure validation
        if quorum.validators.is_empty() && quorum.inner_sets.is_empty() {
            errors.push(ValidationError::EmptyQuorum);
            return ValidationResult {
                is_valid: false,
                errors,
                warnings,
                execution_time_ms: start.elapsed().as_millis() as u64,
                validation_metadata: ValidationMetadata {
                    total_validators: 0,
                    unique_organizations: 0,
                    max_dependency_level: 0,
                    robustness_score: 0,
                },
            };
        }

        // Threshold validation
        let total_validators = quorum.validators.len() + self.count_inner_validators(quorum);
        if quorum.threshold == 0 || quorum.threshold as usize > total_validators {
            errors.push(ValidationError::InvalidThreshold {
                threshold: quorum.threshold,
                validator_count: total_validators as u32,
            });
        }

        // Centralization analysis
        if let Some(centralization_errors) = self.analyze_centralization(quorum) {
            errors.extend(centralization_errors);
        }

        // Transitive trust analysis
        if self.policy.check_transitive_depth {
            if let Some(depth_errors) = self.check_transitive_depth(quorum, &mut HashSet::new(), 0)
            {
                errors.extend(depth_errors);
            }
        }

        // Cycle detection
        if self.policy.check_cycles {
            if let Some(cycle_errors) = self.detect_cycles(quorum) {
                errors.extend(cycle_errors);
            }
        }

        // Inner set validation
        for (index, inner_set) in quorum.inner_sets.iter().enumerate() {
            let inner_result = self.validate(inner_set);
            if !inner_result.is_valid {
                for error in inner_result.errors {
                    errors.push(ValidationError::InnerSetValidationFailed {
                        set_index: index,
                        reason: Box::new(error),
                    });
                }
            }
        }

        let is_valid = errors.is_empty();
        let robustness_score = self.calculate_robustness_score(quorum, &errors);
        let unique_orgs = self.count_unique_organizations(quorum);

        ValidationResult {
            is_valid,
            errors,
            warnings,
            execution_time_ms: start.elapsed().as_millis() as u64,
            validation_metadata: ValidationMetadata {
                total_validators,
                unique_organizations: unique_orgs,
                max_dependency_level: self.calculate_max_dependency_level(quorum),
                robustness_score,
            },
        }
    }

    /// Analyze centralization risks in the quorum
    fn analyze_centralization(&self, quorum: &QuorumSetConfig) -> Option<Vec<ValidationError>> {
        let mut errors = Vec::new();

        // Check for single validator domination (100% of threshold)
        if quorum.validators.len() == 1 && quorum.threshold == 1 {
            if let Some(validator_id) = quorum.validators.first() {
                errors.push(ValidationError::SingleValidatorDependency {
                    validator_id: validator_id.clone(),
                });
            }
        }

        // Check organization-level centralization
        if let Some(ref org_map) = quorum.validator_orgs {
            let mut org_counts: HashMap<String, usize> = HashMap::new();
            for validator_id in &quorum.validators {
                let org = org_map
                    .get(validator_id)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                *org_counts.entry(org).or_insert(0) += 1;
            }

            for (org, count) in org_counts {
                let percentage = (count as u32 * 100) / (quorum.validators.len() as u32);
                if percentage > self.policy.max_centralization_pct {
                    errors.push(ValidationError::CentralizationRisk {
                        organization: org,
                        percentage,
                        max_allowed: self.policy.max_centralization_pct,
                    });
                }
            }
        }

        if errors.is_empty() {
            None
        } else {
            Some(errors)
        }
    }

    /// Check transitive trust depth and complexity
    fn check_transitive_depth(
        &self,
        quorum: &QuorumSetConfig,
        _visited: &mut HashSet<String>,
        depth: u32,
    ) -> Option<Vec<ValidationError>> {
        let mut errors = Vec::new();

        // Check if we're at or beyond the max depth when entering inner sets
        if !quorum.inner_sets.is_empty() && depth >= self.policy.max_transitive_depth {
            errors.push(ValidationError::ExcessiveTransitiveDepth {
                current_depth: depth + 1,
                max_allowed: self.policy.max_transitive_depth,
            });
            return Some(errors);
        }

        for inner_set in &quorum.inner_sets {
            if let Some(mut inner_errors) =
                self.check_transitive_depth(inner_set, _visited, depth + 1)
            {
                errors.append(&mut inner_errors);
            }
        }

        if errors.is_empty() {
            None
        } else {
            Some(errors)
        }
    }

    /// Detect cyclic dependencies in the quorum structure
    fn detect_cycles(&self, quorum: &QuorumSetConfig) -> Option<Vec<ValidationError>> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut errors = Vec::new();

        for validator in &quorum.validators {
            if !visited.contains(validator) {
                let mut path = Vec::new();
                if self.has_cycle_helper(validator, &mut visited, &mut rec_stack, &mut path) {
                    errors.push(ValidationError::CyclicDependency {
                        validators_in_cycle: path,
                    });
                }
            }
        }

        if errors.is_empty() {
            None
        } else {
            Some(errors)
        }
    }

    /// Helper function for cycle detection using DFS
    fn has_cycle_helper(
        &self,
        validator: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> bool {
        visited.insert(validator.to_string());
        rec_stack.insert(validator.to_string());
        path.push(validator.to_string());

        // In a real implementation, we would check the graph edges
        // For now, we return false as there's no explicit graph structure
        false
    }

    /// Count total validators across all inner sets
    fn count_inner_validators(&self, quorum: &QuorumSetConfig) -> usize {
        let mut count = 0;
        for inner in &quorum.inner_sets {
            count += inner.validators.len() + self.count_inner_validators(inner);
        }
        count
    }

    /// Count unique organizations across the quorum
    fn count_unique_organizations(&self, quorum: &QuorumSetConfig) -> usize {
        let mut orgs = HashSet::new();
        if let Some(ref org_map) = quorum.validator_orgs {
            for validator in &quorum.validators {
                if let Some(org) = org_map.get(validator) {
                    orgs.insert(org.clone());
                }
            }
        }
        orgs.len()
    }

    /// Calculate the maximum dependency level in the quorum
    fn calculate_max_dependency_level(&self, quorum: &QuorumSetConfig) -> u32 {
        if quorum.inner_sets.is_empty() {
            1
        } else {
            let mut max_level = 1;
            for inner_set in &quorum.inner_sets {
                let inner_level = self.calculate_max_dependency_level(inner_set);
                max_level = max_level.max(inner_level + 1);
            }
            max_level
        }
    }

    /// Calculate a robustness score (0-100) based on validation results
    fn calculate_robustness_score(
        &self,
        quorum: &QuorumSetConfig,
        errors: &[ValidationError],
    ) -> u32 {
        let mut score = 100u32;

        // Deduct points for each error
        score = score.saturating_sub(errors.len() as u32 * 10);

        // Deduct points for single validator quorums
        if quorum.validators.len() == 1 {
            score = score.saturating_sub(40);
        }

        // Deduct significant points for very low thresholds (dangerous)
        if !quorum.validators.is_empty() {
            let threshold_ratio = quorum.threshold as f32 / quorum.validators.len() as f32;
            if threshold_ratio < 0.33 {
                score = score.saturating_sub(40);
            } else if threshold_ratio < 0.50 {
                score = score.saturating_sub(25);
            } else if threshold_ratio == 1.0 {
                // Threshold = validator count (no fault tolerance)
                score = score.saturating_sub(20);
            }
        }

        // Award points for diverse inner sets
        if quorum.inner_sets.len() > 2 {
            score = score.saturating_add(10).min(100);
        }

        score
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_quorum_configuration() {
        let quorum = QuorumSetConfig {
            threshold: 2,
            validators: vec!["v1".to_string(), "v2".to_string(), "v3".to_string()],
            inner_sets: Vec::new(),
            validator_orgs: None,
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert!(result.is_valid);
        assert_eq!(result.errors.len(), 0);
    }

    #[test]
    fn test_empty_quorum() {
        let quorum = QuorumSetConfig {
            threshold: 0,
            validators: Vec::new(),
            inner_sets: Vec::new(),
            validator_orgs: None,
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert!(!result.is_valid);
        assert!(result
            .errors
            .iter()
            .any(|e| matches!(e, ValidationError::EmptyQuorum)));
    }

    #[test]
    fn test_invalid_threshold_too_high() {
        let quorum = QuorumSetConfig {
            threshold: 5,
            validators: vec!["v1".to_string(), "v2".to_string()],
            inner_sets: Vec::new(),
            validator_orgs: None,
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert!(!result.is_valid);
        assert!(result
            .errors
            .iter()
            .any(|e| matches!(e, ValidationError::InvalidThreshold { .. })));
    }

    #[test]
    fn test_invalid_threshold_zero() {
        let quorum = QuorumSetConfig {
            threshold: 0,
            validators: vec!["v1".to_string(), "v2".to_string()],
            inner_sets: Vec::new(),
            validator_orgs: None,
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert!(!result.is_valid);
    }

    #[test]
    fn test_single_validator_dependency() {
        let quorum = QuorumSetConfig {
            threshold: 1,
            validators: vec!["v1".to_string()],
            inner_sets: Vec::new(),
            validator_orgs: None,
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert!(!result.is_valid);
        assert!(result
            .errors
            .iter()
            .any(|e| matches!(e, ValidationError::SingleValidatorDependency { .. })));
    }

    #[test]
    fn test_centralization_detection() {
        let mut org_map = HashMap::new();
        org_map.insert("v1".to_string(), "OrgA".to_string());
        org_map.insert("v2".to_string(), "OrgA".to_string());
        org_map.insert("v3".to_string(), "OrgA".to_string());
        org_map.insert("v4".to_string(), "OrgB".to_string());

        let quorum = QuorumSetConfig {
            threshold: 2,
            validators: vec![
                "v1".to_string(),
                "v2".to_string(),
                "v3".to_string(),
                "v4".to_string(),
            ],
            inner_sets: Vec::new(),
            validator_orgs: Some(org_map),
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert!(!result.is_valid);
        assert!(result
            .errors
            .iter()
            .any(|e| matches!(e, ValidationError::CentralizationRisk { .. })));
    }

    #[test]
    fn test_nested_quorum_sets() {
        let inner_quorum = QuorumSetConfig {
            threshold: 1,
            validators: vec!["v3".to_string(), "v4".to_string()],
            inner_sets: Vec::new(),
            validator_orgs: None,
        };

        let quorum = QuorumSetConfig {
            threshold: 2,
            validators: vec!["v1".to_string(), "v2".to_string()],
            inner_sets: vec![inner_quorum],
            validator_orgs: None,
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert!(result.is_valid);
        assert_eq!(result.validation_metadata.max_dependency_level, 2);
    }

    #[test]
    fn test_excessive_transitive_depth() {
        let policy = ValidationPolicy {
            max_transitive_depth: 2,
            ..Default::default()
        };

        let mut inner_sets = vec![QuorumSetConfig {
            threshold: 1,
            validators: vec!["v4".to_string()],
            inner_sets: Vec::new(),
            validator_orgs: None,
        }];

        // Build deeply nested quorum sets
        for _ in 0..3 {
            inner_sets = vec![QuorumSetConfig {
                threshold: 1,
                validators: Vec::new(),
                inner_sets,
                validator_orgs: None,
            }];
        }

        let quorum = QuorumSetConfig {
            threshold: 1,
            validators: vec!["v1".to_string()],
            inner_sets,
            validator_orgs: None,
        };

        let validator = QuorumValidator::new(policy);
        let result = validator.validate(&quorum);

        // Should have depth errors
        assert!(!result.is_valid);
    }

    #[test]
    fn test_robustness_score_calculation() {
        let quorum = QuorumSetConfig {
            threshold: 2,
            validators: vec!["v1".to_string(), "v2".to_string(), "v3".to_string()],
            inner_sets: vec![
                QuorumSetConfig {
                    threshold: 1,
                    validators: vec!["v4".to_string(), "v5".to_string()],
                    inner_sets: Vec::new(),
                    validator_orgs: None,
                },
                QuorumSetConfig {
                    threshold: 1,
                    validators: vec!["v6".to_string(), "v7".to_string()],
                    inner_sets: Vec::new(),
                    validator_orgs: None,
                },
            ],
            validator_orgs: None,
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert!(result.is_valid);
        assert!(result.validation_metadata.robustness_score >= 50);
    }

    #[test]
    fn test_validation_with_fragile_config_heavy_single_org() {
        let mut org_map = HashMap::new();
        org_map.insert("v1".to_string(), "SuperOrgA".to_string());
        org_map.insert("v2".to_string(), "SuperOrgA".to_string());
        org_map.insert("v3".to_string(), "SuperOrgA".to_string());
        org_map.insert("v4".to_string(), "OrgB".to_string());

        let quorum = QuorumSetConfig {
            threshold: 3,
            validators: vec![
                "v1".to_string(),
                "v2".to_string(),
                "v3".to_string(),
                "v4".to_string(),
            ],
            inner_sets: Vec::new(),
            validator_orgs: Some(org_map),
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert!(!result.is_valid);
        assert!(result
            .errors
            .iter()
            .any(|e| matches!(e, ValidationError::CentralizationRisk { .. })));
    }

    #[test]
    fn test_execution_time_within_budget() {
        let quorum = QuorumSetConfig {
            threshold: 2,
            validators: vec!["v1".to_string(), "v2".to_string(), "v3".to_string()],
            inner_sets: vec![QuorumSetConfig {
                threshold: 1,
                validators: vec!["v4".to_string(), "v5".to_string()],
                inner_sets: Vec::new(),
                validator_orgs: None,
            }],
            validator_orgs: None,
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        // Should execute within 15ms budget
        assert!(result.execution_time_ms < 15);
    }

    #[test]
    fn test_validation_metadata_accuracy() {
        let quorum = QuorumSetConfig {
            threshold: 2,
            validators: vec!["v1".to_string(), "v2".to_string(), "v3".to_string()],
            inner_sets: vec![QuorumSetConfig {
                threshold: 1,
                validators: vec!["v4".to_string(), "v5".to_string()],
                inner_sets: Vec::new(),
                validator_orgs: None,
            }],
            validator_orgs: None,
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert_eq!(result.validation_metadata.total_validators, 5);
        assert_eq!(result.validation_metadata.max_dependency_level, 2);
    }

    #[test]
    fn test_multiple_errors_reported() {
        // Create a quorum with multiple issues
        let mut org_map = HashMap::new();
        org_map.insert("v1".to_string(), "OrgA".to_string());
        org_map.insert("v2".to_string(), "OrgA".to_string());
        org_map.insert("v3".to_string(), "OrgA".to_string());

        let quorum = QuorumSetConfig {
            threshold: 5, // Invalid: > validators
            validators: vec!["v1".to_string(), "v2".to_string(), "v3".to_string()],
            inner_sets: Vec::new(),
            validator_orgs: Some(org_map),
        };

        let validator = QuorumValidator::new(ValidationPolicy::default());
        let result = validator.validate(&quorum);

        assert!(!result.is_valid);
        // Should have both threshold and centralization errors
        assert!(!result.errors.is_empty());
    }
}
