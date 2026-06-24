//! Main reconciler for StellarNode resources
//!
//! Implements the controller pattern using kube-rs runtime.
//! The reconciler watches StellarNode resources and ensures that the desired state
//! (as specified in the StellarNode spec) matches the actual state in the Kubernetes cluster.
//!
//! # Key Components
//!
//! - [`ControllerState`] - Shared state for the controller including the Kubernetes client
//! - [`run_controller`] - Main entry point that starts the controller loop
//!
//! # Reconciliation Workflow
//!
//! 1. Watch for changes to StellarNode resources
//! 2. Validate the StellarNode spec
//! 3. Create/update Kubernetes resources (Deployments, Services, PVCs, etc.)
//! 4. Check node health and sync status
//! 5. Handle node remediation if needed
//! 6. Update StellarNode status with current state
//! 7. Schedule requeue for periodic health checks

use std::sync::Arc;

use tracing::{info, warn};

use crate::crd::StellarNode;

mod apply;
mod cleanup;
mod controller;
mod error_policy;
mod events;
#[cfg(feature = "reconciler-fuzz")]
mod fuzz;
mod prelude;
mod reconcile;
mod state;
mod support;

trait ToStellarNodeArc {
    fn to_arc(&self) -> Arc<StellarNode>;
}
impl ToStellarNodeArc for Arc<StellarNode> {
    fn to_arc(&self) -> Arc<StellarNode> {
        self.clone()
    }
}
impl ToStellarNodeArc for &Arc<StellarNode> {
    fn to_arc(&self) -> Arc<StellarNode> {
        (*self).clone()
    }
}
impl ToStellarNodeArc for StellarNode {
    fn to_arc(&self) -> Arc<StellarNode> {
        Arc::new(self.clone())
    }
}
impl ToStellarNodeArc for &StellarNode {
    fn to_arc(&self) -> Arc<StellarNode> {
        Arc::new((*self).clone())
    }
}

trait ToControllerStateArc {
    fn to_arc_controller(&self) -> Arc<ControllerState>;
}
impl ToControllerStateArc for Arc<ControllerState> {
    fn to_arc_controller(&self) -> Arc<ControllerState> {
        self.clone()
    }
}
impl ToControllerStateArc for &Arc<ControllerState> {
    fn to_arc_controller(&self) -> Arc<ControllerState> {
        (*self).clone()
    }
}
macro_rules! emit_event {
    ($client:expr, $reporter:expr, $node:expr, $type:expr, $reason:expr, $action:expr, $note:expr $(,)?) => {
        $crate::controller::reconciler::events::emit_event_owned(
            $client.clone(),
            $reporter.clone(),
            $node.to_arc(),
            $type,
            $reason.to_string(),
            $action.to_string(),
            $note.to_string(),
        )
    };
}

macro_rules! publish_stellar_event {
    ($client:expr, $reporter:expr, $node:expr, $type:expr, $reason:expr, $action:expr, $note:expr $(,)?) => {
        $crate::controller::reconciler::events::publish_stellar_event_owned(
            $client.clone(),
            $reporter.clone(),
            $node.to_arc(),
            $type,
            $reason.to_string(),
            $action.to_string(),
            $note.to_string(),
        )
    };
}

macro_rules! apply_or_emit {
    ($ctx:expr, $node:expr, $action:expr, $info:expr, clones: [$($clone:ident),*], $closure:expr $(,)?) => {
        {
            $( let $clone = $clone.clone(); )*
            let _ctx_internal = $ctx.to_arc_controller();
            let _node_internal = $node.to_arc();
            let _client_clone = _ctx_internal.client.clone();
            let _ctx_clone = _ctx_internal.clone();
            let _node_clone = _node_internal.clone();

            let _fut = $closure(_client_clone, _ctx_clone, _node_clone);
            $crate::controller::reconciler::events::apply_or_emit_owned(_ctx_internal, _node_internal, $action, $info.to_string(), _fut)
        }
    };
    ($ctx:expr, $node:expr, $action:expr, $info:expr, $closure:expr $(,)?) => {
        apply_or_emit!($ctx, $node, $action, $info, clones: [], $closure)
    };
}
/// Summary report for a batch of reconciliation results.
///
/// Tracks the number of successful and failed reconciliations
/// within a reporting window and provides a formatted summary log.
#[derive(Debug, Default)]
pub struct BatchSummaryReport {
    /// Number of successful reconciliations in this batch
    pub successes: u64,
    /// Number of failed reconciliations in this batch
    pub failures: u64,
    /// Names of successfully reconciled objects in this batch
    pub reconciled_objects: Vec<String>,
    /// Failure details: (object name, error description)
    pub failure_details: Vec<(String, String)>,
    /// Total events seen (successes + failures)
    pub total: u64,
    /// Emit a summary every N events (batch window size)
    batch_size: u64,
}

impl BatchSummaryReport {
    /// Create a new report that emits a summary every `batch_size` events.
    pub fn new(batch_size: u64) -> Self {
        Self {
            batch_size: batch_size.max(1),
            ..Default::default()
        }
    }

    /// Record a successful reconciliation.
    pub fn record_success(&mut self, object_name: String) {
        self.successes += 1;
        self.total += 1;
        self.reconciled_objects.push(object_name);
        if self.total.is_multiple_of(self.batch_size) {
            self.emit_summary();
        }
    }

    /// Record a failed reconciliation.
    pub fn record_failure(&mut self, object_name: String, error: String) {
        self.failures += 1;
        self.total += 1;
        self.failure_details.push((object_name, error));
        if self.total.is_multiple_of(self.batch_size) {
            self.emit_summary();
        }
    }

    /// Emit the end-of-batch summary log.
    pub fn emit_summary(&self) {
        info!(
            total = self.total,
            successes = self.successes,
            failures = self.failures,
            "=== Reconciliation batch summary ==="
        );
        if !self.reconciled_objects.is_empty() {
            info!(
                objects = ?self.reconciled_objects,
                "Reconciled objects in this batch"
            );
        }
        if !self.failure_details.is_empty() {
            for (name, err) in &self.failure_details {
                warn!(object = %name, error = %err, "Reconciliation failure in batch");
            }
        }
    }

    /// Emit a final summary regardless of batch window position.
    /// Call this when the controller shuts down.
    pub fn emit_final_summary(&self) {
        if self.total == 0 {
            info!("=== End-of-run summary: no reconciliation events processed ===");
            return;
        }
        let success_rate = (self.successes as f64 / self.total as f64) * 100.0;
        info!(
            total = self.total,
            successes = self.successes,
            failures = self.failures,
            success_rate_pct = format!("{:.1}", success_rate),
            "=== End-of-run reconciliation summary ==="
        );
        if !self.failure_details.is_empty() {
            warn!(
                failure_count = self.failures,
                "Failures encountered during this run:"
            );
            for (name, err) in &self.failure_details {
                warn!(object = %name, error = %err, "  Failed reconciliation");
            }
        }
    }
}

pub use events::ActionType;
pub use controller::run_controller;
pub use state::ControllerState;
pub use BatchSummaryReport;
#[cfg(feature = "reconciler-fuzz")]
pub use fuzz::reconcile_for_fuzz;
pub(crate) use apply::apply_stellar_node;
pub(crate) use cleanup::cleanup_stellar_node;
pub(crate) use error_policy::error_policy;
pub(crate) use reconcile::reconcile;
