//! Example reconcile hook operator using the Stellar-K8s SDK.

use async_trait::async_trait;
use stellar_k8s::plugin_sdk::{HookResult, ReconcileContext, ReconcileHook};

pub struct MetricsExporterHook;

#[async_trait]
impl ReconcileHook for MetricsExporterHook {
    fn name(&self) -> &str {
        "metrics-exporter"
    }

    async fn pre_reconcile(&self, ctx: &ReconcileContext) -> HookResult {
        tracing::debug!(node = %ctx.node_name, "exporting pre-reconcile metrics");
        HookResult::Continue
    }
}
