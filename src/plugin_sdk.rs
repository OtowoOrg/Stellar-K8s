//! Stellar-K8s Operator Plugin SDK
//!
//! Provides a stable, documented API for third-party developers to extend the
//! operator with custom **Reconciliation Hooks** and **Sidecar Injectors**.
//!
//! # Quick Start
//!
//! Implement [`ReconcileHook`] to run logic before/after every reconcile cycle:
//!
//! ```rust,no_run
//! use stellar_k8s::plugin_sdk::{ReconcileHook, ReconcileContext, HookResult};
//!
//! pub struct MyHook;
//!
//! #[async_trait::async_trait]
//! impl ReconcileHook for MyHook {
//!     fn name(&self) -> &str { "my-hook" }
//!
//!     async fn pre_reconcile(&self, ctx: &ReconcileContext) -> HookResult {
//!         tracing::info!(node = %ctx.node_name, "pre-reconcile");
//!         HookResult::Continue
//!     }
//! }
//! ```
//!
//! Implement [`SidecarInjector`] to inject containers into managed pods:
//!
//! ```rust,no_run
//! use stellar_k8s::plugin_sdk::{SidecarInjector, ReconcileContext, InjectedSidecar};
//!
//! pub struct MySidecar;
//!
//! #[async_trait::async_trait]
//! impl SidecarInjector for MySidecar {
//!     fn name(&self) -> &str { "my-sidecar" }
//!
//!     async fn sidecars(&self, ctx: &ReconcileContext) -> Vec<InjectedSidecar> {
//!         vec![InjectedSidecar {
//!             name: "my-container".into(),
//!             image: "my-image:latest".into(),
//!             ..Default::default()
//!         }]
//!     }
//! }
//! ```
//!
//! Register plugins with [`PluginRegistry`] and pass it to [`ControllerState`].

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::crd::{NodeType, StellarNode};

// ── Context ───────────────────────────────────────────────────────────────────

/// Immutable context passed to every plugin invocation.
#[derive(Clone, Debug)]
pub struct ReconcileContext {
    /// Name of the StellarNode being reconciled.
    pub node_name: String,
    /// Namespace of the StellarNode.
    pub namespace: String,
    /// Node type (Validator, Horizon, SorobanRpc).
    pub node_type: NodeType,
    /// Full StellarNode spec snapshot (read-only).
    pub node: Arc<StellarNode>,
}

impl ReconcileContext {
    pub fn from_node(node: &Arc<StellarNode>) -> Self {
        Self {
            node_name: node.metadata.name.clone().unwrap_or_default(),
            namespace: node.metadata.namespace.clone().unwrap_or_default(),
            node_type: node.spec.node_type.clone(),
            node: node.clone(),
        }
    }
}

// ── Hook result ───────────────────────────────────────────────────────────────

/// Return value from a [`ReconcileHook`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookResult {
    /// Continue with normal reconciliation.
    Continue,
    /// Abort reconciliation with an error message (logged as a warning; node
    /// will be requeued according to the normal retry budget).
    Abort(String),
}

// ── ReconcileHook trait ───────────────────────────────────────────────────────

/// A hook that runs before and/or after each reconcile cycle.
///
/// Both methods have default no-op implementations so implementors only need
/// to override the phases they care about.
#[async_trait]
pub trait ReconcileHook: Send + Sync + 'static {
    /// Unique, human-readable name used in logs and metrics.
    fn name(&self) -> &str;

    /// Called **before** resources are applied to the cluster.
    ///
    /// Return [`HookResult::Abort`] to skip reconciliation for this cycle.
    async fn pre_reconcile(&self, _ctx: &ReconcileContext) -> HookResult {
        HookResult::Continue
    }

    /// Called **after** resources have been successfully applied.
    async fn post_reconcile(&self, _ctx: &ReconcileContext) {}
}

// ── SidecarInjector trait ─────────────────────────────────────────────────────

/// A minimal description of a sidecar container to inject.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InjectedSidecar {
    /// Container name (must be unique within the pod).
    pub name: String,
    /// Container image (e.g. `"fluent/fluent-bit:3.0"`).
    pub image: String,
    /// Optional command override.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    /// Optional arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Environment variables as `(name, value)` pairs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<(String, String)>,
}

/// Injects additional sidecar containers into every pod managed by the operator.
///
/// The operator merges the returned sidecars into the pod template **after**
/// all built-in containers have been constructed, so injected containers can
/// reference volumes created by the operator (e.g. `data`, `config`).
#[async_trait]
pub trait SidecarInjector: Send + Sync + 'static {
    /// Unique, human-readable name used in logs.
    fn name(&self) -> &str;

    /// Return the sidecars to inject for the given node.
    ///
    /// Return an empty `Vec` to skip injection for this node.
    async fn sidecars(&self, ctx: &ReconcileContext) -> Vec<InjectedSidecar>;
}

// ── PluginRegistry ────────────────────────────────────────────────────────────

/// Central registry that holds all registered plugins.
///
/// Pass an `Arc<PluginRegistry>` to [`ControllerState`] to activate plugins.
///
/// # Example
///
/// ```rust,no_run
/// use std::sync::Arc;
/// use stellar_k8s::plugin_sdk::PluginRegistry;
/// use stellar_k8s::plugin_sdk::examples::{CustomLoggerHook, MetricsMonitorHook};
///
/// let registry = Arc::new(
///     PluginRegistry::new()
///         .with_hook(CustomLoggerHook)
///         .with_hook(MetricsMonitorHook)
/// );
/// ```
#[derive(Default)]
pub struct PluginRegistry {
    hooks: Vec<Arc<dyn ReconcileHook>>,
    injectors: Vec<Arc<dyn SidecarInjector>>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a [`ReconcileHook`].
    pub fn with_hook(mut self, hook: impl ReconcileHook) -> Self {
        self.hooks.push(Arc::new(hook));
        self
    }

    /// Register a [`SidecarInjector`].
    pub fn with_injector(mut self, injector: impl SidecarInjector) -> Self {
        self.injectors.push(Arc::new(injector));
        self
    }

    /// Run all `pre_reconcile` hooks in registration order.
    ///
    /// Returns the first [`HookResult::Abort`] encountered, or
    /// [`HookResult::Continue`] if all hooks pass.
    pub async fn run_pre_reconcile(&self, ctx: &ReconcileContext) -> HookResult {
        for hook in &self.hooks {
            match hook.pre_reconcile(ctx).await {
                HookResult::Continue => {}
                abort @ HookResult::Abort(_) => {
                    tracing::warn!(
                        hook = hook.name(),
                        node = %ctx.node_name,
                        "pre_reconcile hook aborted reconciliation"
                    );
                    return abort;
                }
            }
        }
        HookResult::Continue
    }

    /// Run all `post_reconcile` hooks in registration order.
    pub async fn run_post_reconcile(&self, ctx: &ReconcileContext) {
        for hook in &self.hooks {
            hook.post_reconcile(ctx).await;
        }
    }

    /// Collect sidecars from all injectors for the given context.
    pub async fn collect_sidecars(&self, ctx: &ReconcileContext) -> Vec<InjectedSidecar> {
        let mut result = Vec::new();
        for injector in &self.injectors {
            result.extend(injector.sidecars(ctx).await);
        }
        result
    }

    /// Number of registered hooks.
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    /// Number of registered injectors.
    pub fn injector_count(&self) -> usize {
        self.injectors.len()
    }
}

// ── Built-in example plugins ──────────────────────────────────────────────────

/// Ready-to-use example plugins demonstrating the SDK.
pub mod examples {
    use super::*;
    use tracing::{info, warn};

    // ── CustomLoggerHook ──────────────────────────────────────────────────────

    /// Example hook: emits structured log lines at every reconcile phase.
    ///
    /// Useful as a starting point for audit-trail or debugging plugins.
    pub struct CustomLoggerHook;

    #[async_trait]
    impl ReconcileHook for CustomLoggerHook {
        fn name(&self) -> &str {
            "custom-logger"
        }

        async fn pre_reconcile(&self, ctx: &ReconcileContext) -> HookResult {
            info!(
                plugin = "custom-logger",
                node = %ctx.node_name,
                namespace = %ctx.namespace,
                node_type = ?ctx.node_type,
                "pre_reconcile: starting reconcile cycle"
            );
            HookResult::Continue
        }

        async fn post_reconcile(&self, ctx: &ReconcileContext) {
            info!(
                plugin = "custom-logger",
                node = %ctx.node_name,
                namespace = %ctx.namespace,
                "post_reconcile: reconcile cycle completed"
            );
        }
    }

    // ── MetricsMonitorHook ────────────────────────────────────────────────────

    /// Example hook: tracks per-node reconcile counts in an in-memory counter.
    ///
    /// In production, replace the `AtomicU64` with a Prometheus counter.
    pub struct MetricsMonitorHook {
        reconcile_count: std::sync::atomic::AtomicU64,
    }

    impl MetricsMonitorHook {
        pub fn new() -> Self {
            Self {
                reconcile_count: std::sync::atomic::AtomicU64::new(0),
            }
        }

        /// Return the total number of reconcile cycles observed.
        pub fn count(&self) -> u64 {
            self.reconcile_count
                .load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl Default for MetricsMonitorHook {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait]
    impl ReconcileHook for MetricsMonitorHook {
        fn name(&self) -> &str {
            "metrics-monitor"
        }

        async fn pre_reconcile(&self, ctx: &ReconcileContext) -> HookResult {
            let n = self
                .reconcile_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            info!(
                plugin = "metrics-monitor",
                node = %ctx.node_name,
                reconcile_count = n,
                "reconcile cycle #{n}"
            );
            HookResult::Continue
        }
    }

    // ── LogShipperInjector ────────────────────────────────────────────────────

    /// Example sidecar injector: appends a Fluent Bit log-shipper container to
    /// every Soroban RPC pod.
    pub struct LogShipperInjector {
        /// Fluent Bit image to inject (default: `fluent/fluent-bit:3.0`).
        pub image: String,
    }

    impl Default for LogShipperInjector {
        fn default() -> Self {
            Self {
                image: "fluent/fluent-bit:3.0".to_string(),
            }
        }
    }

    #[async_trait]
    impl SidecarInjector for LogShipperInjector {
        fn name(&self) -> &str {
            "log-shipper"
        }

        async fn sidecars(&self, ctx: &ReconcileContext) -> Vec<InjectedSidecar> {
            // Only inject into Soroban RPC pods.
            if ctx.node_type != NodeType::SorobanRpc {
                return vec![];
            }
            vec![InjectedSidecar {
                name: "log-shipper".to_string(),
                image: self.image.clone(),
                args: vec!["--config=/fluent-bit/etc/fluent-bit.conf".to_string()],
                env: vec![
                    ("NODE_NAME".to_string(), ctx.node_name.clone()),
                    ("NAMESPACE".to_string(), ctx.namespace.clone()),
                ],
                ..Default::default()
            }]
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{StellarNodeSpec, StellarNodeStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn make_node(node_type: NodeType) -> Arc<StellarNode> {
        Arc::new(StellarNode {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                namespace: Some("stellar-system".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type,
                ..Default::default()
            },
            status: None,
        })
    }

    fn make_ctx(node_type: NodeType) -> ReconcileContext {
        ReconcileContext::from_node(&make_node(node_type))
    }

    // ── HookResult ────────────────────────────────────────────────────────────

    #[test]
    fn hook_result_variants() {
        assert_eq!(HookResult::Continue, HookResult::Continue);
        assert_eq!(
            HookResult::Abort("oops".into()),
            HookResult::Abort("oops".into())
        );
    }

    // ── PluginRegistry ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn empty_registry_continues() {
        let reg = PluginRegistry::new();
        let ctx = make_ctx(NodeType::Validator);
        assert_eq!(reg.run_pre_reconcile(&ctx).await, HookResult::Continue);
        assert_eq!(reg.hook_count(), 0);
        assert_eq!(reg.injector_count(), 0);
    }

    #[tokio::test]
    async fn custom_logger_hook_continues() {
        use examples::CustomLoggerHook;
        let reg = PluginRegistry::new().with_hook(CustomLoggerHook);
        let ctx = make_ctx(NodeType::Horizon);
        assert_eq!(reg.run_pre_reconcile(&ctx).await, HookResult::Continue);
        reg.run_post_reconcile(&ctx).await; // must not panic
    }

    #[tokio::test]
    async fn metrics_monitor_increments_count() {
        use examples::MetricsMonitorHook;
        let hook = MetricsMonitorHook::new();
        let ctx = make_ctx(NodeType::Validator);
        hook.pre_reconcile(&ctx).await;
        hook.pre_reconcile(&ctx).await;
        assert_eq!(hook.count(), 2);
    }

    #[tokio::test]
    async fn abort_hook_short_circuits() {
        struct AbortHook;
        #[async_trait]
        impl ReconcileHook for AbortHook {
            fn name(&self) -> &str {
                "abort"
            }
            async fn pre_reconcile(&self, _ctx: &ReconcileContext) -> HookResult {
                HookResult::Abort("blocked".into())
            }
        }

        struct NeverHook;
        #[async_trait]
        impl ReconcileHook for NeverHook {
            fn name(&self) -> &str {
                "never"
            }
            async fn pre_reconcile(&self, _ctx: &ReconcileContext) -> HookResult {
                panic!("should not be called after abort");
            }
        }

        let reg = PluginRegistry::new()
            .with_hook(AbortHook)
            .with_hook(NeverHook);
        let ctx = make_ctx(NodeType::Validator);
        let result = reg.run_pre_reconcile(&ctx).await;
        assert_eq!(result, HookResult::Abort("blocked".into()));
    }

    #[tokio::test]
    async fn log_shipper_injected_only_for_soroban() {
        use examples::LogShipperInjector;
        let reg = PluginRegistry::new().with_injector(LogShipperInjector::default());

        let soroban_ctx = make_ctx(NodeType::SorobanRpc);
        let sidecars = reg.collect_sidecars(&soroban_ctx).await;
        assert_eq!(sidecars.len(), 1);
        assert_eq!(sidecars[0].name, "log-shipper");

        let validator_ctx = make_ctx(NodeType::Validator);
        let sidecars = reg.collect_sidecars(&validator_ctx).await;
        assert!(sidecars.is_empty());
    }

    #[tokio::test]
    async fn multiple_injectors_merged() {
        use examples::LogShipperInjector;

        struct ExtraInjector;
        #[async_trait]
        impl SidecarInjector for ExtraInjector {
            fn name(&self) -> &str {
                "extra"
            }
            async fn sidecars(&self, _ctx: &ReconcileContext) -> Vec<InjectedSidecar> {
                vec![InjectedSidecar {
                    name: "extra-container".into(),
                    image: "extra:latest".into(),
                    ..Default::default()
                }]
            }
        }

        let reg = PluginRegistry::new()
            .with_injector(LogShipperInjector::default())
            .with_injector(ExtraInjector);

        let ctx = make_ctx(NodeType::SorobanRpc);
        let sidecars = reg.collect_sidecars(&ctx).await;
        assert_eq!(sidecars.len(), 2);
    }
}
