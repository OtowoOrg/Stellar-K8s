//! Error retry policy.

use super::prelude::*;
use super::state::ControllerState;

/// Error policy determines how to handle reconciliation errors
pub(crate) fn error_policy(
    node: Arc<StellarNode>,
    error: &Error,
    ctx: Arc<ControllerState>,
) -> Action {
    let node_name = node.name_any();
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let reconcile_id = ctx.next_reconcile_id();

    let node_name_for_span = node_name.clone();
    let namespace_for_span = namespace.clone();
    let resource_version = node
        .metadata
        .resource_version
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let _error_span = info_span!(
        "reconcile_error",
        node_name = %node_name_for_span,
        namespace = %namespace_for_span,
        reconcile_id = %reconcile_id,
        resource_version = %resource_version
    );
    let _enter = _error_span.enter();

    error!("Reconciliation error for {}: {:?}", node_name, error);

    // Get retry count from annotations (default to 0)
    let retry_count = node
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("stellar.org/error-retry-count"))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    // Apply operator retry budget based on error retriability.
    let retry_duration = if error.is_retriable() {
        Duration::from_secs(ctx.retry_budget_retriable_secs)
    } else {
        Duration::from_secs(ctx.retry_budget_nonretriable_secs)
    };

    debug!(
        "Requeuing {} after {:?} (retry_count: {}, retriable: {})",
        node.name_any(),
        retry_duration,
        retry_count,
        error.is_retriable()
    );

    Action::requeue(retry_duration)
}
