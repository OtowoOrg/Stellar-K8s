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
//! Advanced Configuration Management Module
//!
//! Provides validation, versioning, rollback, and drift detection for
//! StellarNode and Operator configurations.

pub mod drift;
pub mod impact;
pub mod rollback;
pub mod validation;
pub mod versioning;

use serde::{Deserialize, Serialize};

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
