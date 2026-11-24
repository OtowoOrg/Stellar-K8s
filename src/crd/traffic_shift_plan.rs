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
//! TrafficShiftPlan Custom Resource Definition (epic #1504).
//!
//! `TrafficShiftPlan` is the controller-owned, declarative plan that drives a
//! multi-region failover (and the matching failback) of a weighted DNS routing
//! record. The operator never edits DNS imperatively: it renders a weighted
//! record from `spec` plus `status` progress and records the rendered record,
//! every gate evaluation and every weight increment in `status.steps`, so the
//! whole shift is auditable with `kubectl get trafficshiftplan -o yaml`.
//!
//! The two invariants the plan encodes:
//!
//! * **Health-gated**: primary and secondary are evaluated independently by the
//!   same [`HealthGateSpec`]. A shift only advances while the gate is open, and
//!   the gate closes again as soon as either region stops meeting the evidence
//!   bar (aborting mid-shift).
//! * **Failback-gated identically**: failback uses exactly the same
//!   [`HealthGateSpec`] thresholds as failover, with the roles of primary and
//!   secondary swapped. There is no cheaper "the primary looks better now"
//!   shortcut (see [`crate::controller::traffic_shift::evaluate_gate`]).
//!
//! ```yaml
//! apiVersion: stellar.org/v1alpha1
//! kind: TrafficShiftPlan
//! metadata:
//!   name: horizon-global
//!   namespace: stellar
//! spec:
//!   primary:
//!     name: eu-west
//!     region: eu-west-1
//!     endpoint: horizon.eu-west.example.com
//!   secondary:
//!     name: us-east
//!     region: us-east-1
//!     endpoint: horizon.us-east.example.com
//!   trigger: Automatic
//!   routing:
//!     hostname: horizon.stellar.example.com
//!     ttlSeconds: 60
//!   healthGate:
//!     failureThreshold: 3
//!     recoveryThreshold: 5
//!     minSuccessRatePercent: 99.0
//!     minSamples: 5
//!     evidenceWindowSeconds: 300
//!   shift:
//!     stepPercent: 25
//!     soakSeconds: 120
//!     drainSeconds: 60
//!   failback:
//!     automatic: true
//!     minStableSeconds: 1800
//!   targets:
//!     rtoSeconds: 900
//!     rpoSeconds: 30
//! ```

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::types::Condition;

#[derive(CustomResource, Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "TrafficShiftPlan",
    status = "TrafficShiftPlanStatus",
    shortname = "tsp",
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Direction","type":"string","jsonPath":".status.direction"}"#,
    printcolumn = r#"{"name":"Primary %","type":"integer","jsonPath":".status.currentWeights.primaryPercent"}"#,
    printcolumn = r#"{"name":"Steps","type":"integer","jsonPath":".status.stepsCompleted"}"#,
    printcolumn = r#"{"name":"RTO (s)","type":"integer","jsonPath":".status.rto.measuredSeconds"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct TrafficShiftPlanSpec {
    /// Region currently authoritative. Fails over away from it.
    pub primary: RegionTarget,
    /// Region traffic is shifted onto during failover, and back to during
    /// failback. Must differ from `primary`.
    pub secondary: RegionTarget,
    /// When the gate may open the plan on its own.
    #[serde(default)]
    pub trigger: FailoverTrigger,
    /// Weighted routing record rendered by the controller.
    #[serde(default)]
    pub routing: RoutingRecordSpec,
    /// Health evidence required of *both* regions, in both directions.
    #[serde(default)]
    pub health_gate: HealthGateSpec,
    /// Incremental shift shape: step size, soak and drain durations.
    #[serde(default)]
    pub shift: ShiftPolicy,
    /// Failback behaviour (still gated by `healthGate`).
    #[serde(default)]
    pub failback: FailbackPolicy,
    /// Declared RTO/RPO the shift is measured against.
    #[serde(default)]
    pub targets: FailoverTargets,
    /// Optional drill identifier; recorded in the plan status so quarterly
    /// drill results can be attributed to the plan that executed them.
    #[serde(default)]
    pub drill_id: Option<String>,
}

/// One routable region.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegionTarget {
    /// Short region identifier used in metrics and record targets.
    pub name: String,
    /// Cloud region, recorded for operator readability.
    #[serde(default)]
    pub region: String,
    /// Hostname published in the weighted record.
    pub endpoint: String,
}

impl Default for RegionTarget {
    fn default() -> Self {
        Self {
            name: String::new(),
            region: String::new(),
            endpoint: String::new(),
        }
    }
}

/// Whether the operator starts a shift without human action.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FailoverTrigger {
    /// Operator must annotate the plan (`stellar.org/trigger-failover`) or set
    /// `spec.trigger` to `Automatic`.
    #[default]
    Manual,
    /// Gate evidence alone starts the shift.
    Automatic,
}

/// Weighted routing record driven by the plan.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutingRecordSpec {
    /// Public hostname the weighted record is published under.
    #[serde(default = "default_hostname")]
    pub hostname: String,
    /// DNS TTL in seconds. Also the lower bound on the propagation wait
    /// between two weight increments.
    #[serde(default = "default_ttl")]
    pub ttl_seconds: u32,
    /// Primary region share before any failover, in percent (0-100).
    #[serde(default = "default_initial_primary_weight")]
    pub initial_primary_weight: u32,
}

impl Default for RoutingRecordSpec {
    fn default() -> Self {
        Self {
            hostname: default_hostname(),
            ttl_seconds: default_ttl(),
            initial_primary_weight: default_initial_primary_weight(),
        }
    }
}

fn default_hostname() -> String {
    "stellar-horizon.stellar.example.com".into()
}
fn default_ttl() -> u32 {
    60
}
fn default_initial_primary_weight() -> u32 {
    100
}

/// Health evidence bar. Applied independently to primary and secondary, and
/// identically for failover and failback.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HealthGateSpec {
    /// Consecutive failed probes that prove a region is down.
    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: u32,
    /// Consecutive successful probes that prove a region is up.
    #[serde(default = "default_recovery_threshold")]
    pub recovery_threshold: u32,
    /// Minimum success ratio over the evidence window, in percent.
    #[serde(default = "default_min_success_rate")]
    pub min_success_rate_percent: f64,
    /// Minimum probes inside the evidence window before the gate may open.
    #[serde(default = "default_min_samples")]
    pub min_samples: u32,
    /// Evidence older than this is treated as stale and fails the gate.
    #[serde(default = "default_evidence_window")]
    pub evidence_window_seconds: i64,
    /// Per-probe timeout in seconds.
    #[serde(default = "default_probe_timeout")]
    pub timeout_seconds: u32,
}

impl Default for HealthGateSpec {
    fn default() -> Self {
        Self {
            failure_threshold: default_failure_threshold(),
            recovery_threshold: default_recovery_threshold(),
            min_success_rate_percent: default_min_success_rate(),
            min_samples: default_min_samples(),
            evidence_window_seconds: default_evidence_window(),
            timeout_seconds: default_probe_timeout(),
        }
    }
}

fn default_failure_threshold() -> u32 {
    3
}
fn default_recovery_threshold() -> u32 {
    5
}
fn default_min_success_rate() -> f64 {
    99.0
}
fn default_min_samples() -> u32 {
    5
}
fn default_evidence_window() -> i64 {
    300
}
fn default_probe_timeout() -> u32 {
    5
}

/// Incremental shift shape.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShiftPolicy {
    /// Primary share moved per increment, in percent (1-100).
    #[serde(default = "default_step_percent")]
    pub step_percent: u32,
    /// Observation window after an increment before the next one.
    #[serde(default = "default_soak")]
    pub soak_seconds: i64,
    /// Connection drain wait applied to the region losing weight.
    #[serde(default = "default_drain")]
    pub drain_seconds: i64,
    /// Abort the shift if the soak error rate exceeds this percent.
    #[serde(default = "default_max_soak_error_rate")]
    pub max_soak_error_rate_percent: f64,
}

impl Default for ShiftPolicy {
    fn default() -> Self {
        Self {
            step_percent: default_step_percent(),
            soak_seconds: default_soak(),
            drain_seconds: default_drain(),
            max_soak_error_rate_percent: default_max_soak_error_rate(),
        }
    }
}

fn default_step_percent() -> u32 {
    25
}
fn default_soak() -> i64 {
    120
}
fn default_drain() -> i64 {
    60
}
fn default_max_soak_error_rate() -> f64 {
    1.0
}

/// Failback behaviour.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailbackPolicy {
    /// Shift back automatically once the gate opens for the primary.
    #[serde(default)]
    pub automatic: bool,
    /// Minimum time the secondary must have served traffic after a failover
    /// before failback may start.
    #[serde(default = "default_min_stable")]
    pub min_stable_seconds: i64,
    /// Run a full soak after the last failback increment.
    #[serde(default = "default_true")]
    pub verify_final_step: bool,
}

fn default_min_stable() -> i64 {
    1800
}
fn default_true() -> bool {
    true
}

/// Declared objectives the shift is measured against.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailoverTargets {
    /// Declared RTO: time from gate open to traffic fully on the secondary.
    #[serde(default = "default_rto")]
    pub rto_seconds: i64,
    /// Declared RPO: tolerated replication lag on the secondary.
    #[serde(default = "default_rpo")]
    pub rpo_seconds: u64,
    /// When set, the plan records `Drill` instead of `Live` in its report.
    #[serde(default)]
    pub drill: bool,
}

impl Default for FailoverTargets {
    fn default() -> Self {
        Self {
            rto_seconds: default_rto(),
            rpo_seconds: default_rpo(),
            drill: false,
        }
    }
}

fn default_rto() -> i64 {
    900
}
fn default_rpo() -> u64 {
    30
}

/// Plan phase.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TrafficShiftPhase {
    /// Nothing in flight; the plan is serving at its configured weights.
    #[default]
    Idle,
    /// Health evidence collected, gate decision taken, no weight changed yet.
    Gated,
    /// Connection drain on the region losing weight.
    Draining,
    /// A new weighted record is being published.
    Shifting,
    /// Soaking a published increment, watching the error budget.
    Soaking,
    /// Target weights reached for the current direction.
    Completed,
    /// Gate closed mid-shift; weights held at the last safe increment.
    Aborted,
    /// Terminal failure (invalid spec, no reachable region).
    Failed,
}

impl TrafficShiftPhase {
    /// Weight is (or should be) fully on the declared primary.
    pub fn is_failover_complete(&self) -> bool {
        matches!(self, TrafficShiftPhase::Completed)
    }

    /// A weight change is pending or in flight.
    pub fn is_in_progress(&self) -> bool {
        matches!(
            self,
            TrafficShiftPhase::Draining | TrafficShiftPhase::Shifting | TrafficShiftPhase::Soaking
        )
    }

    /// Shift is over for now, whatever the outcome.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TrafficShiftPhase::Completed | TrafficShiftPhase::Aborted | TrafficShiftPhase::Failed
        )
    }
}

/// Direction of the current shift.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ShiftDirection {
    /// Traffic moving away from `spec.primary`.
    #[default]
    Failover,
    /// Traffic moving back to `spec.primary`.
    Failback,
}

impl ShiftDirection {
    /// Swap the roles of `primary` and `secondary`.
    pub fn swapped(self) -> Self {
        match self {
            ShiftDirection::Failover => ShiftDirection::Failback,
            ShiftDirection::Failback => ShiftDirection::Failover,
        }
    }

    /// Lower-case label used in metrics and reports.
    pub fn as_str(self) -> &'static str {
        match self {
            ShiftDirection::Failover => "failover",
            ShiftDirection::Failback => "failback",
        }
    }
}

/// Health evidence collected for one region.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegionHealthEvidence {
    pub region: String,
    /// Probes that returned success inside the evidence window.
    pub successes: u32,
    /// Probes that failed inside the evidence window.
    pub failures: u32,
    /// Consecutive successes ending at `last_probe_at`.
    pub consecutive_successes: u32,
    /// Consecutive failures ending at `last_probe_at`.
    pub consecutive_failures: u32,
    /// Success ratio over the evidence window, in percent.
    pub success_rate_percent: f64,
    /// Error ratio observed by the region's own serving layer, in percent.
    #[serde(default)]
    pub error_rate_percent: f64,
    /// Replication lag towards the other region, in seconds (RPO evidence).
    #[serde(default)]
    pub replication_lag_seconds: Option<u64>,
    /// Newest probe timestamp.
    pub last_probe_at: DateTime<Utc>,
    /// Oldest probe timestamp inside the evidence window.
    pub first_probe_at: DateTime<Utc>,
    /// Last probe error, if any.
    #[serde(default)]
    pub last_error: Option<String>,
}

impl RegionHealthEvidence {
    /// Number of probes inside the evidence window.
    pub fn samples(&self) -> u32 {
        self.successes + self.failures
    }

    /// True when the newest evidence is older than `window_seconds`.
    pub fn is_stale(&self, now: DateTime<Utc>, window_seconds: i64) -> bool {
        (now - self.last_probe_at).num_seconds() > window_seconds
    }

    /// Region proves itself healthy against `gate` as of `now`.
    pub fn is_healthy(&self, gate: &HealthGateSpec, now: DateTime<Utc>) -> bool {
        self.consecutive_successes >= gate.recovery_threshold
            && self.samples() >= gate.min_samples
            && self.success_rate_percent >= gate.min_success_rate_percent
            && !self.is_stale(now, gate.evidence_window_seconds)
    }

    /// Region proves itself unhealthy against `gate`.
    pub fn is_unhealthy(&self, gate: &HealthGateSpec) -> bool {
        self.consecutive_failures >= gate.failure_threshold
    }
}

/// A single gate evaluation, recorded so the evidence behind every increment is
/// auditable.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GateDecision {
    /// True when both regions met the evidence bar for `direction`.
    pub open: bool,
    pub direction: ShiftDirection,
    /// Why the gate is open or closed, in operator language.
    pub reason: String,
    /// Evidence thresholds applied, echoed for auditability.
    pub applied: AppliedGate,
    /// Evidence for the region losing weight.
    pub source: RegionHealthEvidence,
    /// Evidence for the region gaining weight.
    pub target: RegionHealthEvidence,
    pub evaluated_at: DateTime<Utc>,
}

/// The exact thresholds a gate decision was taken with.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppliedGate {
    pub failure_threshold: u32,
    pub recovery_threshold: u32,
    pub min_success_rate_percent: f64,
    pub min_samples: u32,
    pub evidence_window_seconds: i64,
}

/// Outcome of one weight increment.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StepOutcome {
    /// Planned but not started.
    #[default]
    Pending,
    /// Connections on the losing region are draining.
    Draining,
    /// Weighted record published.
    Applied,
    /// Soak complete and clean.
    Succeeded,
    /// Gate closed during the step; weights held.
    Aborted,
    /// Terminal failure while publishing this step.
    Failed,
}

impl StepOutcome {
    pub fn is_done(&self) -> bool {
        matches!(self, StepOutcome::Succeeded)
    }
}

/// One recorded increment of the shift.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShiftStepStatus {
    /// Zero-based index within the plan.
    pub index: u32,
    pub direction: ShiftDirection,
    /// Primary share before the increment, in percent.
    pub from_primary_weight: u32,
    /// Primary share after the increment, in percent.
    pub to_primary_weight: u32,
    /// Weighted record published by this step.
    #[serde(default)]
    pub record: Option<serde_json::Value>,
    /// DNS TTL published with the record.
    pub record_ttl_seconds: u32,
    /// Gate decision taken before the increment was published.
    #[serde(default)]
    pub gate: Option<GateDecision>,
    /// Connection drain start on the region losing weight.
    #[serde(default)]
    pub drain_started_at: Option<DateTime<Utc>>,
    /// Increment publish time.
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    /// Soak end time (step publish time plus TTL and soak).
    #[serde(default)]
    pub soak_ends_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub outcome: StepOutcome,
    /// Error rate observed during the soak, in percent.
    #[serde(default)]
    pub soak_error_rate_percent: f64,
    /// Operator-readable note, e.g. why a step aborted.
    #[serde(default)]
    pub message: String,
}

/// Current share of each region.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrafficWeights {
    pub primary_percent: u32,
    pub secondary_percent: u32,
}

impl TrafficWeights {
    /// Weights for a given primary share, clamped to 0-100.
    pub fn from_primary_percent(primary_percent: u32) -> Self {
        let primary_percent = primary_percent.min(100);
        Self {
            primary_percent,
            secondary_percent: 100 - primary_percent,
        }
    }

    /// True when all traffic is on the secondary.
    pub fn is_fully_failed_over(&self) -> bool {
        self.primary_percent == 0
    }

    /// True when all traffic is back on the primary.
    pub fn is_fully_failed_back(&self) -> bool {
        self.secondary_percent == 0
    }
}

/// RTO measurement for the current direction.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RtoMeasurement {
    pub direction: ShiftDirection,
    /// When the gate opened for this direction.
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    /// When the target weights were reached.
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    /// Measured seconds, present once the shift completed.
    #[serde(default)]
    pub measured_seconds: Option<i64>,
    pub target_seconds: i64,
    /// True when the measured RTO is within `target_seconds`.
    #[serde(default)]
    pub met: Option<bool>,
    /// Seconds by which the RTO was missed (0 when met).
    #[serde(default)]
    pub over_by_seconds: i64,
}

/// RPO evidence recorded from the secondary's replication lag.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RpoEvidence {
    pub measured_lag_seconds: u64,
    pub target_seconds: u64,
    pub met: bool,
    pub measured_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrafficShiftPlanStatus {
    #[serde(default)]
    pub observed_generation: Option<i64>,
    #[serde(default)]
    pub phase: TrafficShiftPhase,
    /// Direction of the shift in flight (or the last one).
    #[serde(default)]
    pub direction: ShiftDirection,
    /// Weights currently published.
    #[serde(default)]
    pub current_weights: TrafficWeights,
    /// Number of recorded steps that reached `Succeeded`.
    #[serde(default)]
    pub steps_completed: u32,
    /// Every step of the current and previous directions, oldest first.
    #[serde(default)]
    pub steps: Vec<ShiftStepStatus>,
    /// Last gate evaluation.
    #[serde(default)]
    pub last_gate: Option<GateDecision>,
    /// Last health evidence per region, so operators see the raw inputs.
    #[serde(default)]
    pub health: std::collections::BTreeMap<String, RegionHealthEvidence>,
    /// RTO measurement for the current direction.
    #[serde(default)]
    pub rto: RtoMeasurement,
    /// RPO evidence from the last successful gate evaluation.
    #[serde(default)]
    pub rpo: Option<RpoEvidence>,
    /// Weighted record most recently published by the controller.
    #[serde(default)]
    pub applied_record: Option<serde_json::Value>,
    /// When the shift in flight was started.
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    /// When the shift reached its target weights.
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    /// When the secondary took over the whole traffic share. Failback's
    /// cooldown (`spec.failback.minStableSeconds`) is measured from here, so it
    /// survives a status reset of the in-flight RTO window.
    #[serde(default)]
    pub serving_since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_transition_time: Option<DateTime<Utc>>,
    /// When the controller last evaluated the plan.
    #[serde(default)]
    pub last_evaluated_at: Option<DateTime<Utc>>,
    /// Operator-readable one-screen summary.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}
