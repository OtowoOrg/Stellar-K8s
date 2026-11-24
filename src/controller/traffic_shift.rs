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
//! Multi-region failover orchestration with health-gated traffic shift
//! (epic #1504).
//!
//! The controller drives a [`TrafficShiftPlan`]: when the primary region's
//! health evidence collapses, traffic is moved to the secondary in configurable
//! increments, each increment separated by a soak period, and it is moved back
//! automatically once the primary proves itself recovered.
//!
//! # Building blocks
//!
//! * [`evaluate_gate`] — the health gate. Primary and secondary are evaluated
//!   independently against the *same* [`HealthGateSpec`], and the same
//!   function evaluates failback with the roles swapped. There is no
//!   second, laxer bar for returning traffic to the primary.
//! * [`next_action`] — the incremental state machine. It never moves weight
//!   while the gate is closed, holds the last safe increment if the gate closes
//!   mid-shift, and cannot overshoot the target.
//! * [`drain_deadline`] / [`propagation_deadline`] — connection draining and
//!   DNS TTL propagation. Weight is only moved after the losing region has
//!   drained, and the next increment only after the record has propagated
//!   (`max(ttl, soak)`).
//! * [`weighted_record`] — the declarative weighted routing record the
//!   controller renders. DNS is never edited imperatively; the record is
//!   rendered from the plan and recorded in `status.steps[].record`.
//! * [`measure_rto`] / [`rpo_evidence`] — RTO and RPO measurement against the
//!   declared targets, feeding the DR compliance report via
//!   [`drill_compliance_record`].
//!
//! # Example
//!
//! ```text
//! use std::sync::Arc;
//! use stellar_k8s::controller::traffic_shift::{HttpHealthProbe, reconcile_traffic_shift_plan};
//! use stellar_k8s::crd::TrafficShiftPlan;
//!
//! let status = reconcile_traffic_shift_plan(
//!     &client,
//!     &plan,
//!     Arc::new(HttpHealthProbe::default()),
//! )
//! .await?;
//! tracing::info!(phase = ?status.phase, summary = %status.summary, "shift advanced");
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use kube::api::{Api, Patch, PatchParams};
use kube::{Client, ResourceExt};
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

use crate::crd::traffic_shift_plan::{
    AppliedGate, FailoverTrigger, GateDecision, HealthGateSpec, RegionHealthEvidence, RegionTarget,
    RpoEvidence, RtoMeasurement, ShiftDirection, ShiftStepStatus, StepOutcome, TrafficShiftPhase,
    TrafficShiftPlan, TrafficShiftPlanSpec, TrafficShiftPlanStatus, TrafficWeights,
};
use crate::crd::types::Condition;
use crate::error::{Error, Result};

/// Annotation an operator sets on a `Manual` plan to request a failover.
pub const TRIGGER_FAILOVER_ANNOTATION: &str = "stellar.org/trigger-failover";
/// Annotation an operator sets to suppress a direction (`failover`, `failback`).
pub const SUPPRESS_ANNOTATION: &str = "stellar.org/suppress-shift";
/// Annotation carrying the last rendered weighted record, so DNS changes stay
/// declarative and auditable outside the status subresource.
pub const APPLIED_RECORD_ANNOTATION: &str = "stellar.org/applied-traffic-record";

/// One probe of one region.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeResult {
    pub at: DateTime<Utc>,
    pub success: bool,
    /// Latency of a successful probe, in milliseconds.
    pub latency_ms: Option<u64>,
    /// Replication lag observed by the region's own sync layer, in seconds.
    pub replication_lag_seconds: Option<u64>,
    /// Error rate the region reports for itself, in percent.
    pub error_rate_percent: f64,
    pub error: Option<String>,
}

/// Collects health evidence for a region. Implemented over HTTP in production
/// and by fixtures in tests, which keeps the gate and the state machine fully
/// deterministic.
#[async_trait]
pub trait RegionHealthProbe: Send + Sync {
    /// Probe `target`, returning one result per attempt (at least one).
    async fn probe(&self, target: &RegionTarget, gate: &HealthGateSpec)
        -> Result<Vec<ProbeResult>>;
}

/// HTTP(S) health probe: `failure_threshold` attempts, each answering within
/// `gate.timeout_seconds`.
#[derive(Clone, Debug, Default)]
pub struct HttpHealthProbe {
    client: reqwest::Client,
    /// Health path appended to the region endpoint.
    pub health_path: String,
}

impl HttpHealthProbe {
    /// Probe with an explicit HTTP client (TLS verification already configured).
    pub fn with_client(client: reqwest::Client, health_path: impl Into<String>) -> Self {
        Self {
            client,
            health_path: health_path.into(),
        }
    }
}

#[async_trait]
impl RegionHealthProbe for HttpHealthProbe {
    async fn probe(
        &self,
        target: &RegionTarget,
        gate: &HealthGateSpec,
    ) -> Result<Vec<ProbeResult>> {
        let path = if self.health_path.is_empty() {
            "/health".to_string()
        } else {
            self.health_path.clone()
        };
        let url = format!("https://{}{}", target.endpoint, path);
        let attempts = gate.failure_threshold.max(1) as usize;
        let mut out = Vec::with_capacity(attempts);
        for _ in 0..attempts {
            let started = std::time::Instant::now();
            let at = Utc::now();
            match tokio::time::timeout(
                std::time::Duration::from_secs(gate.timeout_seconds.max(1) as u64),
                self.client.get(&url).send(),
            )
            .await
            {
                Ok(Ok(response)) if response.status().is_success() => out.push(ProbeResult {
                    at,
                    success: true,
                    latency_ms: Some(started.elapsed().as_millis() as u64),
                    replication_lag_seconds: None,
                    error_rate_percent: 0.0,
                    error: None,
                }),
                Ok(Ok(response)) => out.push(ProbeResult {
                    at,
                    success: false,
                    latency_ms: Some(started.elapsed().as_millis() as u64),
                    replication_lag_seconds: None,
                    error_rate_percent: 100.0,
                    error: Some(format!("HTTP {}", response.status())),
                }),
                Ok(Err(e)) => out.push(ProbeResult {
                    at,
                    success: false,
                    latency_ms: None,
                    replication_lag_seconds: None,
                    error_rate_percent: 100.0,
                    error: Some(e.to_string()),
                }),
                Err(_) => out.push(ProbeResult {
                    at,
                    success: false,
                    latency_ms: None,
                    replication_lag_seconds: None,
                    error_rate_percent: 100.0,
                    error: Some("timeout".to_string()),
                }),
            }
        }
        Ok(out)
    }
}

/// Fold probe results into the evidence recorded in status.
///
/// Only probes inside `gate.evidence_window_seconds` of `now` count, and the
/// region is reported stale when nothing landed in that window.
pub fn collect_evidence(
    target: &RegionTarget,
    probes: &[ProbeResult],
    gate: &HealthGateSpec,
    now: DateTime<Utc>,
) -> RegionHealthEvidence {
    let window = Duration::seconds(gate.evidence_window_seconds.max(0));
    let in_window: Vec<&ProbeResult> = probes
        .iter()
        .filter(|p| {
            let age = now - p.at;
            age >= Duration::zero() && age <= window
        })
        .collect();

    let successes = in_window.iter().filter(|p| p.success).count() as u32;
    let failures = in_window.iter().filter(|p| !p.success).count() as u32;
    let total = successes + failures;
    let success_rate_percent = if total == 0 {
        0.0
    } else {
        f64::from(successes) * 100.0 / f64::from(total)
    };

    // Trailing runs, computed oldest to newest over the retained probes.
    let mut ordered: Vec<ProbeResult> = probes.to_vec();
    ordered.sort_by_key(|p| p.at);
    let (consecutive_successes, consecutive_failures) = trailing_runs(&ordered);

    let (first_probe_at, last_probe_at) = match (ordered.first(), ordered.last()) {
        (Some(f), Some(l)) => (f.at, l.at),
        _ => (now, now),
    };
    let last_error = ordered
        .iter()
        .rev()
        .find(|p| !p.success)
        .and_then(|p| p.error.clone());
    // The soak error budget is judged on the *average* error rate the region
    // reported over the window, not its worst second: one slow probe must not
    // abort a shift.
    let (rate_sum, rate_count) = in_window
        .iter()
        .map(|p| (p.error_rate_percent, 1u32))
        .fold((0.0f64, 0u32), |(sum, n), (v, one)| (sum + v, n + one));
    let error_rate_percent = if rate_count == 0 {
        0.0
    } else {
        rate_sum / f64::from(rate_count)
    };
    let replication_lag_seconds = ordered.iter().rev().find_map(|p| p.replication_lag_seconds);

    RegionHealthEvidence {
        region: if target.region.is_empty() {
            target.name.clone()
        } else {
            target.region.clone()
        },
        successes,
        failures,
        consecutive_successes,
        consecutive_failures,
        success_rate_percent,
        error_rate_percent,
        replication_lag_seconds,
        first_probe_at,
        last_probe_at,
        last_error,
    }
}

/// Error rate of the region that is gaining weight in `direction`.
pub fn soak_error_rate(
    direction: ShiftDirection,
    primary: &RegionHealthEvidence,
    secondary: &RegionHealthEvidence,
) -> f64 {
    match direction {
        ShiftDirection::Failover => secondary.error_rate_percent,
        ShiftDirection::Failback => primary.error_rate_percent,
    }
}

/// Consecutive (successes, failures) at the end of an oldest-first probe list.
fn trailing_runs(ordered: &[ProbeResult]) -> (u32, u32) {
    let mut successes = 0u32;
    let mut failures = 0u32;
    for probe in ordered.iter().rev() {
        if probe.success {
            if failures > 0 {
                break;
            }
            successes += 1;
        } else {
            if successes > 0 {
                break;
            }
            failures += 1;
        }
    }
    (successes, failures)
}

/// The health gate.
///
/// Both regions are evaluated independently, and the same criteria decide
/// failover and failback.
///
/// * The region **gaining** weight must prove it is *up* — `recoveryThreshold`
///   consecutive successes, at least `minSamples` inside the evidence window,
///   at least `minSuccessRatePercent`, and evidence no older than
///   `evidenceWindowSeconds`. This is the same bar in both directions, so
///   traffic only ever returns to a primary that has earned it back exactly the
///   way it was given in the first place.
/// * The region **losing** weight must additionally prove it is *down* when the
///   direction is `Failover` — otherwise there is no reason to move at all.
///   Failback does not ask this of the secondary, which is by definition the
///   region that has been serving the traffic and is still healthy.
pub fn evaluate_gate(
    spec: &TrafficShiftPlanSpec,
    direction: ShiftDirection,
    primary: &RegionHealthEvidence,
    secondary: &RegionHealthEvidence,
    now: DateTime<Utc>,
) -> GateDecision {
    let gate = &spec.health_gate;
    let (source, target) = match direction {
        ShiftDirection::Failover => (primary, secondary),
        ShiftDirection::Failback => (secondary, primary),
    };

    let target_ok = target.is_healthy(gate, now);
    let source_ok = match direction {
        ShiftDirection::Failover => source.is_unhealthy(gate),
        // Failback's only additional bar is the target's health, which is the
        // same bar failover applied before the traffic ever left.
        ShiftDirection::Failback => true,
    };
    let target_stale = target.is_stale(now, gate.evidence_window_seconds);
    let source_stale = source.is_stale(now, gate.evidence_window_seconds);

    let reason = if !target_ok && target_stale {
        format!(
            "{} health evidence is older than {}s",
            target.region, gate.evidence_window_seconds
        )
    } else if target.consecutive_successes < gate.recovery_threshold {
        format!(
            "{} has {} consecutive successes (need {})",
            target.region, target.consecutive_successes, gate.recovery_threshold
        )
    } else if target.samples() < gate.min_samples {
        format!(
            "{} has {} samples in the evidence window (need {})",
            target.region,
            target.samples(),
            gate.min_samples
        )
    } else if target.success_rate_percent < gate.min_success_rate_percent {
        format!(
            "{} success rate {:.2}% below {:.2}%",
            target.region, target.success_rate_percent, gate.min_success_rate_percent
        )
    } else if !source_ok {
        format!(
            "{} has only {} consecutive failures (need {})",
            source.region, source.consecutive_failures, gate.failure_threshold
        )
    } else {
        format!(
            "{} down, {} healthy on {}/{} evidence",
            source.region,
            target.region,
            target.consecutive_successes,
            target.samples()
        )
    };

    if source_stale && !target_ok {
        warn!(
            region = %source.region,
            "health evidence is stale; the gate stays closed until the region answers again"
        );
    }

    GateDecision {
        open: source_ok && target_ok,
        direction,
        reason,
        applied: AppliedGate {
            failure_threshold: gate.failure_threshold,
            recovery_threshold: gate.recovery_threshold,
            min_success_rate_percent: gate.min_success_rate_percent,
            min_samples: gate.min_samples,
            evidence_window_seconds: gate.evidence_window_seconds,
        },
        source: source.clone(),
        target: target.clone(),
        evaluated_at: now,
    }
}

/// What the controller should do this cycle.
#[derive(Clone, Debug, PartialEq)]
pub enum ShiftAction {
    /// Nothing to do; the plan is at its target weights.
    Idle { phase: TrafficShiftPhase },
    /// The gate is closed. `direction` is the direction that was waiting on it.
    Hold {
        direction: ShiftDirection,
        reason: String,
    },
    /// The gate is open but the plan is in the wrong direction for the
    /// evidence; waiting (e.g. failback cooldown).
    Wait {
        until: DateTime<Utc>,
        reason: String,
    },
    /// Drain connections on the region losing weight until `until`.
    Drain {
        direction: ShiftDirection,
        step_index: u32,
        from_primary_weight: u32,
        to_primary_weight: u32,
        until: DateTime<Utc>,
    },
    /// Publish a new weighted record.
    Publish {
        direction: ShiftDirection,
        step_index: u32,
        from_primary_weight: u32,
        to_primary_weight: u32,
        record: serde_json::Value,
        ttl_seconds: u32,
    },
    /// Soak the published increment until `until`.
    Soak {
        direction: ShiftDirection,
        step_index: u32,
        until: DateTime<Utc>,
    },
    /// The direction finished; the target weights are published.
    Complete { direction: ShiftDirection },
    /// The gate closed mid-shift; weights are held where they are.
    Abort {
        direction: ShiftDirection,
        step_index: u32,
        reason: String,
    },
    /// The plan can never progress as written.
    Fail { reason: String },
}

impl ShiftAction {
    /// True when the action changes DNS weights.
    pub fn mutates_traffic(&self) -> bool {
        matches!(self, ShiftAction::Publish { .. })
    }

    /// Direction the action belongs to, when it has one.
    pub fn direction(&self) -> Option<ShiftDirection> {
        match self {
            ShiftAction::Hold { direction, .. }
            | ShiftAction::Drain { direction, .. }
            | ShiftAction::Publish { direction, .. }
            | ShiftAction::Soak { direction, .. }
            | ShiftAction::Complete { direction }
            | ShiftAction::Abort { direction, .. } => Some(*direction),
            _ => None,
        }
    }
}

/// Step count to move the primary share from `from` to `target` in
/// `step_percent` increments. Always at least 1, never more than 100.
pub fn step_count(from: u32, target: u32, step_percent: u32) -> u32 {
    let step = step_percent.clamp(1, 100) as i64;
    let distance = (from as i64 - target as i64).abs();
    if distance == 0 {
        0
    } else {
        ((distance + step - 1) / step) as u32
    }
}

/// Primary share after `index + 1` increments moving from `from` to `target`.
///
/// Each increment moves exactly `step_percent`, and the result is clamped to
/// the two endpoints so a step size that does not divide the distance still
/// lands on the target instead of overshooting it.
pub fn weight_after_step(from: u32, target: u32, step_percent: u32, index: u32) -> u32 {
    let step = i64::from(step_percent.clamp(1, 100));
    let increments = i64::from(index) + 1;
    let (from, target) = (i64::from(from), i64::from(target));
    let moved = (target - from).signum() * step * increments;
    (from + moved).clamp(from.min(target), from.max(target)) as u32
}

/// End of the connection drain for a step that starts at `from`.
pub fn drain_deadline(spec: &TrafficShiftPlanSpec, from: DateTime<Utc>) -> DateTime<Utc> {
    from + Duration::seconds(spec.shift.drain_seconds.max(0))
}

/// Earliest time the next increment may be published after `published_at`.
///
/// DNS caches hold the previous weights for the record TTL, so the propagation
/// wait is `max(ttl, soak)`: a soak shorter than the TTL would advance the
/// shift before resolvers could have observed the previous increment.
pub fn propagation_deadline(
    spec: &TrafficShiftPlanSpec,
    published_at: DateTime<Utc>,
) -> DateTime<Utc> {
    let wait = i64::from(spec.routing.ttl_seconds)
        .max(spec.shift.soak_seconds)
        .max(0);
    published_at + Duration::seconds(wait)
}

/// Wall-clock time the plan needs to move from `from` to `target`: one drain
/// and one propagation wait per increment.
pub fn projected_duration_secs(spec: &TrafficShiftPlanSpec, from: u32, target: u32) -> i64 {
    let steps = step_count(from, target, spec.shift.step_percent) as i64;
    let per_step = spec.shift.drain_seconds.max(0)
        + i64::from(spec.routing.ttl_seconds)
            .max(spec.shift.soak_seconds)
            .max(0);
    steps * per_step
}

/// Render the weighted routing record for a primary share, in percent.
///
/// The controller publishes exactly this document (external-dns
/// `DNSEndpoint`-style endpoints with weights) and never mutates DNS in place,
/// so every DNS change is a reviewable diff of the plan's status.
pub fn weighted_record(
    spec: &TrafficShiftPlanSpec,
    primary_percent: u32,
    step_index: u32,
) -> serde_json::Value {
    let weights = TrafficWeights::from_primary_percent(primary_percent);
    json!({
        "apiVersion": "externaldns.k8s.io/v1alpha1",
        "kind": "DNSEndpoint",
        "metadata": {
            "name": spec.routing.hostname,
            "annotations": {
                // Marks the record as controller-owned: hand edits are reverted
                // on the next reconcile.
                "stellar.org/managed-by": "traffic-shift-plan",
                "stellar.org/plan-step": step_index.to_string(),
            },
        },
        "spec": {
            "endpoints": [{
                "dnsName": spec.routing.hostname,
                "recordType": "A",
                "recordTTL": spec.routing.ttl_seconds,
                "targets": [
                    {"target": spec.primary.endpoint, "weight": weights.primary_percent},
                    {"target": spec.secondary.endpoint, "weight": weights.secondary_percent},
                ],
            }],
        }
    })
}

/// Validate the plan, returning operator-readable problems.
pub fn validate_plan(spec: &TrafficShiftPlanSpec) -> Vec<String> {
    let mut problems = Vec::new();
    if spec.primary.name.is_empty() || spec.primary.endpoint.is_empty() {
        problems.push("spec.primary needs a name and an endpoint".into());
    }
    if spec.secondary.name.is_empty() || spec.secondary.endpoint.is_empty() {
        problems.push("spec.secondary needs a name and an endpoint".into());
    }
    if !spec.primary.name.is_empty() && spec.primary.name == spec.secondary.name {
        problems.push("spec.primary and spec.secondary must be different regions".into());
    }
    if spec.routing.hostname.is_empty() {
        problems.push("spec.routing.hostname is required".into());
    }
    if spec.routing.ttl_seconds == 0 {
        problems.push("spec.routing.ttlSeconds must be greater than zero".into());
    }
    if spec.routing.initial_primary_weight > 100 {
        problems.push("spec.routing.initialPrimaryWeight must be 0-100".into());
    }
    if spec.shift.step_percent == 0 || spec.shift.step_percent > 100 {
        problems.push("spec.shift.stepPercent must be 1-100".into());
    }
    if spec.shift.drain_seconds < 0 || spec.shift.soak_seconds < 0 {
        problems.push("spec.shift drain/soak seconds must not be negative".into());
    }
    if spec.health_gate.failure_threshold == 0 {
        problems.push("spec.healthGate.failureThreshold must be greater than zero".into());
    }
    if spec.health_gate.recovery_threshold == 0 {
        problems.push("spec.healthGate.recoveryThreshold must be greater than zero".into());
    }
    if !(0.0..=100.0).contains(&spec.health_gate.min_success_rate_percent) {
        problems.push("spec.healthGate.minSuccessRatePercent must be 0-100".into());
    }
    if spec.health_gate.evidence_window_seconds <= 0 {
        problems.push("spec.healthGate.evidenceWindowSeconds must be greater than zero".into());
    }
    if spec.targets.rto_seconds <= 0 {
        problems.push("spec.targets.rtoSeconds must be greater than zero".into());
    }
    // A plan that cannot finish inside its own RTO can never be compliant.
    let projected = projected_duration_secs(spec, 100, 0);
    if projected > spec.targets.rto_seconds {
        problems.push(format!(
            "shift needs {projected}s ({} steps x {}s) but spec.targets.rtoSeconds is {}",
            step_count(100, 0, spec.shift.step_percent),
            spec.shift.drain_seconds.max(0)
                + i64::from(spec.routing.ttl_seconds)
                    .max(spec.shift.soak_seconds)
                    .max(0),
            spec.targets.rto_seconds
        ));
    }
    problems
}

/// Operator overrides read from the plan's annotations, applied on top of the
/// gate so a human can pause a shift without editing the spec.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanControl {
    /// `stellar.org/trigger-failover: "true"` on a `Manual` plan.
    pub failover_requested: bool,
    /// Directions listed in `stellar.org/suppress-shift`.
    pub suppress_failover: bool,
    pub suppress_failback: bool,
}

impl PlanControl {
    /// Is `direction` suppressed?
    pub fn is_suppressed(&self, direction: ShiftDirection) -> bool {
        match direction {
            ShiftDirection::Failover => self.suppress_failover,
            ShiftDirection::Failback => self.suppress_failback,
        }
    }
}

/// Read the operator overrides from a plan's annotations.
pub fn plan_control(plan: &TrafficShiftPlan) -> PlanControl {
    PlanControl {
        failover_requested: failover_requested(plan),
        suppress_failover: is_suppressed(plan, ShiftDirection::Failover),
        suppress_failback: is_suppressed(plan, ShiftDirection::Failback),
    }
}

/// Decide the next action of the shift state machine.
///
/// `evidence` must be the health of `spec.primary` and `spec.secondary`;
/// `gate` is the current gate decision for the direction the plan is currently
/// in. The function is pure: the caller applies the action to the status.
pub fn next_action(
    spec: &TrafficShiftPlanSpec,
    status: &TrafficShiftPlanStatus,
    primary: &RegionHealthEvidence,
    secondary: &RegionHealthEvidence,
    now: DateTime<Utc>,
) -> ShiftAction {
    next_action_with_control(
        spec,
        status,
        primary,
        secondary,
        now,
        PlanControl::default(),
    )
}

/// [`next_action`] with operator overrides applied.
pub fn next_action_with_control(
    spec: &TrafficShiftPlanSpec,
    status: &TrafficShiftPlanStatus,
    primary: &RegionHealthEvidence,
    secondary: &RegionHealthEvidence,
    now: DateTime<Utc>,
    control: PlanControl,
) -> ShiftAction {
    let problems = validate_plan(spec);
    if !problems.is_empty() {
        return ShiftAction::Fail {
            reason: problems.join("; "),
        };
    }

    // `Failed` needs a spec fix, `Completed` has nothing left to do. An
    // `Aborted` plan is resumable: it holds the last safe increment and only
    // advances again once the soak error budget has recovered.
    if matches!(
        status.phase,
        TrafficShiftPhase::Failed | TrafficShiftPhase::Completed
    ) {
        return ShiftAction::Idle {
            phase: status.phase,
        };
    }

    let direction = status.direction;
    let target_primary_weight = target_weight(spec, direction);
    let current = status.current_weights.primary_percent;
    let step_index = next_step_index(status, direction);
    let to_primary_weight = weight_after_step(
        current,
        target_primary_weight,
        spec.shift.step_percent,
        step_index,
    );
    let from_primary_weight = current;

    // The increment in flight — if any — always finishes before the state
    // machine looks at anything else, including the target.
    if let Some(pending) = status
        .steps
        .iter()
        .rev()
        .find(|s| s.direction == direction && s.index + 1 == step_index)
    {
        match pending.outcome {
            StepOutcome::Applied => {
                let until = pending
                    .soak_ends_at
                    .unwrap_or_else(|| propagation_deadline(spec, now));
                if now < until {
                    return ShiftAction::Soak {
                        direction,
                        step_index: pending.index,
                        until,
                    };
                }
                // The error budget is judged on the evidence of the region
                // that just took the weight, sampled live at the end of soak.
                let rate = soak_error_rate(direction, primary, secondary);
                if rate > spec.shift.max_soak_error_rate_percent {
                    return ShiftAction::Abort {
                        direction,
                        step_index: pending.index,
                        reason: format!(
                            "soak error rate {rate:.2}% exceeds {:.2}%",
                            spec.shift.max_soak_error_rate_percent
                        ),
                    };
                }
            }
            StepOutcome::Draining => {
                // Connections are still draining: no weight moves.
                let until = pending
                    .drain_started_at
                    .map(|s| drain_deadline(spec, s))
                    .unwrap_or(now);
                if now < until {
                    return ShiftAction::Drain {
                        direction,
                        step_index: pending.index,
                        from_primary_weight: pending.from_primary_weight,
                        to_primary_weight: pending.to_primary_weight,
                        until,
                    };
                }
                // The drain is done: publish this increment.
                return ShiftAction::Publish {
                    direction,
                    step_index: pending.index,
                    from_primary_weight: pending.from_primary_weight,
                    to_primary_weight: pending.to_primary_weight,
                    record: weighted_record(spec, pending.to_primary_weight, pending.index),
                    ttl_seconds: spec.routing.ttl_seconds,
                };
            }
            _ => {}
        }
    }

    // The target weights are published and the last soak has run its course.
    if current == target_primary_weight {
        return ShiftAction::Complete { direction };
    }

    // Operator overrides: a suppressed direction never moves weight, and a
    // `Manual` plan needs an explicit request before it fails over.
    if control.is_suppressed(direction) {
        return ShiftAction::Hold {
            direction,
            reason: format!("{direction:?} suppressed by annotation"),
        };
    }
    if spec.trigger == FailoverTrigger::Manual
        && direction == ShiftDirection::Failover
        && !control.failover_requested
    {
        return ShiftAction::Hold {
            direction,
            reason: format!(
                "spec.trigger is Manual; set the {TRIGGER_FAILOVER_ANNOTATION} annotation to start"
            ),
        };
    }

    // Gate: evaluated identically for both directions.
    let gate = evaluate_gate(spec, direction, primary, secondary, now);
    if !gate.open {
        return ShiftAction::Hold {
            direction,
            reason: gate.reason,
        };
    }

    // Failback cooldown: the secondary must have served traffic long enough to
    // have caught up before the primary is trusted again.
    if direction == ShiftDirection::Failback {
        if let Some(ready) = failback_ready_at(spec, status) {
            if now < ready {
                let served = status
                    .serving_since
                    .or(status.completed_at)
                    .map(|s| (now - s).num_seconds().max(0))
                    .unwrap_or(0);
                return ShiftAction::Wait {
                    until: ready,
                    reason: format!(
                        "secondary has served traffic for {served}s of {}s",
                        spec.failback.min_stable_seconds
                    ),
                };
            }
        }
    }

    // A fresh increment always drains the losing region first.
    ShiftAction::Drain {
        direction,
        step_index,
        from_primary_weight,
        to_primary_weight,
        until: drain_deadline(spec, now),
    }
}

/// Primary share the plan converges to for `direction`.
pub fn target_weight(spec: &TrafficShiftPlanSpec, direction: ShiftDirection) -> u32 {
    match direction {
        // All traffic leaves the primary.
        ShiftDirection::Failover => 0,
        // All traffic returns to the configured steady-state share.
        ShiftDirection::Failback => spec.routing.initial_primary_weight,
    }
}

/// Earliest time a failback may start: the end of the failover plus
/// `spec.failback.minStableSeconds` of the secondary carrying the traffic.
pub fn failback_ready_at(
    spec: &TrafficShiftPlanSpec,
    status: &TrafficShiftPlanStatus,
) -> Option<DateTime<Utc>> {
    status
        .serving_since
        .or(status.completed_at)
        .map(|since| since + Duration::seconds(spec.failback.min_stable_seconds.max(0)))
}

/// Next step index to record for `direction`: one past the highest index
/// already recorded, so each increment keeps its own audit entry.
fn next_step_index(status: &TrafficShiftPlanStatus, direction: ShiftDirection) -> u32 {
    status
        .steps
        .iter()
        .filter(|s| s.direction == direction)
        .map(|s| s.index + 1)
        .max()
        .unwrap_or(0)
}

/// Which direction the plan should be moving, given the evidence.
///
/// A direction in progress is never abandoned, and a direction is only left
/// behind once its target weights are published. Only then can a failback be
/// considered, and only on the same gate evidence failover required.
pub fn desired_direction(
    spec: &TrafficShiftPlanSpec,
    status: &TrafficShiftPlanStatus,
    primary: &RegionHealthEvidence,
    secondary: &RegionHealthEvidence,
    now: DateTime<Utc>,
) -> ShiftDirection {
    if status.current_weights.primary_percent != target_weight(spec, status.direction) {
        // Still en route to this direction's target.
        return status.direction;
    }
    let settled = status.phase == TrafficShiftPhase::Completed;
    match status.direction {
        // A settled failover either waits, or starts a failback on the same
        // evidence the gate used to move traffic in the first place.
        ShiftDirection::Failover if settled => {
            let ready = status.current_weights.is_fully_failed_over()
                && spec.failback.automatic
                && evaluate_gate(spec, ShiftDirection::Failback, primary, secondary, now).open;
            if ready {
                ShiftDirection::Failback
            } else {
                ShiftDirection::Failover
            }
        }
        // A settled failback only re-arms the failover when the primary fails
        // the gate again; a merely noisy primary never pulls weight.
        ShiftDirection::Failback if settled => {
            let rearm = status.current_weights.is_fully_failed_back()
                && evaluate_gate(spec, ShiftDirection::Failover, primary, secondary, now).open;
            if rearm {
                ShiftDirection::Failover
            } else {
                ShiftDirection::Failback
            }
        }
        _ => status.direction,
    }
}

/// RTO measurement for the current direction, against the declared target.
pub fn measure_rto(spec: &TrafficShiftPlanSpec, status: &TrafficShiftPlanStatus) -> RtoMeasurement {
    let mut rto = status.rto.clone();
    rto.direction = status.direction;
    rto.target_seconds = spec.targets.rto_seconds;
    if let (Some(start), Some(end)) = (status.started_at, status.completed_at) {
        let measured = (end - start).num_seconds();
        rto.measured_seconds = Some(measured);
        rto.met = Some(measured <= spec.targets.rto_seconds);
        rto.over_by_seconds = (measured - spec.targets.rto_seconds).max(0);
    } else {
        rto.measured_seconds = None;
        rto.met = None;
        rto.over_by_seconds = 0;
    }
    rto
}

/// RPO evidence from the secondary's replication lag, against the declared
/// target.
pub fn rpo_evidence(
    spec: &TrafficShiftPlanSpec,
    secondary: &RegionHealthEvidence,
) -> Option<RpoEvidence> {
    secondary.replication_lag_seconds.map(|lag| RpoEvidence {
        measured_lag_seconds: lag,
        target_seconds: spec.targets.rpo_seconds,
        met: lag <= spec.targets.rpo_seconds,
        measured_at: secondary.last_probe_at,
    })
}

/// Compliance record for the DR report: one entry per plan with its measured
/// RTO/RPO, the steps it executed and whether the declared targets were met.
pub fn drill_compliance_record(plan: &TrafficShiftPlan) -> serde_json::Value {
    let status = plan.status.clone().unwrap_or_default();
    let spec = &plan.spec;
    let rto = measure_rto(spec, &status);
    json!({
        "plan": plan.name_any(),
        "namespace": plan.namespace().unwrap_or_else(|| "default".into()),
        "drillId": spec.drill_id,
        "mode": if spec.targets.drill { "drill" } else { "live" },
        "primary": spec.primary.name,
        "secondary": spec.secondary.name,
        "direction": status.direction,
        "phase": status.phase,
        "rto": {
            "targetSeconds": rto.target_seconds,
            "measuredSeconds": rto.measured_seconds,
            "met": rto.met,
            "overBySeconds": rto.over_by_seconds,
        },
        "rpo": status.rpo.as_ref().map(|r| json!({
            "targetSeconds": r.target_seconds,
            "measuredSeconds": r.measured_lag_seconds,
            "met": r.met,
        })),
        "steps": status.steps.iter().map(|s| json!({
            "index": s.index,
            "direction": s.direction,
            "fromPrimaryWeight": s.from_primary_weight,
            "toPrimaryWeight": s.to_primary_weight,
            "outcome": s.outcome,
            "startedAt": s.started_at,
            "soakEndsAt": s.soak_ends_at,
            "message": s.message,
        })).collect::<Vec<_>>(),
        "rtoMet": rto.met.unwrap_or(false),
    })
}

/// One-screen, operator-readable rendering of the plan status.
pub fn render_summary(spec: &TrafficShiftPlanSpec, status: &TrafficShiftPlanStatus) -> String {
    let weights = &status.current_weights;
    let gate = status.last_gate.as_ref();
    let mut out = format!(
        "phase={:?} direction={:?} weights={}/{} ({}={}%, {}={}%)",
        status.phase,
        status.direction,
        weights.primary_percent,
        weights.secondary_percent,
        spec.primary.name,
        weights.primary_percent,
        spec.secondary.name,
        weights.secondary_percent
    );
    match gate {
        Some(g) if g.open => out.push_str(&format!(" gate=open ({})", g.reason)),
        Some(g) => out.push_str(&format!(" gate=closed ({})", g.reason)),
        None => out.push_str(" gate=unevaluated"),
    }
    if let Some(rto) = status.rto.measured_seconds {
        out.push_str(&format!(
            " rto={}s/{}s {}",
            rto,
            status.rto.target_seconds,
            match status.rto.met {
                Some(true) => "MET",
                Some(false) => "MISSED",
                None => "UNKNOWN",
            }
        ));
    }
    out.push_str(&format!(" steps={}", status.steps.len()));
    out
}

/// Reconcile one `TrafficShiftPlan`.
///
/// One cycle: probe both regions, evaluate the gate, take at most one state
/// machine action, publish the resulting weighted record and record everything
/// in the plan status.
pub async fn reconcile_traffic_shift_plan(
    client: &Client,
    plan: &TrafficShiftPlan,
    probe: Arc<dyn RegionHealthProbe>,
) -> Result<TrafficShiftPlanStatus> {
    let spec = &plan.spec;
    let now = Utc::now();
    let name = plan.name_any();
    let namespace = plan.namespace().unwrap_or_else(|| "default".to_string());

    let primary_probes = probe.probe(&spec.primary, &spec.health_gate).await?;
    let secondary_probes = probe.probe(&spec.secondary, &spec.health_gate).await?;
    let primary = collect_evidence(&spec.primary, &primary_probes, &spec.health_gate, now);
    let secondary = collect_evidence(&spec.secondary, &secondary_probes, &spec.health_gate, now);

    let status = reconcile_cycle_with_control(
        spec,
        plan.status.clone().unwrap_or_default(),
        &primary,
        &secondary,
        plan.metadata.generation,
        now,
        plan_control(plan),
    );
    let status = match status {
        Ok(s) => s,
        Err((mut failed, reason)) => {
            // A plan that cannot progress is failed loudly rather than
            // silently left half-shifted.
            failed.phase = TrafficShiftPhase::Failed;
            failed.summary = reason.clone();
            failed.rto = measure_rto(spec, &failed);
            failed.conditions = conditions_for(&failed);
            failed
        }
    };

    info!(
        plan = %name,
        namespace = %namespace,
        phase = ?status.phase,
        direction = ?status.direction,
        primary_pct = status.current_weights.primary_percent,
        "TrafficShiftPlan reconciled"
    );
    record_metrics(&namespace, &name, &status);

    // Declarative DNS: the rendered record is mirrored into an annotation so
    // the live DNS state is reviewable with `kubectl get`, and only when it
    // actually changed, so an idle plan does not churn the API server.
    let previous_record = plan.status.as_ref().and_then(|s| s.applied_record.clone());
    if status.applied_record.is_some() && status.applied_record != previous_record {
        let api: Api<TrafficShiftPlan> = Api::namespaced(client.clone(), &namespace);
        api.patch(
            &name,
            &PatchParams::apply("stellar-operator-traffic-shift"),
            &Patch::Apply(json!({
                "apiVersion": "stellar.org/v1alpha1",
                "kind": "TrafficShiftPlan",
                "metadata": {
                    "name": name,
                    "annotations": {
                        APPLIED_RECORD_ANNOTATION: status.applied_record.to_string(),
                    },
                },
            })),
        )
        .await
        .map_err(Error::KubeError)?;
    }

    let api: Api<TrafficShiftPlan> = Api::namespaced(client.clone(), &namespace);
    api.patch_status(
        &name,
        &PatchParams::default(),
        &Patch::Merge(json!({ "status": status })),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(status)
}

/// One deterministic reconcile cycle, without a cluster or a network.
///
/// Picks the direction, asks the state machine for the next action, applies it
/// and refreshes the derived fields (RTO, RPO, summary, conditions). The
/// returned `Err` carries the failed status so callers can still persist it.
pub fn reconcile_cycle(
    spec: &TrafficShiftPlanSpec,
    previous: TrafficShiftPlanStatus,
    primary: &RegionHealthEvidence,
    secondary: &RegionHealthEvidence,
    generation: i64,
    now: DateTime<Utc>,
) -> Result<TrafficShiftPlanStatus, (TrafficShiftPlanStatus, String)> {
    reconcile_cycle_with_control(
        spec,
        previous,
        primary,
        secondary,
        generation,
        now,
        PlanControl::default(),
    )
}

/// [`reconcile_cycle`] with operator overrides applied.
pub fn reconcile_cycle_with_control(
    spec: &TrafficShiftPlanSpec,
    previous: TrafficShiftPlanStatus,
    primary: &RegionHealthEvidence,
    secondary: &RegionHealthEvidence,
    generation: i64,
    now: DateTime<Utc>,
    control: PlanControl,
) -> Result<TrafficShiftPlanStatus, (TrafficShiftPlanStatus, String)> {
    let mut status = previous;
    if status.current_weights.primary_percent == 0 && status.current_weights.secondary_percent == 0
    {
        status.current_weights =
            TrafficWeights::from_primary_percent(spec.routing.initial_primary_weight);
    }
    status.observed_generation = Some(generation);
    status.last_evaluated_at = Some(now);
    status.health = [
        (spec.primary.name.clone(), primary.clone()),
        (spec.secondary.name.clone(), secondary.clone()),
    ]
    .into_iter()
    .collect();

    // A new direction only ever starts once the previous one reached its
    // target, or after an operator reset it.
    if status.phase.is_terminal() {
        let desired = desired_direction(spec, &status, primary, secondary, now);
        if desired != status.direction {
            status.direction = desired;
            status.phase = TrafficShiftPhase::Gated;
            status.started_at = None;
            status.completed_at = None;
        }
    }

    let action = next_action_with_control(spec, &status, primary, secondary, now, control);
    apply_action(spec, &mut status, &action, primary, secondary, now);

    if status.phase != TrafficShiftPhase::Failed {
        if let Some(gate) = status.last_gate.as_ref() {
            if gate.open {
                status.rpo = rpo_evidence(spec, secondary);
            }
        }
    }
    status.rto = measure_rto(spec, &status);
    status.summary = render_summary(spec, &status);
    status.conditions = conditions_for(&status);

    match &action {
        ShiftAction::Fail { reason } => Err((status, reason.clone())),
        _ => Ok(status),
    }
}

/// Is the plan explicitly suppressed for `direction`?
pub fn is_suppressed(plan: &TrafficShiftPlan, direction: ShiftDirection) -> bool {
    plan.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(SUPPRESS_ANNOTATION))
        .map(|v| {
            v.split(',').any(|d| {
                d.trim().eq_ignore_ascii_case(match direction {
                    ShiftDirection::Failover => "failover",
                    ShiftDirection::Failback => "failback",
                })
            })
        })
        .unwrap_or(false)
}

/// Did an operator request a failover on a `Manual` plan?
pub fn failover_requested(plan: &TrafficShiftPlan) -> bool {
    plan.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(TRIGGER_FAILOVER_ANNOTATION))
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
        .unwrap_or(false)
}

/// Apply a state machine action to a status, recording the step.
pub fn apply_action(
    spec: &TrafficShiftPlanSpec,
    status: &mut TrafficShiftPlanStatus,
    action: &ShiftAction,
    primary: &RegionHealthEvidence,
    secondary: &RegionHealthEvidence,
    now: DateTime<Utc>,
) {
    if status.phase != TrafficShiftPhase::Idle {
        status.last_transition_time = Some(now);
    }
    let gate = evaluate_gate(
        spec,
        action.direction().unwrap_or(status.direction),
        primary,
        secondary,
        now,
    );
    status.last_gate = Some(gate);

    match action {
        ShiftAction::Fail { reason } => {
            status.phase = TrafficShiftPhase::Failed;
            push_summary(status, reason);
        }
        ShiftAction::Idle { phase } => {
            status.phase = *phase;
        }
        ShiftAction::Hold { reason, .. } => {
            // Hold where we are: no weight moves, the gate is recorded.
            if !status.phase.is_in_progress() {
                status.phase = TrafficShiftPhase::Gated;
            }
            status.summary = reason.clone();
        }
        ShiftAction::Wait { until, reason } => {
            status.phase = TrafficShiftPhase::Gated;
            status.summary = format!("{} (until {until})", reason);
        }
        ShiftAction::Drain {
            direction,
            step_index,
            from_primary_weight,
            to_primary_weight,
            ..
        } => {
            status.direction = *direction;
            status.phase = TrafficShiftPhase::Draining;
            if status.started_at.is_none() {
                status.started_at = Some(now);
            }
            // A clean soak is what closes out the previous increment; starting
            // the next one is the proof that it passed.
            close_out_soaked_steps(status, *direction, *step_index, now);
            upsert_step(
                status,
                ShiftStepStatus {
                    index: *step_index,
                    direction: *direction,
                    from_primary_weight: *from_primary_weight,
                    to_primary_weight: *to_primary_weight,
                    record_ttl_seconds: spec.routing.ttl_seconds,
                    drain_started_at: Some(now),
                    started_at: None,
                    outcome: StepOutcome::Draining,
                    message: format!("draining {}s before publish", spec.shift.drain_seconds),
                    ..Default::default()
                },
            );
        }
        ShiftAction::Publish {
            direction,
            step_index,
            from_primary_weight,
            to_primary_weight,
            record,
            ttl_seconds,
        } => {
            status.direction = *direction;
            status.phase = TrafficShiftPhase::Shifting;
            if status.started_at.is_none() {
                status.started_at = Some(now);
            }
            close_out_soaked_steps(status, *direction, *step_index, now);
            status.applied_record = Some(record.clone());
            upsert_step(
                status,
                ShiftStepStatus {
                    index: *step_index,
                    direction: *direction,
                    from_primary_weight: *from_primary_weight,
                    to_primary_weight: *to_primary_weight,
                    record: Some(record.clone()),
                    record_ttl_seconds: *ttl_seconds,
                    drain_started_at: None,
                    started_at: Some(now),
                    soak_ends_at: Some(propagation_deadline(spec, now)),
                    outcome: StepOutcome::Applied,
                    message: format!(
                        "published {from_primary_weight}% -> {to_primary_weight}% on the primary"
                    ),
                    ..Default::default()
                },
            );
            status.current_weights = TrafficWeights::from_primary_percent(*to_primary_weight);
        }
        ShiftAction::Soak {
            direction,
            step_index,
            until,
        } => {
            status.direction = *direction;
            status.phase = TrafficShiftPhase::Soaking;
            let rate = soak_error_rate(*direction, primary, secondary);
            if let Some(step) = status
                .steps
                .iter_mut()
                .find(|s| s.direction == *direction && s.index == *step_index)
            {
                step.soak_ends_at = Some(*until);
                // Keep the worst reading seen during the soak, so the recorded
                // value reflects the window and not just its last second.
                step.soak_error_rate_percent = step.soak_error_rate_percent.max(rate);
            }
        }
        ShiftAction::Abort {
            direction,
            step_index,
            reason,
        } => {
            // Hold the last safe increment: never roll weight back
            // automatically, never push further.
            status.direction = *direction;
            status.phase = TrafficShiftPhase::Aborted;
            if let Some(step) = status
                .steps
                .iter_mut()
                .find(|s| s.direction == *direction && s.index == *step_index)
            {
                step.outcome = StepOutcome::Aborted;
                step.completed_at = Some(now);
                step.message = reason.clone();
            }
            push_summary(status, reason);
        }
        ShiftAction::Complete { direction } => {
            status.direction = *direction;
            status.phase = TrafficShiftPhase::Completed;
            status.completed_at = Some(now);
            if *direction == ShiftDirection::Failover {
                // Start the failback cooldown from the moment the secondary
                // became the sole server.
                status.serving_since = Some(now);
            }
            let target = target_weight(spec, *direction);
            status.current_weights = TrafficWeights::from_primary_percent(target);
            if let Some(step) = status
                .steps
                .iter_mut()
                .rev()
                .find(|s| s.direction == *direction)
            {
                if step.outcome == StepOutcome::Applied {
                    step.outcome = StepOutcome::Succeeded;
                    step.completed_at = Some(now);
                }
            }
        }
    }

    status.steps_completed = status.steps.iter().filter(|s| s.outcome.is_done()).count() as u32;
}

/// Mark every earlier increment of `direction` whose soak has run its course
/// as succeeded, so `stepsCompleted` and the audit trail stay accurate.
fn close_out_soaked_steps(
    status: &mut TrafficShiftPlanStatus,
    direction: ShiftDirection,
    before_index: u32,
    now: DateTime<Utc>,
) {
    for step in status.steps.iter_mut() {
        if step.direction == direction
            && step.index < before_index
            && step.outcome == StepOutcome::Applied
        {
            step.outcome = StepOutcome::Succeeded;
            step.completed_at = Some(now);
        }
    }
}

/// Record a step, replacing any earlier record of the same increment.
///
/// The drain timestamp is carried across the transition from `Draining` to
/// `Applied` so the audit trail shows when the losing region started draining
/// as well as when its weight actually moved.
fn upsert_step(status: &mut TrafficShiftPlanStatus, step: ShiftStepStatus) {
    match status
        .steps
        .iter_mut()
        .find(|s| s.direction == step.direction && s.index == step.index)
    {
        Some(existing) => {
            let drained = existing.drain_started_at.or(step.drain_started_at);
            let soaked = existing
                .soak_error_rate_percent
                .max(step.soak_error_rate_percent);
            *existing = step;
            existing.drain_started_at = drained;
            existing.soak_error_rate_percent = soaked;
        }
        None => status.steps.push(step),
    }
}

fn push_summary(status: &mut TrafficShiftPlanStatus, reason: &str) {
    status.summary = reason.to_string();
}

fn conditions_for(status: &TrafficShiftPlanStatus) -> Vec<Condition> {
    let generation = status.observed_generation.unwrap_or_default();
    let mut conditions = Vec::new();
    let met = status.rto.met.unwrap_or(false);
    conditions.push(
        Condition::ready(
            matches!(
                status.phase,
                TrafficShiftPhase::Idle | TrafficShiftPhase::Completed
            ) && met,
            match status.phase {
                TrafficShiftPhase::Failed => "InvalidPlan",
                TrafficShiftPhase::Aborted => "GateClosedMidShift",
                _ if met => "TargetWeightsReached",
                _ => "AwaitingRtoEvidence",
            },
            &status.summary,
        )
        .with_observed_generation(generation),
    );
    if status.phase == TrafficShiftPhase::Failed {
        conditions.push(
            Condition::degraded("InvalidPlan", &status.summary)
                .with_observed_generation(generation),
        );
    }
    if let Some(rpo) = &status.rpo {
        if !rpo.met {
            conditions.push(
                Condition::degraded(
                    "RpoTargetMissed",
                    &format!(
                        "replication lag {}s exceeds declared RPO {}s",
                        rpo.measured_lag_seconds, rpo.target_seconds
                    ),
                )
                .with_observed_generation(generation),
            );
        }
    }
    conditions
}

#[cfg(feature = "metrics")]
fn record_metrics(namespace: &str, name: &str, status: &TrafficShiftPlanStatus) {
    use crate::controller::metrics;

    let direction = status.direction.as_str();
    metrics::set_traffic_shift_phase(namespace, name, direction, status.phase);
    metrics::set_traffic_shift_primary_weight(
        namespace,
        name,
        status.current_weights.primary_percent,
    );
    if let Some(measured) = status.rto.measured_seconds {
        metrics::set_traffic_shift_rto_seconds(namespace, name, direction, measured);
    }
}

#[cfg(not(feature = "metrics"))]
fn record_metrics(_namespace: &str, _name: &str, _status: &TrafficShiftPlanStatus) {}
