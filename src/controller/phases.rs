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
//! Modular reconciler phases with explicit state transitions (issue #1047).
//!
//! # Why
//!
//! [`super::reconciler::reconcile`] and [`super::reconciler::apply_stellar_node`]
//! run a long, linear sequence of steps. The sequence is real — validation
//! must precede provisioning, provisioning must precede the workload rollout —
//! but before this module it was implicit: encoded only in statement order and
//! numbered comments. Nothing declared which stage the reconciler was in, no
//! log line named it, and a step accidentally moved across a boundary would
//! produce no signal at all.
//!
//! This module makes that pipeline explicit:
//!
//! * [`ReconcilePhase`] names each stage.
//! * [`ReconcilePhase::can_transition_to`] is the authoritative transition
//!   table — an illegal move is a typed error, not a silent reordering.
//! * [`PhaseMachine`] tracks the current phase, records every transition with
//!   a timestamp and reason, and reports how long each phase took.
//!
//! # Relationship to `StellarNodeStatus::phase`
//!
//! These are two different things and must not be confused:
//!
//! * `StellarNodeStatus::phase` (deprecated) describes the *resource
//!   lifecycle* as an observer sees it: `Running`, `Degraded`, `Failed`.
//! * [`ReconcilePhase`] describes the *reconciler's own pipeline* during a
//!   single pass.
//!
//! [`ReconcilePhase::lifecycle_phase`] maps one onto the other so existing
//! status reporting keeps working unchanged.
//!
//! # Example
//!
//! ```
//! use stellar_k8s::controller::phases::{PhaseMachine, ReconcilePhase};
//!
//! let mut machine = PhaseMachine::new();
//! assert_eq!(machine.current(), ReconcilePhase::Initializing);
//!
//! machine.transition_to(ReconcilePhase::Validating, "spec validation").unwrap();
//! machine.transition_to(ReconcilePhase::Provisioning, "spec is valid").unwrap();
//!
//! // Provisioning cannot jump straight to publishing.
//! assert!(machine.transition_to(ReconcilePhase::Publishing, "skip").is_err());
//!
//! // The machine keeps a full audit trail of the pass.
//! assert_eq!(machine.history().len(), 2);
//! ```

use std::fmt;
use std::time::Instant;

use chrono::{DateTime, Utc};
use tracing::{debug, warn};

use crate::error::{Error, Result};

/// A single stage of one reconciliation pass.
///
/// The order of the variants matches the order the reconciler normally walks
/// them in; [`ReconcilePhase::can_transition_to`] defines which moves are
/// actually legal, including the shortcuts (a deletion skips straight to
/// [`ReconcilePhase::Finalizing`], a validation failure skips straight to
/// [`ReconcilePhase::Failed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum ReconcilePhase {
    /// Entry state: leader check, context and configuration resolution.
    #[default]
    Initializing,
    /// Spec validation, security policy enforcement, network safety checks.
    Validating,
    /// Deletion path: finalizer-driven cleanup of owned resources.
    Finalizing,
    /// Durable prerequisites: PVCs, ConfigMaps, secrets, mTLS material.
    Provisioning,
    /// The workload itself: Deployment/StatefulSet, Service, Ingress, PDB.
    Deploying,
    /// Elasticity: HPA/VPA, replica and disk scaling.
    Scaling,
    /// Health checks, sync state, archive integrity.
    Observing,
    /// Automatic recovery from a detected failure.
    Remediating,
    /// Status subresource and Kubernetes event publication.
    Publishing,
    /// Terminal: the pass completed successfully.
    Succeeded,
    /// Terminal: the pass aborted with an error.
    Failed,
}

impl ReconcilePhase {
    /// Every phase, in pipeline order. Useful for metrics registration and
    /// exhaustiveness tests.
    pub const ALL: [ReconcilePhase; 11] = [
        ReconcilePhase::Initializing,
        ReconcilePhase::Validating,
        ReconcilePhase::Finalizing,
        ReconcilePhase::Provisioning,
        ReconcilePhase::Deploying,
        ReconcilePhase::Scaling,
        ReconcilePhase::Observing,
        ReconcilePhase::Remediating,
        ReconcilePhase::Publishing,
        ReconcilePhase::Succeeded,
        ReconcilePhase::Failed,
    ];

    /// Stable, lowercase identifier. Safe to use as a metric label value.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReconcilePhase::Initializing => "initializing",
            ReconcilePhase::Validating => "validating",
            ReconcilePhase::Finalizing => "finalizing",
            ReconcilePhase::Provisioning => "provisioning",
            ReconcilePhase::Deploying => "deploying",
            ReconcilePhase::Scaling => "scaling",
            ReconcilePhase::Observing => "observing",
            ReconcilePhase::Remediating => "remediating",
            ReconcilePhase::Publishing => "publishing",
            ReconcilePhase::Succeeded => "succeeded",
            ReconcilePhase::Failed => "failed",
        }
    }

    /// True once the pass has ended; no further transitions are legal.
    pub fn is_terminal(&self) -> bool {
        matches!(self, ReconcilePhase::Succeeded | ReconcilePhase::Failed)
    }

    /// The `StellarNodeStatus::phase` string this reconcile phase corresponds
    /// to, so status reporting keeps its existing vocabulary.
    pub fn lifecycle_phase(&self) -> &'static str {
        match self {
            ReconcilePhase::Initializing | ReconcilePhase::Validating => "Pending",
            ReconcilePhase::Finalizing => "Terminating",
            ReconcilePhase::Provisioning | ReconcilePhase::Deploying | ReconcilePhase::Scaling => {
                "Creating"
            }
            ReconcilePhase::Observing => "Syncing",
            ReconcilePhase::Remediating => "Remediating",
            ReconcilePhase::Publishing | ReconcilePhase::Succeeded => "Running",
            ReconcilePhase::Failed => "Failed",
        }
    }

    /// The authoritative transition table.
    ///
    /// Any non-terminal phase may move to [`ReconcilePhase::Failed`] — an
    /// error can surface anywhere. Everything else is enumerated explicitly so
    /// that reordering the pipeline requires editing this table.
    pub fn can_transition_to(&self, next: ReconcilePhase) -> bool {
        use ReconcilePhase::*;

        if self.is_terminal() {
            return false;
        }
        if next == Failed {
            return true;
        }

        match self {
            Initializing => matches!(next, Validating | Finalizing | Succeeded),
            // A spec that fails validation still publishes its status.
            Validating => matches!(next, Provisioning | Publishing),
            Finalizing => matches!(next, Succeeded),
            Provisioning => matches!(next, Deploying),
            // Scaling is skipped when autoscaling is not configured.
            Deploying => matches!(next, Scaling | Observing),
            Scaling => matches!(next, Observing),
            // Observing either finds a problem to remediate or reports.
            Observing => matches!(next, Remediating | Publishing),
            // Remediation re-observes to confirm recovery, or reports.
            Remediating => matches!(next, Observing | Publishing),
            Publishing => matches!(next, Succeeded),
            Succeeded | Failed => false,
        }
    }

    /// The phases reachable from this one, in declaration order.
    pub fn allowed_transitions(&self) -> Vec<ReconcilePhase> {
        ReconcilePhase::ALL
            .into_iter()
            .filter(|next| self.can_transition_to(*next))
            .collect()
    }
}

impl fmt::Display for ReconcilePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One recorded move between phases.
#[derive(Debug, Clone)]
pub struct PhaseTransition {
    /// Phase the reconciler left.
    pub from: ReconcilePhase,
    /// Phase the reconciler entered.
    pub to: ReconcilePhase,
    /// Wall-clock time of the transition, for correlating with events.
    pub at: DateTime<Utc>,
    /// Why the transition happened; surfaced in logs and diagnostics.
    pub reason: String,
    /// How long the reconciler spent in `from` before moving on.
    pub elapsed_ms: u64,
}

impl fmt::Display for PhaseTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} → {} after {}ms ({})",
            self.from, self.to, self.elapsed_ms, self.reason
        )
    }
}

/// Tracks the current reconcile phase and enforces the transition table.
///
/// A machine is created per reconciliation pass. It is deliberately cheap:
/// one enum, one `Instant`, and a `Vec` that grows by at most one entry per
/// phase, so instrumenting the hot reconcile path costs nothing meaningful.
#[derive(Debug)]
pub struct PhaseMachine {
    current: ReconcilePhase,
    history: Vec<PhaseTransition>,
    phase_entered_at: Instant,
    started_at: Instant,
}

impl PhaseMachine {
    /// Start a new pass in [`ReconcilePhase::Initializing`].
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            current: ReconcilePhase::Initializing,
            history: Vec::new(),
            phase_entered_at: now,
            started_at: now,
        }
    }

    /// The phase the reconciler is in right now.
    pub fn current(&self) -> ReconcilePhase {
        self.current
    }

    /// Every transition recorded so far, oldest first.
    pub fn history(&self) -> &[PhaseTransition] {
        &self.history
    }

    /// True once the pass has reached a terminal phase.
    pub fn is_terminal(&self) -> bool {
        self.current.is_terminal()
    }

    /// Milliseconds spent in the current phase so far.
    pub fn elapsed_in_phase_ms(&self) -> u64 {
        self.phase_entered_at.elapsed().as_millis() as u64
    }

    /// Milliseconds since the pass began.
    pub fn total_elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Move to `next`, recording the reason.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PhaseTransitionError`] if the transition table
    /// forbids the move, which always means the pipeline itself is wrong.
    pub fn transition_to(
        &mut self,
        next: ReconcilePhase,
        reason: impl Into<String>,
    ) -> Result<&PhaseTransition> {
        let reason = reason.into();

        if !self.current.can_transition_to(next) {
            let message = format!(
                "cannot move from '{}' to '{}' (allowed: {}); reason was '{}'",
                self.current,
                next,
                self.current
                    .allowed_transitions()
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                reason,
            );
            warn!(
                from = %self.current,
                to = %next,
                "rejected illegal reconcile phase transition"
            );
            return Err(Error::PhaseTransitionError(message));
        }

        let transition = PhaseTransition {
            from: self.current,
            to: next,
            at: Utc::now(),
            reason,
            elapsed_ms: self.elapsed_in_phase_ms(),
        };

        debug!(
            from = %transition.from,
            to = %transition.to,
            elapsed_ms = transition.elapsed_ms,
            reason = %transition.reason,
            "reconcile phase transition"
        );

        self.current = next;
        self.phase_entered_at = Instant::now();
        self.history.push(transition);
        Ok(self.history.last().expect("just pushed a transition"))
    }

    /// Move to [`ReconcilePhase::Failed`], which is legal from any
    /// non-terminal phase. Idempotent once terminal, so error paths can call
    /// it without checking first.
    pub fn fail(&mut self, reason: impl Into<String>) {
        if self.current.is_terminal() {
            return;
        }
        // Failed is reachable from every non-terminal phase, so this cannot
        // return Err; the result is dropped deliberately.
        let _ = self.transition_to(ReconcilePhase::Failed, reason);
    }

    /// Move to [`ReconcilePhase::Succeeded`] if the table allows it, otherwise
    /// leave the machine untouched. Used on the happy path where the caller
    /// does not want a late bookkeeping error to mask a successful reconcile.
    pub fn succeed(&mut self, reason: impl Into<String>) {
        if self.current.can_transition_to(ReconcilePhase::Succeeded) {
            let _ = self.transition_to(ReconcilePhase::Succeeded, reason);
        }
    }

    /// Run `body` inside `phase`, transitioning in first and marking the pass
    /// failed if `body` returns an error.
    ///
    /// This is what makes the phases *modular*: a pipeline step becomes a
    /// self-contained unit that declares the phase it belongs to, and the
    /// bookkeeping (enter, time, fail, record) happens in exactly one place.
    ///
    /// ```
    /// # use stellar_k8s::controller::phases::{PhaseMachine, ReconcilePhase};
    /// # use stellar_k8s::error::Result;
    /// # tokio_test::block_on(async {
    /// let mut machine = PhaseMachine::new();
    /// let value = machine
    ///     .run(ReconcilePhase::Validating, "validate spec", || async { Ok(42) })
    ///     .await
    ///     .unwrap();
    /// assert_eq!(value, 42);
    /// assert_eq!(machine.current(), ReconcilePhase::Validating);
    /// # });
    /// ```
    pub async fn run<F, Fut, T>(
        &mut self,
        phase: ReconcilePhase,
        reason: impl Into<String>,
        body: F,
    ) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        self.transition_to(phase, reason)?;
        match body().await {
            Ok(value) => Ok(value),
            Err(err) => {
                self.fail(format!("{phase} failed: {err}"));
                Err(err)
            }
        }
    }

    /// A single-line trace of the whole pass, for logs and diagnostics.
    ///
    /// ```text
    /// initializing → validating → provisioning → deploying (total 812ms)
    /// ```
    pub fn summary(&self) -> String {
        let mut parts = Vec::with_capacity(self.history.len() + 1);
        if let Some(first) = self.history.first() {
            parts.push(first.from.as_str());
        } else {
            parts.push(self.current.as_str());
        }
        parts.extend(self.history.iter().map(|t| t.to.as_str()));
        format!(
            "{} (total {}ms)",
            parts.join(" → "),
            self.total_elapsed_ms()
        )
    }

    /// Time spent in each phase that has been left, in pipeline order.
    ///
    /// The current phase is excluded because it has not finished yet; use
    /// [`PhaseMachine::elapsed_in_phase_ms`] for that.
    pub fn phase_durations(&self) -> Vec<(ReconcilePhase, u64)> {
        self.history
            .iter()
            .map(|t| (t.from, t.elapsed_ms))
            .collect()
    }
}

impl Default for PhaseMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Phase metadata ───────────────────────────────────────────────────

    #[test]
    fn all_phases_have_distinct_identifiers() {
        let mut seen = std::collections::HashSet::new();
        for phase in ReconcilePhase::ALL {
            assert!(seen.insert(phase.as_str()), "duplicate id for {phase:?}");
        }
        assert_eq!(seen.len(), ReconcilePhase::ALL.len());
    }

    #[test]
    fn identifiers_are_metric_label_safe() {
        for phase in ReconcilePhase::ALL {
            let id = phase.as_str();
            assert!(!id.is_empty());
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{id} is not a safe label value"
            );
        }
    }

    #[test]
    fn only_succeeded_and_failed_are_terminal() {
        let terminal: Vec<_> = ReconcilePhase::ALL
            .into_iter()
            .filter(|p| p.is_terminal())
            .collect();
        assert_eq!(
            terminal,
            vec![ReconcilePhase::Succeeded, ReconcilePhase::Failed]
        );
    }

    #[test]
    fn every_phase_maps_to_a_lifecycle_phase() {
        for phase in ReconcilePhase::ALL {
            assert!(!phase.lifecycle_phase().is_empty(), "{phase:?}");
        }
        assert_eq!(ReconcilePhase::Finalizing.lifecycle_phase(), "Terminating");
        assert_eq!(ReconcilePhase::Failed.lifecycle_phase(), "Failed");
        assert_eq!(ReconcilePhase::Succeeded.lifecycle_phase(), "Running");
    }

    #[test]
    fn display_matches_as_str() {
        for phase in ReconcilePhase::ALL {
            assert_eq!(phase.to_string(), phase.as_str());
        }
    }

    #[test]
    fn default_phase_is_initializing() {
        assert_eq!(ReconcilePhase::default(), ReconcilePhase::Initializing);
    }

    // ── Transition table ─────────────────────────────────────────────────

    #[test]
    fn terminal_phases_allow_no_transitions() {
        for terminal in [ReconcilePhase::Succeeded, ReconcilePhase::Failed] {
            assert!(terminal.allowed_transitions().is_empty(), "{terminal:?}");
            for next in ReconcilePhase::ALL {
                assert!(!terminal.can_transition_to(next));
            }
        }
    }

    #[test]
    fn every_non_terminal_phase_can_fail() {
        for phase in ReconcilePhase::ALL.into_iter().filter(|p| !p.is_terminal()) {
            assert!(
                phase.can_transition_to(ReconcilePhase::Failed),
                "{phase:?} cannot reach Failed"
            );
        }
    }

    #[test]
    fn every_non_terminal_phase_has_a_forward_edge() {
        for phase in ReconcilePhase::ALL.into_iter().filter(|p| !p.is_terminal()) {
            let forward: Vec<_> = phase
                .allowed_transitions()
                .into_iter()
                .filter(|p| *p != ReconcilePhase::Failed)
                .collect();
            assert!(!forward.is_empty(), "{phase:?} is a dead end");
        }
    }

    #[test]
    fn every_phase_is_reachable_from_initializing() {
        let mut reachable = std::collections::HashSet::new();
        let mut queue = vec![ReconcilePhase::Initializing];
        reachable.insert(ReconcilePhase::Initializing);
        while let Some(phase) = queue.pop() {
            for next in phase.allowed_transitions() {
                if reachable.insert(next) {
                    queue.push(next);
                }
            }
        }
        for phase in ReconcilePhase::ALL {
            assert!(reachable.contains(&phase), "{phase:?} is unreachable");
        }
    }

    #[test]
    fn no_phase_transitions_to_itself() {
        for phase in ReconcilePhase::ALL {
            assert!(!phase.can_transition_to(phase), "{phase:?} self-loops");
        }
    }

    #[test]
    fn deletion_path_skips_straight_to_finalizing() {
        assert!(ReconcilePhase::Initializing.can_transition_to(ReconcilePhase::Finalizing));
        assert!(ReconcilePhase::Finalizing.can_transition_to(ReconcilePhase::Succeeded));
        // Finalizing must not fall through into the apply pipeline.
        assert!(!ReconcilePhase::Finalizing.can_transition_to(ReconcilePhase::Provisioning));
    }

    #[test]
    fn scaling_may_be_skipped_but_not_reordered() {
        assert!(ReconcilePhase::Deploying.can_transition_to(ReconcilePhase::Observing));
        assert!(ReconcilePhase::Deploying.can_transition_to(ReconcilePhase::Scaling));
        // Deploying must never precede provisioning.
        assert!(!ReconcilePhase::Deploying.can_transition_to(ReconcilePhase::Provisioning));
        assert!(!ReconcilePhase::Scaling.can_transition_to(ReconcilePhase::Deploying));
    }

    #[test]
    fn remediation_loops_back_to_observing() {
        assert!(ReconcilePhase::Observing.can_transition_to(ReconcilePhase::Remediating));
        assert!(ReconcilePhase::Remediating.can_transition_to(ReconcilePhase::Observing));
        assert!(ReconcilePhase::Remediating.can_transition_to(ReconcilePhase::Publishing));
    }

    #[test]
    fn validation_failure_can_still_publish_status() {
        assert!(ReconcilePhase::Validating.can_transition_to(ReconcilePhase::Publishing));
        assert!(!ReconcilePhase::Validating.can_transition_to(ReconcilePhase::Deploying));
    }

    // ── Machine behaviour ────────────────────────────────────────────────

    #[test]
    fn new_machine_starts_initializing_with_no_history() {
        let machine = PhaseMachine::new();
        assert_eq!(machine.current(), ReconcilePhase::Initializing);
        assert!(machine.history().is_empty());
        assert!(!machine.is_terminal());
    }

    #[test]
    fn legal_transition_advances_and_records() {
        let mut machine = PhaseMachine::new();
        machine
            .transition_to(ReconcilePhase::Validating, "spec check")
            .unwrap();
        assert_eq!(machine.current(), ReconcilePhase::Validating);
        assert_eq!(machine.history().len(), 1);
        assert_eq!(machine.history()[0].from, ReconcilePhase::Initializing);
        assert_eq!(machine.history()[0].to, ReconcilePhase::Validating);
        assert_eq!(machine.history()[0].reason, "spec check");
    }

    #[test]
    fn illegal_transition_is_rejected_without_changing_state() {
        let mut machine = PhaseMachine::new();
        let err = machine
            .transition_to(ReconcilePhase::Deploying, "skip ahead")
            .unwrap_err();
        assert!(matches!(err, Error::PhaseTransitionError(_)));
        assert_eq!(machine.current(), ReconcilePhase::Initializing);
        assert!(machine.history().is_empty());
    }

    #[test]
    fn rejection_message_lists_the_allowed_moves() {
        let mut machine = PhaseMachine::new();
        let err = machine
            .transition_to(ReconcilePhase::Deploying, "skip")
            .unwrap_err()
            .to_string();
        assert!(err.contains("validating"), "{err}");
        assert!(err.contains("finalizing"), "{err}");
        assert!(err.contains("SK8S-023"), "{err}");
    }

    #[test]
    fn full_apply_pipeline_is_legal_end_to_end() {
        let mut machine = PhaseMachine::new();
        for (phase, reason) in [
            (ReconcilePhase::Validating, "validate"),
            (ReconcilePhase::Provisioning, "pvc + configmap"),
            (ReconcilePhase::Deploying, "workload"),
            (ReconcilePhase::Scaling, "hpa"),
            (ReconcilePhase::Observing, "health"),
            (ReconcilePhase::Publishing, "status"),
            (ReconcilePhase::Succeeded, "done"),
        ] {
            machine.transition_to(phase, reason).unwrap();
        }
        assert_eq!(machine.current(), ReconcilePhase::Succeeded);
        assert!(machine.is_terminal());
        assert_eq!(machine.history().len(), 7);
    }

    #[test]
    fn deletion_pipeline_is_legal_end_to_end() {
        let mut machine = PhaseMachine::new();
        machine
            .transition_to(ReconcilePhase::Finalizing, "deletion timestamp set")
            .unwrap();
        machine
            .transition_to(ReconcilePhase::Succeeded, "finalizer removed")
            .unwrap();
        assert!(machine.is_terminal());
    }

    #[test]
    fn fail_is_legal_from_any_non_terminal_phase() {
        for phase in [
            ReconcilePhase::Validating,
            ReconcilePhase::Provisioning,
            ReconcilePhase::Observing,
        ] {
            let mut machine = PhaseMachine::new();
            // Walk to `phase` via a legal path, then fail from there.
            let path: &[ReconcilePhase] = match phase {
                ReconcilePhase::Validating => &[ReconcilePhase::Validating],
                ReconcilePhase::Provisioning => {
                    &[ReconcilePhase::Validating, ReconcilePhase::Provisioning]
                }
                _ => &[
                    ReconcilePhase::Validating,
                    ReconcilePhase::Provisioning,
                    ReconcilePhase::Deploying,
                    ReconcilePhase::Observing,
                ],
            };
            for step in path {
                machine.transition_to(*step, "walk").unwrap();
            }
            machine.fail("boom");
            assert_eq!(machine.current(), ReconcilePhase::Failed);
        }
    }

    #[test]
    fn fail_is_idempotent_once_terminal() {
        let mut machine = PhaseMachine::new();
        machine.fail("first");
        let len = machine.history().len();
        machine.fail("second");
        assert_eq!(machine.history().len(), len);
        assert_eq!(machine.history().last().unwrap().reason, "first");
    }

    #[test]
    fn succeed_is_a_no_op_when_illegal() {
        let mut machine = PhaseMachine::new();
        machine
            .transition_to(ReconcilePhase::Validating, "validate")
            .unwrap();
        // Validating cannot reach Succeeded directly.
        machine.succeed("done");
        assert_eq!(machine.current(), ReconcilePhase::Validating);
        assert_eq!(machine.history().len(), 1);
    }

    #[test]
    fn transitions_after_a_terminal_phase_are_rejected() {
        let mut machine = PhaseMachine::new();
        machine.fail("boom");
        let err = machine.transition_to(ReconcilePhase::Validating, "retry");
        assert!(err.is_err());
    }

    #[test]
    fn summary_lists_the_whole_path() {
        let mut machine = PhaseMachine::new();
        machine
            .transition_to(ReconcilePhase::Validating, "validate")
            .unwrap();
        machine
            .transition_to(ReconcilePhase::Provisioning, "provision")
            .unwrap();
        let summary = machine.summary();
        assert!(summary.starts_with("initializing → validating → provisioning"));
        assert!(summary.contains("total"));
    }

    #[test]
    fn summary_of_a_fresh_machine_names_the_current_phase() {
        assert!(PhaseMachine::new().summary().starts_with("initializing"));
    }

    #[test]
    fn phase_durations_cover_every_left_phase() {
        let mut machine = PhaseMachine::new();
        machine
            .transition_to(ReconcilePhase::Validating, "validate")
            .unwrap();
        machine
            .transition_to(ReconcilePhase::Provisioning, "provision")
            .unwrap();
        let durations = machine.phase_durations();
        assert_eq!(durations.len(), 2);
        assert_eq!(durations[0].0, ReconcilePhase::Initializing);
        assert_eq!(durations[1].0, ReconcilePhase::Validating);
    }

    #[test]
    fn transition_display_is_human_readable() {
        let mut machine = PhaseMachine::new();
        let transition = machine
            .transition_to(ReconcilePhase::Validating, "spec check")
            .unwrap()
            .clone();
        let text = transition.to_string();
        assert!(text.contains("initializing → validating"));
        assert!(text.contains("spec check"));
    }

    // ── run() ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_enters_the_phase_and_returns_the_value() {
        let mut machine = PhaseMachine::new();
        let value = machine
            .run(ReconcilePhase::Validating, "validate", || async { Ok(7) })
            .await
            .unwrap();
        assert_eq!(value, 7);
        assert_eq!(machine.current(), ReconcilePhase::Validating);
    }

    #[tokio::test]
    async fn run_marks_the_pass_failed_when_the_body_errors() {
        let mut machine = PhaseMachine::new();
        let result: Result<()> = machine
            .run(ReconcilePhase::Validating, "validate", || async {
                Err(Error::ValidationError("bad spec".into()))
            })
            .await;
        assert!(result.is_err());
        assert_eq!(machine.current(), ReconcilePhase::Failed);
        assert!(machine
            .history()
            .last()
            .unwrap()
            .reason
            .contains("bad spec"));
    }

    #[tokio::test]
    async fn run_rejects_an_illegal_phase_without_executing_the_body() {
        let mut machine = PhaseMachine::new();
        let mut executed = false;
        let result: Result<()> = machine
            .run(ReconcilePhase::Deploying, "skip", || {
                executed = true;
                async { Ok(()) }
            })
            .await;
        assert!(result.is_err());
        assert!(
            !executed,
            "body must not run when the transition is illegal"
        );
        assert_eq!(machine.current(), ReconcilePhase::Initializing);
    }
}
