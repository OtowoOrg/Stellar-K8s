//! Stellar-K8s Operator SDK
//!
//! Framework for building custom operators and extensions with code generation,
//! testing utilities, and plugin registration.

pub mod codegen;
pub mod testing;

pub use crate::plugin_sdk::{
    HookResult, InjectedSidecar, PluginRegistry, ReconcileContext, ReconcileHook, SidecarInjector,
};

pub use codegen::{generate_controller_stub, ControllerStub};
pub use testing::{MockReconcileContext, TestHarness};
