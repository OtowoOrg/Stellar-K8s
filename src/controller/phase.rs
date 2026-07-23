//! Explicit reconcile-phase state machine.
//!
//! `apply_stellar_node` and `cleanup_stellar_node` (see
//! [`super::reconciler`]) were, until now, a single long function whose only
//! record of "where we are" was a sequence of numbered comments (`// 1.
//! Core infrastructure`, `// 5. Create/update the Deployment`, ...). This
//! module gives that implicit structure an explicit, typed, testable name:
//! a [`Phase`] enum with a defined transition table, tracked per-reconcile
//! by a [`PhaseTracker`].
//!
//! This is deliberately observational, not load-bearing: recording a phase
//! transition never fails or changes control flow. An illegal transition
//! (a programming error — phases skipped or taken out of order) is logged
//! as a warning and the tracker still moves to the requested phase, so a
//! bug in phase bookkeeping can never turn into a reconcile failure. The
//! actual reconciliation behavior driven by `apply_stellar_node` is
//! unchanged; phases are metadata layered on top of it.
//!
//! Each call to `apply_stellar_node` / `cleanup_stellar_node` creates its
//! own [`PhaseTracker`] — phase state is per-invocation, not persisted
//! across reconciles.

use std::fmt;

use tracing::warn;

/// A stage of a single reconcile pass.
///
/// The happy-path (create/update) sequence is:
/// `Initializing -> Provisioning -> Configuring -> Observing -> Reconciling
/// -> Finalizing -> Completed`.
///
/// The deletion sequence is: `Initializing -> Deleting -> Completed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Spec/security validation, network-safety checks, plugin `pre_reconcile` hooks.
    Initializing,
    /// Core infrastructure: PVC, ConfigMap, managed database.
    Provisioning,
    /// Suspension handling, mTLS certs, Deployment/StatefulSet, canary state machine.
    Configuring,
    /// Health checks, sync-state scaling, quorum analysis, archive pruning.
    Observing,
    /// Disaster recovery, cross-cloud failover, auto-remediation.
    Reconciling,
    /// Final status/condition patch and metrics emission.
    Finalizing,
    /// The finalizer cleanup path for a resource being deleted.
    Deleting,
    /// Terminal: this reconcile pass is done.
    Completed,
}

impl Phase {
    /// The phases that may legally follow this one.
    fn allowed_next(self) -> &'static [Phase] {
        match self {
            Phase::Initializing => &[Phase::Provisioning, Phase::Deleting],
            Phase::Provisioning => &[Phase::Configuring],
            Phase::Configuring => &[Phase::Observing],
            Phase::Observing => &[Phase::Reconciling],
            Phase::Reconciling => &[Phase::Finalizing],
            Phase::Finalizing => &[Phase::Completed],
            Phase::Deleting => &[Phase::Completed],
            Phase::Completed => &[],
        }
    }

    fn is_legal_transition(self, next: Phase) -> bool {
        self.allowed_next().contains(&next)
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Phase::Initializing => "Initializing",
            Phase::Provisioning => "Provisioning",
            Phase::Configuring => "Configuring",
            Phase::Observing => "Observing",
            Phase::Reconciling => "Reconciling",
            Phase::Finalizing => "Finalizing",
            Phase::Deleting => "Deleting",
            Phase::Completed => "Completed",
        };
        write!(f, "{s}")
    }
}

/// One recorded transition, kept for post-hoc debugging (e.g. attaching the
/// phase history to an error log when a reconcile fails partway through).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseTransition {
    pub from: Phase,
    pub to: Phase,
}

/// Tracks the current phase and transition history for one reconcile pass.
#[derive(Debug, Clone)]
pub struct PhaseTracker {
    current: Phase,
    history: Vec<PhaseTransition>,
    object: String,
}

impl PhaseTracker {
    /// Start a new tracker at [`Phase::Initializing`] for the given object
    /// (typically `"namespace/name"`, used only in log output).
    pub fn new(object: impl Into<String>) -> Self {
        Self {
            current: Phase::Initializing,
            history: Vec::new(),
            object: object.into(),
        }
    }

    pub fn current(&self) -> Phase {
        self.current
    }

    pub fn history(&self) -> &[PhaseTransition] {
        &self.history
    }

    /// Move to `next`. Always succeeds from the caller's perspective — an
    /// out-of-order transition is logged, not returned as an error, so that
    /// phase bookkeeping can never affect reconciliation control flow.
    pub fn transition(&mut self, next: Phase) {
        if !self.current.is_legal_transition(next) {
            warn!(
                object = %self.object,
                from = %self.current,
                to = %next,
                "reconcile phase transition out of order (bug in phase bookkeeping, reconciliation continues unaffected)"
            );
        }
        tracing::debug!(object = %self.object, from = %self.current, to = %next, "reconcile phase transition");
        self.history.push(PhaseTransition {
            from: self.current,
            to: next,
        });
        self.current = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_initializing() {
        let tracker = PhaseTracker::new("stellar/test-node");
        assert_eq!(tracker.current(), Phase::Initializing);
        assert!(tracker.history().is_empty());
    }

    #[test]
    fn happy_path_sequence_is_legal() {
        let mut tracker = PhaseTracker::new("stellar/test-node");
        for phase in [
            Phase::Provisioning,
            Phase::Configuring,
            Phase::Observing,
            Phase::Reconciling,
            Phase::Finalizing,
            Phase::Completed,
        ] {
            tracker.transition(phase);
        }
        assert_eq!(tracker.current(), Phase::Completed);
        assert_eq!(tracker.history().len(), 6);
    }

    #[test]
    fn deletion_sequence_is_legal() {
        let mut tracker = PhaseTracker::new("stellar/test-node");
        tracker.transition(Phase::Deleting);
        tracker.transition(Phase::Completed);
        assert_eq!(tracker.current(), Phase::Completed);
    }

    #[test]
    fn every_happy_path_step_is_individually_legal() {
        let sequence = [
            Phase::Initializing,
            Phase::Provisioning,
            Phase::Configuring,
            Phase::Observing,
            Phase::Reconciling,
            Phase::Finalizing,
            Phase::Completed,
        ];
        for pair in sequence.windows(2) {
            assert!(
                pair[0].is_legal_transition(pair[1]),
                "{:?} -> {:?} should be legal",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn completed_is_terminal() {
        assert!(Phase::Completed.allowed_next().is_empty());
    }

    #[test]
    fn out_of_order_transition_does_not_panic_and_still_moves() {
        let mut tracker = PhaseTracker::new("stellar/test-node");
        // Skipping straight to Observing is illegal but must not panic or
        // block the tracker from recording it.
        tracker.transition(Phase::Observing);
        assert_eq!(tracker.current(), Phase::Observing);
        assert_eq!(tracker.history().len(), 1);
        assert_eq!(tracker.history()[0].from, Phase::Initializing);
        assert_eq!(tracker.history()[0].to, Phase::Observing);
    }

    #[test]
    fn skipping_backward_is_illegal() {
        assert!(!Phase::Observing.is_legal_transition(Phase::Provisioning));
    }

    #[test]
    fn display_matches_variant_name() {
        assert_eq!(Phase::Provisioning.to_string(), "Provisioning");
        assert_eq!(Phase::Deleting.to_string(), "Deleting");
    }

    #[test]
    fn initializing_can_branch_to_deleting_or_provisioning() {
        assert!(Phase::Initializing.is_legal_transition(Phase::Provisioning));
        assert!(Phase::Initializing.is_legal_transition(Phase::Deleting));
        assert!(!Phase::Initializing.is_legal_transition(Phase::Completed));
    }
}
