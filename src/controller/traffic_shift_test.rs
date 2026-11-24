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

//! Deterministic tests for the health-gated traffic shift state machine
//! (epic #1504). No cluster and no network: the gate, the state machine, the
//! TTL/drain arithmetic and the RTO/RPO measurement are all pure functions of
//! a fixture clock.

use chrono::{DateTime, Duration, Utc};

use crate::controller::traffic_shift::*;
use crate::crd::traffic_shift_plan::{
    FailbackPolicy, FailoverTargets, FailoverTrigger, HealthGateSpec, RegionTarget,
    RoutingRecordSpec, ShiftPolicy, TrafficShiftPlan, TrafficShiftPlanSpec, TrafficShiftPlanStatus,
    TrafficWeights,
};

const T0: &str = "2026-01-01T00:00:00Z";

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
        + Duration::seconds(secs)
}

fn region(name: &str) -> RegionTarget {
    RegionTarget {
        name: name.into(),
        region: format!("{name}-region"),
        endpoint: format!("horizon.{name}.example.com"),
    }
}

fn spec() -> TrafficShiftPlanSpec {
    TrafficShiftPlanSpec {
        primary: region("eu-west"),
        secondary: region("us-east"),
        trigger: FailoverTrigger::Automatic,
        routing: RoutingRecordSpec {
            hostname: "horizon.stellar.example.com".into(),
            ttl_seconds: 60,
            initial_primary_weight: 100,
        },
        health_gate: HealthGateSpec {
            failure_threshold: 3,
            recovery_threshold: 5,
            min_success_rate_percent: 99.0,
            min_samples: 5,
            evidence_window_seconds: 300,
            timeout_seconds: 5,
        },
        shift: ShiftPolicy {
            step_percent: 25,
            soak_seconds: 120,
            drain_seconds: 30,
            max_soak_error_rate_percent: 1.0,
        },
        failback: FailbackPolicy {
            automatic: true,
            min_stable_seconds: 600,
            verify_final_step: true,
        },
        targets: FailoverTargets {
            rto_seconds: 900,
            rpo_seconds: 30,
            drill: false,
        },
        drill_id: Some("2026-q1-full-region".into()),
    }
}

/// Evidence with the given trailing run, for the given region.
fn evidence(
    region_name: &str,
    successes: u32,
    failures: u32,
    at_secs: i64,
) -> RegionHealthEvidence {
    let now = at(at_secs);
    let samples = successes + failures;
    RegionHealthEvidence {
        region: format!("{region_name}-region"),
        successes,
        failures,
        consecutive_successes: successes,
        consecutive_failures: failures,
        success_rate_percent: if samples == 0 {
            0.0
        } else {
            successes as f64 * 100.0 / samples as f64
        },
        error_rate_percent: 0.0,
        replication_lag_seconds: Some(5),
        last_probe_at: now,
        first_probe_at: now - Duration::seconds(10),
        last_error: None,
    }
}

fn healthy(name: &str, at_secs: i64) -> RegionHealthEvidence {
    evidence(name, 10, 0, at_secs)
}

fn down(name: &str, at_secs: i64) -> RegionHealthEvidence {
    evidence(name, 0, 10, at_secs)
}

fn initial_status() -> TrafficShiftPlanStatus {
    TrafficShiftPlanStatus {
        phase: TrafficShiftPhase::Idle,
        direction: ShiftDirection::Failover,
        current_weights: TrafficWeights::from_primary_percent(100),
        rto: RtoMeasurement {
            target_seconds: 900,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn plan_with(spec: &TrafficShiftPlanSpec, status: TrafficShiftPlanStatus) -> TrafficShiftPlan {
    let mut plan = TrafficShiftPlan::new(spec.clone(), initial_status());
    plan.metadata.name = Some("horizon-global".into());
    plan.metadata.namespace = Some("stellar".into());
    plan.metadata.generation = 3;
    plan.status = Some(status);
    plan
}

fn set_annotations(plan: &mut TrafficShiftPlan, kv: &[(&str, &str)]) {
    let annotations = plan
        .metadata
        .annotations
        .get_or_insert_with(Default::default);
    for (k, v) in kv {
        annotations.insert((*k).to_string(), (*v).to_string());
    }
}

// ── Health gate ──────────────────────────────────────────────────────────────

#[test]
fn gate_evaluates_primary_and_secondary_independently() {
    let s = spec();

    // Primary down, secondary proven healthy: failover opens.
    let open = evaluate_gate(
        &s,
        ShiftDirection::Failover,
        &down("eu-west", 0),
        &healthy("us-east", 0),
        at(0),
    );
    assert!(open.open, "{}", open.reason);
    assert_eq!(open.source.region, "eu-west-region");
    assert_eq!(open.target.region, "us-east-region");

    // Primary down but secondary has one fewer success than the threshold:
    // closed, and the reason names the secondary.
    let thin = evidence("us-east", 4, 0, 0);
    let closed = evaluate_gate(
        &s,
        ShiftDirection::Failover,
        &down("eu-west", 0),
        &thin,
        at(0),
    );
    assert!(!closed.open);
    assert!(
        closed.reason.contains("us-east-region"),
        "{}",
        closed.reason
    );

    // Primary is flapping but healthy: no failover. The gate needs proof of
    // failure, not merely the absence of proof of health.
    let flapping = evidence("eu-west", 9, 1, 0);
    let held = evaluate_gate(
        &s,
        ShiftDirection::Failover,
        &flapping,
        &healthy("us-east", 0),
        at(0),
    );
    assert!(!held.open);
    assert!(held.reason.contains("eu-west-region"), "{}", held.reason);
}

#[test]
fn gate_requires_min_samples_and_success_rate() {
    let s = spec();
    // One clean success: not enough evidence.
    let one = evidence("us-east", 1, 0, 0);
    assert!(
        !evaluate_gate(
            &s,
            ShiftDirection::Failover,
            &down("eu-west", 0),
            &one,
            at(0)
        )
        .open
    );
    // Enough samples but a dirty success rate.
    let dirty = evidence("us-east", 9, 1, 0);
    let g = evaluate_gate(
        &s,
        ShiftDirection::Failover,
        &down("eu-west", 0),
        &dirty,
        at(0),
    );
    assert!(!g.open);
    assert!(g.reason.contains("90.00%"), "{}", g.reason);
    // And the recorded thresholds echo the spec, so the decision is auditable.
    assert_eq!(g.applied.recovery_threshold, 5);
    assert_eq!(g.applied.failure_threshold, 3);
    assert_eq!(g.applied.min_samples, 5);
}

#[test]
fn stale_evidence_never_opens_the_gate() {
    let s = spec();
    let target = healthy("us-east", 0);
    // Target evidence is older than the evidence window.
    let stale = evaluate_gate(
        &s,
        ShiftDirection::Failover,
        &down("eu-west", 0),
        &target,
        at(s.health_gate.evidence_window_seconds + 1),
    );
    assert!(!stale.open);
    assert!(stale.reason.contains("older than"), "{}", stale.reason);
}

#[test]
#[test]
fn failback_needs_exactly_the_same_evidence_as_failover() {
    let s = spec();
    let secondary = healthy("us-east", 0);

    // The primary recovered but has not yet produced `recovery_threshold`
    // consecutive successes: the same bar that once kept traffic on the
    // secondary. Failback stays closed.
    let recovering = evidence("eu-west", 3, 0, 0);
    let closed = evaluate_gate(&s, ShiftDirection::Failback, &recovering, &secondary, at(0));
    assert!(!closed.open);
    assert!(
        closed.reason.contains("eu-west-region"),
        "{}",
        closed.reason
    );

    // With the full evidence the gate opens, and the thresholds applied are the
    // very same struct failover used: there is no laxer bar for the way back.
    let recovered = healthy("eu-west", 0);
    let open = evaluate_gate(&s, ShiftDirection::Failback, &recovered, &secondary, at(0));
    assert!(open.open, "{}", open.reason);
    let failover_gate = evaluate_gate(
        &s,
        ShiftDirection::Failover,
        &down("eu-west", 0),
        &secondary,
        at(0),
    );
    assert!(failover_gate.open, "{}", failover_gate.reason);
    assert_eq!(open.applied, failover_gate.applied);

    // The evidence that admits the primary as a failback target is exactly the
    // evidence that admitted it as a failover target.
    assert_eq!(open.target.region, "eu-west-region");
    assert_eq!(open.source.region, "us-east-region");
    assert_eq!(open.target.consecutive_successes, 10);
}

#[test]
fn collect_evidence_folds_probes_into_runs_and_rates() {
    let s = spec();
    let now = at(600);
    let target = region("us-east");
    let probes = vec![
        ProbeResult {
            at: now - Duration::seconds(200),
            success: true,
            latency_ms: Some(10),
            replication_lag_seconds: Some(4),
            error_rate_percent: 0.0,
            error: None,
        },
        ProbeResult {
            at: now - Duration::seconds(100),
            success: false,
            latency_ms: None,
            replication_lag_seconds: None,
            error_rate_percent: 100.0,
            error: Some("connection refused".into()),
        },
        ProbeResult {
            at: now - Duration::seconds(10),
            success: true,
            latency_ms: Some(12),
            replication_lag_seconds: Some(6),
            error_rate_percent: 0.0,
            error: None,
        },
        // Outside the evidence window: ignored entirely.
        ProbeResult {
            at: now - Duration::seconds(1000),
            success: false,
            latency_ms: None,
            replication_lag_seconds: None,
            error_rate_percent: 100.0,
            error: Some("ancient".into()),
        },
    ];
    let ev = collect_evidence(&target, &probes, &s.health_gate, now);
    assert_eq!(ev.successes, 2);
    assert_eq!(ev.failures, 1);
    assert_eq!(ev.consecutive_successes, 1);
    assert_eq!(ev.consecutive_failures, 0);
    assert!((ev.success_rate_percent - 66.666).abs() < 0.01);
    assert_eq!(ev.replication_lag_seconds, Some(6));
    assert_eq!(ev.last_error.as_deref(), Some("connection refused"));
    assert_eq!(ev.region, "us-east-region");
}

#[test]
fn collect_evidence_reports_consecutive_failure_runs() {
    let s = spec();
    let now = at(600);
    let target = region("eu-west");
    let probes: Vec<ProbeResult> = (0..5)
        .map(|i| ProbeResult {
            at: now - Duration::seconds(50 - i as i64 * 10),
            success: false,
            latency_ms: None,
            replication_lag_seconds: None,
            error_rate_percent: 100.0,
            error: Some("timeout".into()),
        })
        .collect();
    let ev = collect_evidence(&target, &probes, &s.health_gate, now);
    assert_eq!(ev.consecutive_failures, 5);
    assert_eq!(ev.consecutive_successes, 0);
    assert!(ev.is_unhealthy(&s.health_gate));
    assert!(!ev.is_healthy(&s.health_gate, now));
}

// ── Step arithmetic ──────────────────────────────────────────────────────────

#[test]
fn steps_reach_the_target_without_overshoot() {
    let s = spec();
    // 100 -> 0 in 25% increments: exactly four steps landing on 0.
    assert_eq!(step_count(100, 0, 25), 4);
    let weights: Vec<u32> = (0..4).map(|i| weight_after_step(100, 0, 25, i)).collect();
    assert_eq!(weights, vec![75, 50, 25, 0]);

    // A step size that does not divide the distance still lands on target.
    assert_eq!(step_count(100, 0, 30), 4);
    assert_eq!(weight_after_step(100, 0, 30, 3), 10);
    assert_eq!(weight_after_step(100, 0, 30, 4), 0);
    // And a 100% step is a single all-at-once move.
    assert_eq!(step_count(100, 0, 100), 1);
    assert_eq!(weight_after_step(100, 0, 100, 0), 0);
}

#[test]
fn drain_and_propagation_respect_ttl_and_soak() {
    let s = spec();
    let start = at(0);
    assert_eq!(drain_deadline(&s, start), at(30));
    // soak (120s) is longer than the TTL (60s), so the soak wins.
    assert_eq!(propagation_deadline(&s, start), at(120));

    // With a soak shorter than the TTL, the TTL is the lower bound: resolvers
    // must have expired the previous record before the next increment.
    let mut short_soak = s.clone();
    short_soak.shift.soak_seconds = 5;
    assert_eq!(propagation_deadline(&short_soak, start), at(60));

    // 4 steps x (30s drain + 120s propagation) = 600s, inside the 900s RTO.
    assert_eq!(projected_duration_secs(&s, 100, 0), 600);
    assert!(validate_plan(&s).is_empty());
}

#[test]
fn validate_plan_flags_plans_that_cannot_meet_their_rto() {
    let mut s = spec();
    s.targets.rto_seconds = 120;
    let problems = validate_plan(&s);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("spec.targets.rtoSeconds")),
        "{problems:?}"
    );

    let mut bad = spec();
    bad.shift.step_percent = 0;
    bad.health_gate.recovery_threshold = 0;
    bad.primary = bad.secondary.clone();
    let problems = validate_plan(&bad);
    assert!(problems.iter().any(|p| p.contains("stepPercent")));
    assert!(problems.iter().any(|p| p.contains("recoveryThreshold")));
    assert!(problems.iter().any(|p| p.contains("different regions")));
}

#[test]
fn weighted_record_is_declarative_and_carries_the_ttl() {
    let s = spec();
    let record = weighted_record(&s, 75, 0);
    assert_eq!(record["kind"], "DNSEndpoint");
    assert_eq!(
        record["spec"]["endpoints"][0]["dnsName"],
        "horizon.stellar.example.com"
    );
    assert_eq!(record["spec"]["endpoints"][0]["recordTTL"], 60);
    let targets = &record["spec"]["endpoints"][0]["targets"];
    assert_eq!(targets[0]["target"], "horizon.eu-west.example.com");
    assert_eq!(targets[0]["weight"], 75);
    assert_eq!(targets[1]["target"], "horizon.us-east.example.com");
    assert_eq!(targets[1]["weight"], 25);
    // Weights always sum to 100 so resolvers never see a partial shift.
    assert_eq!(
        targets[0]["weight"].as_u64().unwrap() + targets[1]["weight"].as_u64().unwrap(),
        100
    );
    assert_eq!(
        record["metadata"]["annotations"]["stellar.org/managed-by"],
        "traffic-shift-plan"
    );
}

// ── State machine ────────────────────────────────────────────────────────────

/// One step of the pure state machine, returning the action taken.
fn step_once(
    spec: &TrafficShiftPlanSpec,
    status: &mut TrafficShiftPlanStatus,
    p: &RegionHealthEvidence,
    s: &RegionHealthEvidence,
    t: i64,
) -> ShiftAction {
    let now = at(t);
    let action = next_action(spec, status, p, s, now);
    *status = reconcile_cycle(spec, status.clone(), p, s, 1, now).unwrap_or_else(|(st, _)| st);
    action
}

/// Drive the pure state machine one simulated second at a time from
/// `from_secs`, returning the final status and every action that changed
/// traffic.
fn drive(
    spec: &TrafficShiftPlanSpec,
    start: TrafficShiftPlanStatus,
    primary_at: impl Fn(i64) -> RegionHealthEvidence,
    secondary_at: impl Fn(i64) -> RegionHealthEvidence,
    from_secs: i64,
    horizon_secs: i64,
) -> (TrafficShiftPlanStatus, Vec<ShiftAction>) {
    let mut status = start;
    let mut log = Vec::new();
    for t in from_secs..from_secs + horizon_secs {
        let action = step_once(spec, &mut status, &primary_at(t), &secondary_at(t), t);
        if action.mutates_traffic() {
            log.push(action);
        }
    }
    (status, log)
}

#[test]
fn failover_walks_the_increments_with_soaks_in_between() {
    let s = spec();
    let status = initial_status();
    let down_primary = |_t: i64| down("eu-west", 0);
    let up_secondary = |t: i64| healthy("us-east", t);

    // Gate closed while the primary still has only two consecutive failures.
    let mut st = status.clone();
    let thin = evidence("eu-west", 0, 2, 0);
    let a = next_action(&s, &st, &thin, &up_secondary(0), at(0));
    assert!(matches!(a, ShiftAction::Hold { .. }), "{a:?}");
    assert_eq!(st.current_weights.primary_percent, 100, "no weight moves");

    // Full run: 30s drain + 120s soak per step, four steps.
    let (final_status, published) = drive(&s, status, down_primary, up_secondary, 0, 1200);
    assert_eq!(published.len(), 4, "exactly four increments published");
    assert_eq!(final_status.phase, TrafficShiftPhase::Completed);
    assert_eq!(final_status.current_weights.primary_percent, 0);
    assert_eq!(final_status.current_weights.secondary_percent, 100);

    // Every step is recorded, in order, with its weights and its record.
    let steps = &final_status.steps;
    assert_eq!(steps.len(), 4);
    let pairs: Vec<(u32, u32)> = steps
        .iter()
        .map(|s| (s.from_primary_weight, s.to_primary_weight))
        .collect();
    assert_eq!(pairs, vec![(100, 75), (75, 50), (50, 25), (25, 0)]);
    assert!(steps.iter().all(|s| s.outcome == StepOutcome::Succeeded));
    assert!(steps.iter().all(|s| s.record.is_some()));
    assert_eq!(
        steps.iter().map(|s| s.index).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(final_status.steps_completed, 4);

    // No increment skipped a drain or a soak, and the audit trail records when
    // the losing region started draining as well as when the weight moved.
    for step in steps {
        let drained = step.drain_started_at.expect("drain recorded");
        let published = step.started_at.expect("published");
        assert!(published >= drained, "the drain came first");
        assert!(
            published - drained >= Duration::seconds(30),
            "drain honoured"
        );
        let soak_end = step.soak_ends_at.expect("soak scheduled");
        assert!(
            soak_end - published >= Duration::seconds(120),
            "soak honoured"
        );
    }
}

#[test]
fn drain_completes_before_any_weight_moves() {
    let s = spec();
    let mut st = initial_status();
    // First cycle: the gate is open but connections must drain first.
    match step_once(&s, &mut st, &down("eu-west", 0), &healthy("us-east", 0), 0) {
        ShiftAction::Drain { until, .. } => assert_eq!(until, at(30)),
        other => panic!("expected Drain, got {other:?}"),
    }
    // Halfway through the drain: still no publish, and the weights are intact.
    let mid = step_once(
        &s,
        &mut st,
        &down("eu-west", 15),
        &healthy("us-east", 15),
        15,
    );
    assert!(matches!(mid, ShiftAction::Drain { .. }), "{mid:?}");
    assert_eq!(st.current_weights.primary_percent, 100);
    // After the drain: the record is published, and nothing else moves yet.
    let after = step_once(
        &s,
        &mut st,
        &down("eu-west", 30),
        &healthy("us-east", 30),
        30,
    );
    assert!(after.mutates_traffic(), "{after:?}");
    assert_eq!(after.direction(), Some(ShiftDirection::Failover));
    assert_eq!(st.current_weights.primary_percent, 75);
    // The next cycle soaks rather than publishing a second increment.
    let soak = step_once(
        &s,
        &mut st,
        &down("eu-west", 31),
        &healthy("us-east", 31),
        31,
    );
    assert!(matches!(soak, ShiftAction::Soak { .. }), "{soak:?}");
    assert_eq!(st.current_weights.primary_percent, 75);
}

#[test]
fn a_single_step_never_moves_more_than_the_configured_increment() {
    let s = spec();
    let mut st = initial_status();
    // Run until the first publish and check the delta.
    for t in 0..60 {
        let a = step_once(&s, &mut st, &down("eu-west", t), &healthy("us-east", t), t);
        if let ShiftAction::Publish {
            from_primary_weight,
            to_primary_weight,
            ..
        } = &a
        {
            assert_eq!(*from_primary_weight, 100);
            assert_eq!(*to_primary_weight, 75);
            assert_eq!(100 - *to_primary_weight, s.shift.step_percent);
            return;
        }
    }
    panic!("no increment published within the horizon");
}

#[test]
fn gate_closing_mid_shift_holds_the_last_safe_increment() {
    let s = spec();
    let mut st = initial_status();
    // Get one increment published.
    for t in 0..60 {
        let a = step_once(&s, &mut st, &down("eu-west", t), &healthy("us-east", t), t);
        if a.mutates_traffic() {
            break;
        }
    }
    assert_eq!(st.current_weights.primary_percent, 75);
    let held = st.current_weights.primary_percent;

    // The secondary now degrades mid-soak: the gate closes, the shift stops.
    for t in 60..300 {
        step_once(
            &s,
            &mut st,
            &down("eu-west", t),
            &evidence("us-east", 0, 6, t),
            t,
        );
        assert_eq!(
            st.current_weights.primary_percent, held,
            "weights moved while the gate was closed at t={t}"
        );
    }
    assert_ne!(st.phase, TrafficShiftPhase::Completed);
    assert_eq!(st.steps.len(), 1, "no further steps were started");
}

#[test]
fn soak_error_budget_aborts_the_shift() {
    let s = spec();
    let mut st = initial_status();
    // Publish the first increment, then let the soak start.
    for t in 0..60 {
        let a = step_once(&s, &mut st, &down("eu-west", t), &healthy("us-east", t), t);
        if a.mutates_traffic() {
            step_once(
                &s,
                &mut st,
                &down("eu-west", t + 1),
                &healthy("us-east", t + 1),
                t + 1,
            );
            break;
        }
    }
    assert_eq!(st.phase, TrafficShiftPhase::Soaking);
    assert_eq!(st.current_weights.primary_percent, 75);

    // The soak expires, but the region that took the weight reports an
    // unacceptable error rate.
    let mut unhealthy_soak = healthy("us-east", 200);
    unhealthy_soak.error_rate_percent = 12.0;
    let st_out = reconcile_cycle(
        &s,
        st.clone(),
        &down("eu-west", 200),
        &unhealthy_soak,
        1,
        at(200),
    )
    .unwrap();
    assert_eq!(st_out.phase, TrafficShiftPhase::Aborted);
    assert_eq!(
        st_out.current_weights.primary_percent, 75,
        "held where it was"
    );
    assert_eq!(st_out.steps[0].outcome, StepOutcome::Aborted);
    assert!(
        st_out.steps[0].message.contains("12.00%"),
        "{}",
        st_out.steps[0].message
    );

    // An aborted plan resumes only once the error budget recovers.
    let recovered = reconcile_cycle(
        &s,
        st_out,
        &down("eu-west", 400),
        &healthy("us-east", 400),
        1,
        at(400),
    )
    .unwrap();
    assert_ne!(recovered.phase, TrafficShiftPhase::Aborted);
}

#[test]
fn failback_mirrors_failover_through_the_same_gates() {
    let s = spec();
    // Fail over completely: four increments of 150s each.
    let (st, _) = drive(
        &s,
        initial_status(),
        |_t| down("eu-west", 0),
        |t| healthy("us-east", t),
        0,
        1200,
    );
    assert_eq!(st.phase, TrafficShiftPhase::Completed);
    assert_eq!(st.direction, ShiftDirection::Failover);
    assert_eq!(st.rto.measured_seconds, Some(600));
    assert_eq!(st.rto.met, Some(true));
    assert_eq!(st.serving_since, Some(at(600)));

    // The primary answers, but not with enough evidence: the plan stays put.
    let primed = reconcile_cycle(
        &s,
        st.clone(),
        &evidence("eu-west", 3, 0, 900),
        &healthy("us-east", 900),
        1,
        at(900),
    )
    .unwrap();
    assert_eq!(primed.direction, ShiftDirection::Failover);
    assert_eq!(primed.phase, TrafficShiftPhase::Completed);
    assert_eq!(primed.current_weights.primary_percent, 0);

    // Full evidence arms the failback, but the cooldown holds the weights.
    let primed = reconcile_cycle(
        &s,
        st,
        &healthy("eu-west", 900),
        &healthy("us-east", 900),
        1,
        at(900),
    )
    .unwrap();
    assert_eq!(primed.direction, ShiftDirection::Failback);
    assert_eq!(primed.phase, TrafficShiftPhase::Gated);
    assert_eq!(primed.current_weights.primary_percent, 0);
    let waiting = next_action(
        &s,
        &primed,
        &healthy("eu-west", 1000),
        &healthy("us-east", 1000),
        at(1000),
    );
    assert!(matches!(waiting, ShiftAction::Wait { .. }), "{waiting:?}");

    // The cooldown elapses: the same gate now walks traffic back in 25%
    // increments, each with its own drain and soak.
    let (back, published) = drive(
        &s,
        primed,
        |t| healthy("eu-west", t),
        |t| healthy("us-east", t),
        1200,
        1200,
    );
    assert_eq!(published.len(), 4, "four failback increments");
    assert_eq!(back.phase, TrafficShiftPhase::Completed);
    assert_eq!(back.direction, ShiftDirection::Failback);
    assert_eq!(back.current_weights.primary_percent, 100);
    assert_eq!(back.current_weights.secondary_percent, 0);
    let back_steps: Vec<(u32, u32)> = back
        .steps
        .iter()
        .filter(|s| s.direction == ShiftDirection::Failback)
        .map(|s| (s.from_primary_weight, s.to_primary_weight))
        .collect();
    assert_eq!(back_steps, vec![(0, 25), (25, 50), (50, 75), (75, 100)]);
    assert!(back.rto.measured_seconds.unwrap() <= s.targets.rto_seconds);
}

#[test]
fn failback_does_not_flip_back_on_a_flapping_primary() {
    let s = spec();
    // Plan already fully failed over.
    let mut st = initial_status();
    st.current_weights = TrafficWeights::from_primary_percent(0);
    st.phase = TrafficShiftPhase::Completed;
    st.completed_at = Some(at(-5000));
    st.direction = ShiftDirection::Failover;

    // The primary answers, but not cleanly enough to be trusted.
    let flapping = evidence("eu-west", 9, 1, 0);
    assert_eq!(
        desired_direction(&s, &st, &flapping, &healthy("us-east", 0), at(0)),
        ShiftDirection::Failover,
        "a flapping primary must not pull traffic back"
    );

    // Clean evidence does move it, and only then.
    assert_eq!(
        desired_direction(
            &s,
            &st,
            &healthy("eu-west", 0),
            &healthy("us-east", 0),
            at(0)
        ),
        ShiftDirection::Failback
    );
}

#[test]
fn failback_is_opt_in() {
    let mut s = spec();
    s.failback.automatic = false;
    let mut st = initial_status();
    st.current_weights = TrafficWeights::from_primary_percent(0);
    st.phase = TrafficShiftPhase::Completed;
    st.completed_at = Some(at(-5000));
    st.direction = ShiftDirection::Failover;
    assert_eq!(
        desired_direction(
            &s,
            &st,
            &healthy("eu-west", 0),
            &healthy("us-east", 0),
            at(0)
        ),
        ShiftDirection::Failover
    );
    // And the state machine never invents a failback step.
    let a = next_action(
        &s,
        &st,
        &healthy("eu-west", 0),
        &healthy("us-east", 0),
        at(0),
    );
    assert!(matches!(a, ShiftAction::Idle { .. }), "{a:?}");
}

#[test]
fn mid_failover_direction_is_never_abandoned() {
    let s = spec();
    let mut st = initial_status();
    st.current_weights = TrafficWeights::from_primary_percent(50);
    st.phase = TrafficShiftPhase::Soaking;
    st.direction = ShiftDirection::Failover;
    // Even with the primary fully healthy again, a failover in progress
    // finishes: switching direction halfway would move weight twice.
    assert_eq!(
        desired_direction(
            &s,
            &st,
            &healthy("eu-west", 0),
            &healthy("us-east", 0),
            at(0)
        ),
        ShiftDirection::Failover
    );
}

#[test]
fn manual_plans_need_an_explicit_request() {
    let mut s = spec();
    s.trigger = FailoverTrigger::Manual;
    let st = initial_status();
    let control = PlanControl::default();
    let a = next_action_with_control(
        &s,
        &st,
        &down("eu-west", 0),
        &healthy("us-east", 0),
        at(0),
        control,
    );
    match a {
        ShiftAction::Hold { reason, .. } => assert!(reason.contains(TRIGGER_FAILOVER_ANNOTATION)),
        other => panic!("expected Hold, got {other:?}"),
    }
    // With the annotation set, the same evidence starts the shift.
    let requested = PlanControl {
        failover_requested: true,
        ..control
    };
    let a = next_action_with_control(
        &s,
        &st,
        &down("eu-west", 0),
        &healthy("us-east", 0),
        at(0),
        requested,
    );
    assert!(matches!(a, ShiftAction::Drain { .. }), "{a:?}");
}

#[test]
fn suppression_holds_the_plan_without_touching_the_spec() {
    let s = spec();
    let st = initial_status();
    let control = PlanControl {
        suppress_failover: true,
        ..Default::default()
    };
    let a = next_action_with_control(
        &s,
        &st,
        &down("eu-west", 0),
        &healthy("us-east", 0),
        at(0),
        control,
    );
    match a {
        ShiftAction::Hold { reason, .. } => assert!(reason.contains("suppressed")),
        other => panic!("expected Hold, got {other:?}"),
    }
    // Suppressing failback only stops the return trip, not a failover.
    let failback_only = PlanControl {
        suppress_failback: true,
        ..Default::default()
    };
    assert!(!failback_only.is_suppressed(ShiftDirection::Failover));
    assert!(failback_only.is_suppressed(ShiftDirection::Failback));
}

#[test]
fn plan_control_reads_operator_annotations() {
    let s = spec();
    let mut plan = plan_with(&s, initial_status());
    assert_eq!(plan_control(&plan), PlanControl::default());

    set_annotations(
        &mut plan,
        &[
            (TRIGGER_FAILOVER_ANNOTATION, "true"),
            (SUPPRESS_ANNOTATION, "failback"),
        ],
    );
    let c = plan_control(&plan);
    assert!(c.failover_requested);
    assert!(!c.suppress_failover);
    assert!(c.suppress_failback);

    set_annotations(&mut plan, &[(SUPPRESS_ANNOTATION, "failover, failback")]);
    assert!(plan_control(&plan).suppress_failover);
    assert!(plan_control(&plan).suppress_failback);
}

// ── RTO / RPO and reporting ──────────────────────────────────────────────────

#[test]
fn rto_is_measured_against_the_declared_target() {
    let s = spec();
    let (met, _) = drive(
        &s,
        initial_status(),
        |_t| down("eu-west", 0),
        |t| healthy("us-east", t),
        0,
        1200,
    );
    // 4 increments x (30s drain + 120s propagation).
    assert_eq!(met.rto.measured_seconds, Some(600));
    assert_eq!(met.rto.target_seconds, 900);
    assert_eq!(met.rto.met, Some(true));
    assert_eq!(met.rto.over_by_seconds, 0);
    assert_eq!(met.rto.direction, ShiftDirection::Failover);

    // A plan whose own RTO is shorter than its shift is rejected up front.
    let mut tight = spec();
    tight.targets.rto_seconds = 300;
    let problems = validate_plan(&tight);
    assert!(!problems.is_empty(), "the plan is rejected up front");
    assert!(
        problems.iter().any(|p| p.contains("rtoSeconds")),
        "{problems:?}"
    );

    // And when the very same shift is measured against a target it cannot
    // meet, the miss is reported rather than hidden.
    let rto = measure_rto(&tight, &met);
    assert_eq!(rto.measured_seconds, Some(600));
    assert_eq!(rto.target_seconds, 300);
    assert_eq!(rto.met, Some(false));
    assert_eq!(rto.over_by_seconds, 300);
}

#[test]
fn rpo_evidence_uses_the_secondary_lag() {
    let s = spec();
    let mut secondary = healthy("us-east", 0);
    secondary.replication_lag_seconds = Some(12);
    let rpo = rpo_evidence(&s, &secondary).expect("lag reported");
    assert_eq!(rpo.measured_lag_seconds, 12);
    assert_eq!(rpo.target_seconds, 30);
    assert!(rpo.met);

    secondary.replication_lag_seconds = Some(90);
    let rpo = rpo_evidence(&s, &secondary).expect("lag reported");
    assert!(!rpo.met);
    assert_eq!(rpo.measured_lag_seconds, 90);

    secondary.replication_lag_seconds = None;
    assert!(rpo_evidence(&s, &secondary).is_none());
}

#[test]
fn drill_record_feeds_the_dr_compliance_report() {
    let s = spec();
    let (status, _) = drive(
        &s,
        initial_status(),
        |_t| down("eu-west", 0),
        |t| healthy("us-east", t),
        0,
        1200,
    );
    let plan = plan_with(&s, status);
    let record = drill_compliance_record(&plan);
    assert_eq!(record["plan"], "horizon-global");
    assert_eq!(record["namespace"], "stellar");
    assert_eq!(record["drillId"], "2026-q1-full-region");
    assert_eq!(record["mode"], "live");
    assert_eq!(record["direction"], "failover");
    assert_eq!(record["rto"]["targetSeconds"], 900);
    assert_eq!(record["rto"]["measuredSeconds"], 600);
    assert_eq!(record["rto"]["met"], true);
    assert_eq!(record["rtoMet"], true);
    assert_eq!(record["steps"].as_array().unwrap().len(), 4);
    assert_eq!(record["rpo"]["measuredSeconds"], 5);

    // A drill-mode plan is labelled as such in the report.
    let mut drill = s.clone();
    drill.targets.drill = true;
    let plan = plan_with(&drill, initial_status());
    assert_eq!(drill_compliance_record(&plan)["mode"], "drill");
}

#[test]
fn summary_is_operator_readable() {
    let s = spec();
    let (status, _) = drive(
        &s,
        initial_status(),
        |_t| down("eu-west", 0),
        |t| healthy("us-east", t),
        0,
        1200,
    );
    let summary = render_summary(&s, &status);
    assert!(summary.contains("phase=Completed"), "{summary}");
    assert!(summary.contains("direction=Failover"), "{summary}");
    assert!(summary.contains("eu-west=0%"), "{summary}");
    assert!(summary.contains("us-east=100%"), "{summary}");
    assert!(summary.contains("rto=600s/900s MET"), "{summary}");
    assert!(summary.contains("steps=4"), "{summary}");

    // The status carries the same summary, so `kubectl get` shows it, and the
    // conditions make the outcome machine-readable.
    assert!(status.summary.contains("phase=Completed"));
    assert!(!status.conditions.is_empty());
    assert!(status.conditions.iter().any(|c| c.type_ == "Ready"));
}

#[test]
fn conditions_flag_an_invalid_plan_and_a_missed_rpo() {
    let mut s = spec();
    s.primary.name = String::new();
    let (status, reason) = reconcile_cycle(
        &s,
        initial_status(),
        &down("eu-west", 0),
        &healthy("us-east", 0),
        7,
        at(0),
    )
    .unwrap_err();
    assert!(reason.contains("spec.primary"), "{reason}");
    assert_eq!(status.phase, TrafficShiftPhase::Failed);
    assert!(status.observed_generation == Some(7));
    assert!(status
        .conditions
        .iter()
        .any(|c| c.type_ == "Degraded" && c.reason == "InvalidPlan"));

    // A secondary whose lag exceeds the declared RPO raises a Degraded
    // condition while the shift itself is fine.
    let s = spec();
    let mut lagging = healthy("us-east", 0);
    lagging.replication_lag_seconds = Some(300);
    let status = reconcile_cycle(
        &s,
        initial_status(),
        &down("eu-west", 0),
        &lagging,
        1,
        at(0),
    )
    .unwrap();
    assert!(status
        .conditions
        .iter()
        .any(|c| c.reason == "RpoTargetMissed"));
}

#[test]
fn status_records_raw_evidence_for_both_regions() {
    let s = spec();
    let status = reconcile_cycle(
        &s,
        initial_status(),
        &down("eu-west", 0),
        &healthy("us-east", 0),
        1,
        at(0),
    )
    .unwrap();
    assert_eq!(status.health.len(), 2);
    assert_eq!(status.health["eu-west"].consecutive_failures, 10);
    assert_eq!(status.health["us-east"].consecutive_successes, 10);
    assert!(status.last_gate.is_some());
    assert_eq!(status.last_gate.as_ref().unwrap().open, true);
    assert_eq!(status.last_evaluated_at, Some(at(0)));
}
