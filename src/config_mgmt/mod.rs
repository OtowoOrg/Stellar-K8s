//! Advanced Configuration Management Module
//!
//! Provides validation, versioning, rollback, and drift detection for
//! StellarNode and Operator configurations.

pub mod validation;
pub mod versioning;
pub mod rollback;
pub mod drift;
pub mod impact;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Result of a configuration change operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigChangeResult {
    pub success: bool,
    pub version: u64,
    pub message: String,
    pub impact_score: f32,
    pub validation_errors: Vec<String>,
}

/// Metadata for configuration history tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMetadata {
    pub author: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub reason: String,
    pub previous_hash: String,
}
