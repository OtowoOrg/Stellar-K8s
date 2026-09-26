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
//! Graceful Degradation Modes for Partial Control-Plane Outage
//!
//! Keeps the data plane serving when etcd, cluster DNS, the admission webhook
//! layer or the scheduler becomes unavailable.
//!
//! # Design
//!
//! - [`probes`] checks each component independently, on its own timeout.
//!   Probes whose signal depends on another component (the scheduler lease
//!   depends on etcd) are marked inconclusive rather than blamed while that
//!   dependency is down.
//! - [`tracker::DegradationTracker`] applies failure/recovery hysteresis per
//!   component and derives a declared [`DegradationLevel`]
//!   (`Normal < Reduced < Degraded < Frozen`). It records every transition
//!   and produces a post-incident mode report when the level returns to
//!   `Normal`.
//! - [`DegradationGate`] is a lock-free handle the reconciler consults before
//!   writing or taking disruptive actions, so a degraded control plane never
//!   causes the operator to restart or reschedule serving pods.
//! - [`monitor::ControlPlaneHealthMonitor`] drives the loop and publishes the
//!   state on the `ControlPlaneHealth` CR. Its own state is in memory, so
//!   detection keeps working during an etcd outage and the status catches up
//!   automatically on recovery.
//!
//! ## Acceptance Criteria (from #1494)
//! - Workload traffic uninterrupted during 15min etcd outage (`Frozen`
//!   withholds every operator write; the data plane never depends on etcd)
//! - Degradation transitions logged and alerted (tracing +
//!   `stellar_control_plane_*` metrics + `monitoring/control-plane-degradation-alerts.yaml`)
//! - Webhook outage never blocks pod startup in permissive mode
//!   (`failurePolicy: Ignore`, enforced by the monitor)
//! - Full recovery without manual intervention (hysteresis-based recovery)

pub mod monitor;
pub mod probes;
pub mod tracker;

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

pub use crate::crd::control_plane_health::{
    ComponentState, ControlPlaneComponent, DegradationLevel, PermittedActions,
};

/// Operator action classes governed by the degradation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorAction {
    /// Any write to the Kubernetes API.
    Write,
    /// An action that interrupts serving pods (restart, delete, evict).
    Disruptive,
}

/// Shared, lock-free view of the current degradation level.
///
/// Defaults to `Normal` so the operator is unrestricted when no monitor runs.
#[derive(Debug, Clone, Default)]
pub struct DegradationGate {
    inner: Arc<GateInner>,
}

#[derive(Debug, Default)]
struct GateInner {
    level: AtomicU8,
    suppressed: AtomicU64,
}

impl DegradationGate {
    /// Process-wide gate shared by the monitor and the reconcilers.
    pub fn global() -> &'static DegradationGate {
        static GLOBAL: OnceLock<DegradationGate> = OnceLock::new();
        GLOBAL.get_or_init(DegradationGate::default)
    }

    pub fn level(&self) -> DegradationLevel {
        DegradationLevel::from_index(self.inner.level.load(Ordering::Acquire))
    }

    pub fn set_level(&self, level: DegradationLevel) {
        self.inner.level.store(level.as_index(), Ordering::Release);
    }

    /// `Ok` if `action` is allowed; otherwise counts it as suppressed and
    /// returns the level that withheld it.
    pub fn check(&self, action: OperatorAction) -> Result<(), DegradationLevel> {
        let level = self.level();
        let permitted = level.permitted();
        let allowed = match action {
            OperatorAction::Write => permitted.writes,
            OperatorAction::Disruptive => permitted.disruptive_actions,
        };
        if allowed {
            return Ok(());
        }
        self.inner.suppressed.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        crate::controller::metrics::inc_control_plane_suppressed_action(match action {
            OperatorAction::Write => "write",
            OperatorAction::Disruptive => "disruptive",
        });
        Err(level)
    }

    /// Total actions withheld since process start.
    pub fn suppressed_total(&self) -> u64 {
        self.inner.suppressed.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_defaults_to_normal_and_permits_everything() {
        let gate = DegradationGate::default();
        assert_eq!(gate.level(), DegradationLevel::Normal);
        assert!(gate.check(OperatorAction::Write).is_ok());
        assert!(gate.check(OperatorAction::Disruptive).is_ok());
        assert_eq!(gate.suppressed_total(), 0);
    }

    #[test]
    fn gate_enforces_level_policy_and_counts_suppressions() {
        let gate = DegradationGate::default();
        gate.set_level(DegradationLevel::Reduced);
        assert!(gate.check(OperatorAction::Write).is_ok());
        assert_eq!(
            gate.check(OperatorAction::Disruptive),
            Err(DegradationLevel::Reduced)
        );
        gate.set_level(DegradationLevel::Frozen);
        assert_eq!(
            gate.check(OperatorAction::Write),
            Err(DegradationLevel::Frozen)
        );
        assert_eq!(gate.suppressed_total(), 2);
        // Clones share state.
        assert_eq!(gate.clone().level(), DegradationLevel::Frozen);
    }
}
