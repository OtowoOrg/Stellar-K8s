//! Testing utilities for operator extension development.

use std::sync::Arc;

use crate::crd::{NodeType, StellarNode};
use crate::plugin_sdk::{PluginRegistry, ReconcileContext, ReconcileHook};
use kube::core::ObjectMeta;

/// Build a mock reconcile context for unit tests.
pub struct MockReconcileContext;

impl MockReconcileContext {
    pub fn validator(node_name: &str, namespace: &str) -> ReconcileContext {
        let node = Arc::new(StellarNode {
            metadata: ObjectMeta {
                name: Some(node_name.to_string()),
                namespace: Some(namespace.to_string()),
                ..Default::default()
            },
            spec: Default::default(),
            status: None,
        });
        ReconcileContext::from_node(&node)
    }
}

/// Test harness wrapping a plugin registry for hook tests.
pub struct TestHarness {
    pub registry: PluginRegistry,
}

impl TestHarness {
    pub fn new() -> Self {
        Self {
            registry: PluginRegistry::new(),
        }
    }

    pub fn with_hook(mut self, hook: impl ReconcileHook + 'static) -> Self {
        self.registry = self.registry.with_hook(hook);
        self
    }
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_sdk::HookResult;
    use async_trait::async_trait;

    struct NoopHook;
    #[async_trait]
    impl ReconcileHook for NoopHook {
        fn name(&self) -> &str {
            "noop"
        }
        async fn pre_reconcile(&self, _ctx: &ReconcileContext) -> HookResult {
            HookResult::Continue
        }
    }

    #[test]
    fn mock_context_has_node_type() {
        let ctx = MockReconcileContext::validator("v1", "stellar");
        assert_eq!(ctx.node_name, "v1");
        assert_eq!(ctx.namespace, "stellar");
        assert_eq!(ctx.node_type, NodeType::Validator);
    }

    #[test]
    fn harness_registers_hooks() {
        let harness = TestHarness::new().with_hook(NoopHook);
        assert_eq!(harness.registry.hook_count(), 1);
    }
}
