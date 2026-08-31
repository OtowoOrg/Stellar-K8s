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
//! Webhook Module
//!
//! This module provides a Wasm-based admission webhook for custom
//! StellarNode validation logic.
//!
//! # Features
//!
//! - **Wasm Plugin Runtime**: Execute custom validation logic in a sandboxed environment
//! - **Admission Webhook**: Kubernetes ValidatingAdmissionWebhook integration
//! - **Plugin Management**: Load, unload, and manage validation plugins
//! - **Security**: Resource limits, fuel metering, and integrity verification
//!
//! # Architecture
//!
//! The webhook server:
//! 1. Receives admission review requests from Kubernetes API server
//! 2. Loads and executes WASM plugins in a sandboxed runtime
//! 3. Collects validation results from all plugins
//! 4. Returns admission response (allow/deny) to Kubernetes
//!
//! # Plugin Development
//!
//! Plugins are WASM modules that implement custom validation logic:
//! - Validate quorum set configurations
//! - Enforce organizational policies
//! - Check resource constraints
//! - Verify network connectivity
//!
//! # Example: Creating a Plugin
//!
//! ```rust,ignore
//! use stellar_k8s::webhook::{WasmRuntime, WebhookServer, PluginConfig, PluginMetadata};
//!
//! // Create the runtime
//! let runtime = WasmRuntime::new()?;
//!
//! // Create the webhook server
//! let server = WebhookServer::new(runtime);
//!
//! // Add a plugin
//! let plugin = PluginConfig {
//!     metadata: PluginMetadata {
//!         name: "my-validator".to_string(),
//!         version: "1.0.0".to_string(),
//!         ..Default::default()
//!     },
//!     wasm_binary: Some(wasm_bytes),
//!     operations: vec![Operation::Create, Operation::Update],
//!     enabled: true,
//!     ..Default::default()
//! };
//! server.add_plugin(plugin).await?;
//!
//! // Start the server
//! server.start("0.0.0.0:8443".parse()?).await?;
//! ```

pub mod config_guardrails;
pub mod mutation;
pub mod org_validator;
pub mod runtime;
pub mod server;
pub mod types;

pub use config_guardrails::{
    blocking_violations, check_config_guardrails, GuardrailViolation, Severity,
};
pub use mutation::apply_mutations;
pub use runtime::{WasmRuntime, WasmRuntimeBuilder};
pub use server::{LoadPluginRequest, PluginInfo, PluginListResponse, TlsConfig, WebhookServer};
pub use types::{
    AggregatedValidationResult, ConfigMapRef, DbTriggerInput, DbTriggerOutput, Operation,
    PluginConfig, PluginExecutionResult, PluginLimits, PluginMetadata, SecretRef, UserInfo,
    ValidationError, ValidationErrorType, ValidationInput, ValidationOutput,
};
