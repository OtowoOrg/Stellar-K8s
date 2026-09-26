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
//! Degradation state machine.
//!
//! Pure and clock-injected: it consumes probe outcomes and produces the
//! level, transitions and incident reports without doing any I/O.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use chrono::{DateTime, Utc};

use crate::crd::control_plane_health::{
    ComponentState, ComponentStatus, ControlPlaneComponent, ControlPlaneHealthSpec,
    ControlPlaneHealthStatus, DegradationLevel, IncidentReport, LevelTransition,
};

/// Result of probing one component once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    Healthy,
    Unhealthy(String),
    /// The probe could not tell (e.g. its signal source is itself down).
    Inconclusive(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackerConfig {
    pub failure_threshold: u32,
    pub recovery_threshold: u32,
    pub incident_history: usize,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self::from(&ControlPlaneHealthSpec::default())
    }
}

impl From<&ControlPlaneHealthSpec> for TrackerConfig {
    fn from(spec: &ControlPlaneHealthSpec) -> Self {
        Self {
            failure_threshold: spec.failure_threshold.max(1),
            recovery_threshold: spec.recovery_threshold.max(1),
            incident_history: spec.incident_history as usize,
        }
    }
}

/// What changed in one probe round.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoundOutcome {
    /// `(component, from, to)` for every component whose state changed.
    pub component_changes: Vec<(ControlPlaneComponent, ComponentState, ComponentState)>,
    pub transition: Option<LevelTransition>,
    /// Report of the incident that ended this round, if any.
    pub closed_incident: Option<IncidentReport>,
}

#[derive(Debug, Clone, Default)]
struct ComponentTracker {
    state: ComponentState,
    consecutive_failures: u32,
    consecutive_successes: u32,
    last_probe: Option<DateTime<Utc>>,
    last_transition: Option<DateTime<Utc>>,
    message: Option<String>,
}

#[derive(Debug, Clone)]
struct ActiveIncident {
    report: IncidentReport,
    started: DateTime<Utc>,
    suppressed_baseline: u64,
    components: BTreeSet<ControlPlaneComponent>,
}

#[derive(Debug, Clone)]
pub struct DegradationTracker {
    config: TrackerConfig,
    components: BTreeMap<ControlPlaneComponent, ComponentTracker>,
    level: DegradationLevel,
    level_since: DateTime<Utc>,
    active: Option<ActiveIncident>,
    incidents: VecDeque<IncidentReport>,
}

impl DegradationTracker {
    pub fn new(config: TrackerConfig, now: DateTime<Utc>) -> Self {
        Self {
            config,
            components: ControlPlaneComponent::ALL
                .into_iter()
                .map(|c| (c, ComponentTracker::default()))
                .collect(),
            level: DegradationLevel::Normal,
            level_since: now,
            active: None,
            incidents: VecDeque::new(),
        }
    }

    pub fn set_config(&mut self, config: TrackerConfig) {
        self.config = config;
        self.incidents.truncate(config.incident_history);
    }

    pub fn level(&self) -> DegradationLevel {
        self.level
    }

    pub fn component_state(&self, component: ControlPlaneComponent) -> ComponentState {
        self.components[&component].state
    }

    /// Applies one round of probe results.
    ///
    /// `suppressed_total` is the gate's running count of withheld actions,
    /// used to attribute suppressions to the active incident.
    pub fn record_round(
        &mut self,
        results: &[(ControlPlaneComponent, ProbeOutcome)],
        suppressed_total: u64,
        now: DateTime<Utc>,
    ) -> RoundOutcome {
        let mut outcome = RoundOutcome::default();

        // Dependencies first so dependents see this round's state.
        let mut ordered: Vec<_> = results.iter().collect();
        ordered.sort_by_key(|(c, _)| c.depends_on().len());

        for (component, result) in ordered {
            let result = self.attribute(*component, result);
            let (failure_threshold, recovery_threshold) = (
                self.config.failure_threshold,
                self.config.recovery_threshold,
            );
            let tracker = self
                .components
                .get_mut(component)
                .expect("all components tracked");
            let before = tracker.state;
            tracker.last_probe = Some(now);

            match result {
                ProbeOutcome::Healthy => {
                    tracker.consecutive_failures = 0;
                    tracker.consecutive_successes += 1;
                    tracker.message = None;
                    let recovered = match tracker.state {
                        ComponentState::Unhealthy => {
                            tracker.consecutive_successes >= recovery_threshold
                        }
                        ComponentState::Unknown => true,
                        ComponentState::Healthy => false,
                    };
                    if recovered {
                        tracker.state = ComponentState::Healthy;
                    }
                }
                ProbeOutcome::Unhealthy(msg) => {
                    tracker.consecutive_successes = 0;
                    tracker.consecutive_failures += 1;
                    tracker.message = Some(msg);
                    if tracker.consecutive_failures >= failure_threshold {
                        tracker.state = ComponentState::Unhealthy;
                    }
                }
                ProbeOutcome::Inconclusive(msg) => {
                    tracker.message = Some(msg);
                }
            }

            if tracker.state != before {
                tracker.last_transition = Some(now);
                outcome
                    .component_changes
                    .push((*component, before, tracker.state));
            }
        }

        let unhealthy: Vec<ControlPlaneComponent> = self
            .components
            .iter()
            .filter(|(_, t)| t.state == ComponentState::Unhealthy)
            .map(|(c, _)| *c)
            .collect();
        let target = unhealthy
            .iter()
            .map(|c| c.implied_level())
            .max()
            .unwrap_or(DegradationLevel::Normal);

        if target != self.level {
            let reason = if unhealthy.is_empty() {
                "all control-plane components healthy".to_string()
            } else {
                format!("unhealthy components: {}", join(&unhealthy))
            };
            let transition = LevelTransition {
                from: self.level,
                to: target,
                at: now.to_rfc3339(),
                reason,
            };
            self.level = target;
            self.level_since = now;

            let incident = self.active.get_or_insert_with(|| ActiveIncident {
                report: IncidentReport {
                    id: format!("cph-{}", now.format("%Y%m%dT%H%M%SZ")),
                    started_at: now.to_rfc3339(),
                    ended_at: None,
                    duration_seconds: None,
                    peak_level: target,
                    components: Vec::new(),
                    transitions: Vec::new(),
                    suppressed_actions: 0,
                },
                started: now,
                suppressed_baseline: suppressed_total,
                components: BTreeSet::new(),
            });
            incident.report.transitions.push(transition.clone());
            outcome.transition = Some(transition);
        }

        if let Some(incident) = self.active.as_mut() {
            incident.components.extend(unhealthy.iter().copied());
            incident.report.components = incident.components.iter().copied().collect();
            incident.report.peak_level = incident.report.peak_level.max(self.level);
            incident.report.suppressed_actions =
                suppressed_total.saturating_sub(incident.suppressed_baseline);
        }

        if self.level == DegradationLevel::Normal {
            if let Some(mut incident) = self.active.take() {
                incident.report.ended_at = Some(now.to_rfc3339());
                incident.report.duration_seconds =
                    Some((now - incident.started).num_seconds().max(0) as u64);
                self.incidents.push_front(incident.report.clone());
                self.incidents.truncate(self.config.incident_history);
                outcome.closed_incident = Some(incident.report);
            }
        }

        outcome
    }

    /// Downgrades a failure to inconclusive while a dependency is failing,
    /// so the outage is attributed to the root component only.
    fn attribute(&self, component: ControlPlaneComponent, result: &ProbeOutcome) -> ProbeOutcome {
        if let ProbeOutcome::Unhealthy(msg) = result {
            let failing: Vec<ControlPlaneComponent> = component
                .depends_on()
                .iter()
                .copied()
                .filter(|dep| {
                    let t = &self.components[dep];
                    t.state == ComponentState::Unhealthy || t.consecutive_failures > 0
                })
                .collect();
            if !failing.is_empty() {
                return ProbeOutcome::Inconclusive(format!(
                    "{msg} (not attributed: dependency {} failing)",
                    join(&failing)
                ));
            }
        }
        result.clone()
    }

    /// Full status snapshot for the `ControlPlaneHealth` CR.
    pub fn status(&self, observed_generation: Option<i64>) -> ControlPlaneHealthStatus {
        ControlPlaneHealthStatus {
            level: self.level,
            level_since: Some(self.level_since.to_rfc3339()),
            permitted: self.level.permitted(),
            components: self
                .components
                .iter()
                .map(|(c, t)| ComponentStatus {
                    component: *c,
                    state: t.state,
                    implied_level: c.implied_level(),
                    consecutive_failures: t.consecutive_failures,
                    consecutive_successes: t.consecutive_successes,
                    last_probe_time: t.last_probe.map(|t| t.to_rfc3339()),
                    last_transition_time: t.last_transition.map(|t| t.to_rfc3339()),
                    message: t.message.clone(),
                })
                .collect(),
            active_incident: self.active.as_ref().map(|i| i.report.clone()),
            incidents: self.incidents.iter().cloned().collect(),
            observed_generation,
        }
    }
}

fn join(components: &[ControlPlaneComponent]) -> String {
    components
        .iter()
        .map(|c| format!("{c:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::degradation::{DegradationGate, OperatorAction};
    use chrono::{Duration, TimeZone};
    use ControlPlaneComponent::*;

    const ROUND: i64 = 10; // seconds, the default probe interval

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap()
    }

    fn all_healthy() -> Vec<(ControlPlaneComponent, ProbeOutcome)> {
        ControlPlaneComponent::ALL
            .into_iter()
            .map(|c| (c, ProbeOutcome::Healthy))
            .collect()
    }

    /// Probe results observed while `killed` is down, including the knock-on
    /// effects real probes would see.
    fn outage(killed: ControlPlaneComponent) -> Vec<(ControlPlaneComponent, ProbeOutcome)> {
        all_healthy()
            .into_iter()
            .map(|(c, r)| {
                let down = c == killed
                    // The scheduler lease goes stale while etcd is down.
                    || (killed == Etcd && c == Scheduler);
                // The webhook probe falls back to its cached address when DNS
                // is down, so it stays healthy.
                if down {
                    (c, ProbeOutcome::Unhealthy(format!("{c:?} down")))
                } else {
                    (c, r)
                }
            })
            .collect()
    }

    struct Run {
        levels: Vec<(DateTime<Utc>, DegradationLevel)>,
        transitions: Vec<LevelTransition>,
        closed: Vec<IncidentReport>,
        disruptive_allowed_while_degraded: usize,
        tracker: DegradationTracker,
    }

    /// Chaos-matrix scenario: 5 min healthy, `killed` isolated for 15 min,
    /// then 5 min healthy. A simulated reconciler attempts a disruptive action
    /// every round, as it would with traffic held flat.
    fn chaos(killed: ControlPlaneComponent) -> Run {
        let gate = DegradationGate::default();
        let mut tracker = DegradationTracker::new(TrackerConfig::default(), t0());
        let rounds = |mins: i64| mins * 60 / ROUND;
        let (healthy, down) = (rounds(5), rounds(15));
        let mut run = Run {
            levels: Vec::new(),
            transitions: Vec::new(),
            closed: Vec::new(),
            disruptive_allowed_while_degraded: 0,
            tracker: tracker.clone(),
        };
        for i in 0..(healthy + down + healthy) {
            let now = t0() + Duration::seconds(i * ROUND);
            let results = if (healthy..healthy + down).contains(&i) {
                outage(killed)
            } else {
                all_healthy()
            };
            let out = tracker.record_round(&results, gate.suppressed_total(), now);
            gate.set_level(tracker.level());
            if gate.check(OperatorAction::Disruptive).is_ok()
                && tracker.level() != DegradationLevel::Normal
            {
                run.disruptive_allowed_while_degraded += 1;
            }
            run.levels.push((now, tracker.level()));
            run.transitions.extend(out.transition);
            run.closed.extend(out.closed_incident);
        }
        run.tracker = tracker;
        run
    }

    #[test]
    fn chaos_matrix_each_component_isolated_for_15_minutes() {
        for killed in ControlPlaneComponent::ALL {
            let run = chaos(killed);
            let expected = killed.implied_level();

            // Detected within failure_threshold rounds; transitions logged.
            assert_eq!(
                run.transitions.len(),
                2,
                "{killed:?}: {:?}",
                run.transitions
            );
            assert_eq!(run.transitions[0].from, DegradationLevel::Normal);
            assert_eq!(run.transitions[0].to, expected, "{killed:?}");
            let outage_start = t0() + Duration::minutes(5);
            let detected: DateTime<Utc> = run.transitions[0].at.parse().unwrap();
            assert!(detected - outage_start <= Duration::seconds(3 * ROUND));

            // Held for the whole outage; only the killed component is blamed.
            let degraded: Vec<_> = run.levels.iter().filter(|(_, l)| *l == expected).collect();
            assert!(degraded.len() as i64 >= 15 * 60 / ROUND - 3, "{killed:?}");
            assert_eq!(run.disruptive_allowed_while_degraded, 0, "{killed:?}");

            // Automatic recovery with a post-incident mode report.
            assert_eq!(run.transitions[1].to, DegradationLevel::Normal);
            assert_eq!(run.levels.last().unwrap().1, DegradationLevel::Normal);
            assert_eq!(run.closed.len(), 1);
            let report = &run.closed[0];
            assert_eq!(report.components, vec![killed], "{killed:?} blamed wrongly");
            assert_eq!(report.peak_level, expected);
            assert_eq!(report.transitions.len(), 2);
            let secs = report.duration_seconds.unwrap();
            assert!((15 * 60..=16 * 60).contains(&secs), "{killed:?}: {secs}s");
            assert!(report.suppressed_actions > 0);

            let status = run.tracker.status(Some(1));
            assert!(status.active_incident.is_none());
            assert_eq!(status.incidents.len(), 1);
            assert!(status
                .components
                .iter()
                .all(|c| c.state == ComponentState::Healthy));
        }
    }

    #[test]
    fn etcd_outage_freezes_writes_and_does_not_blame_scheduler() {
        let run = chaos(Etcd);
        assert!(run
            .levels
            .iter()
            .any(|(_, l)| !l.permitted().writes && *l == DegradationLevel::Frozen));
        assert_eq!(run.closed[0].components, vec![Etcd]);
    }

    #[test]
    fn single_failures_and_flapping_do_not_degrade() {
        let mut tracker = DegradationTracker::new(TrackerConfig::default(), t0());
        for i in 0..20 {
            let now = t0() + Duration::seconds(i * ROUND);
            let results = if i % 2 == 0 {
                outage(Dns)
            } else {
                all_healthy()
            };
            tracker.record_round(&results, 0, now);
            assert_eq!(tracker.level(), DegradationLevel::Normal);
        }
    }

    #[test]
    fn level_is_the_most_severe_unhealthy_component() {
        let mut tracker = DegradationTracker::new(TrackerConfig::default(), t0());
        let mut results = outage(Webhook);
        results[1] = (Dns, ProbeOutcome::Unhealthy("dns".into()));
        for i in 0..3 {
            tracker.record_round(&results, 0, t0() + Duration::seconds(i * ROUND));
        }
        assert_eq!(tracker.level(), DegradationLevel::Degraded);

        // DNS recovers first: the level steps down to Reduced, still one incident.
        let results = outage(Webhook);
        let mut last = RoundOutcome::default();
        for i in 3..6 {
            last = tracker.record_round(&results, 0, t0() + Duration::seconds(i * ROUND));
        }
        assert_eq!(tracker.level(), DegradationLevel::Reduced);
        assert_eq!(last.transition.unwrap().from, DegradationLevel::Degraded);
        let active = tracker.status(None).active_incident.unwrap();
        assert_eq!(active.peak_level, DegradationLevel::Degraded);
        assert_eq!(active.components, vec![Dns, Webhook]);
    }

    #[test]
    fn inconclusive_probes_leave_state_unchanged() {
        let mut tracker = DegradationTracker::new(TrackerConfig::default(), t0());
        let results = [(Scheduler, ProbeOutcome::Inconclusive("lease hidden".into()))];
        for i in 0..10 {
            tracker.record_round(&results, 0, t0() + Duration::seconds(i * ROUND));
        }
        assert_eq!(tracker.component_state(Scheduler), ComponentState::Unknown);
        assert_eq!(tracker.level(), DegradationLevel::Normal);
    }

    #[test]
    fn incident_history_is_bounded() {
        let config = TrackerConfig {
            incident_history: 2,
            ..TrackerConfig::default()
        };
        let mut tracker = DegradationTracker::new(config, t0());
        let mut i = 0;
        for _ in 0..4 {
            for results in [outage(Webhook), all_healthy()] {
                for _ in 0..3 {
                    tracker.record_round(&results, 0, t0() + Duration::seconds(i * ROUND));
                    i += 1;
                }
            }
        }
        assert_eq!(tracker.status(None).incidents.len(), 2);
    }
}
