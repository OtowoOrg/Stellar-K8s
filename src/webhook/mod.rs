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
//! # Example
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
