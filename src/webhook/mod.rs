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

pub mod mutation;
pub mod runtime;
pub mod server;
pub mod types;

pub use mutation::apply_mutations;
pub use runtime::{WasmRuntime, WasmRuntimeBuilder};
pub use server::{LoadPluginRequest, PluginInfo, PluginListResponse, TlsConfig, WebhookServer};
pub use types::{
    AggregatedValidationResult, ConfigMapRef, DbTriggerInput, DbTriggerOutput, Operation,
    PluginConfig, PluginExecutionResult, PluginLimits, PluginMetadata, SecretRef, UserInfo,
    ValidationError, ValidationErrorType, ValidationInput, ValidationOutput,
};
