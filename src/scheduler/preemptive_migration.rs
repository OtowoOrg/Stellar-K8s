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
//! Preemptive migration driven by scheduled-node-group drain / interruption
//! signals (#1484).
//!
//! Planned spot interruption on a node group starts migration at least
//! [`MIN_PREEMPTIVE_LEAD`] before the expected event when the signal carries
//! a precise timestamp. Critical workloads are only moved to on-demand.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::crd::{CapacityClass, WorkloadTier};

use super::affinity::PodPlacement;
use super::capacity::{is_capacity_allowed, place_workloads, WorkloadRequest};
use super::optimizer::NodeResources;

/// Minimum lead time before a signaled interruption (2 minutes).
pub const MIN_PREEMPTIVE_LEAD: Duration = Duration::from_secs(120);

/// Annotation written by scheduled-node-group / cluster-autoscaler drains.
pub const SCHEDULED_INTERRUPT_ANNOTATION: &str = "stellar.org/scheduled-interrupt-at";
/// Node-group identity used by scheduled-node-group draining.
pub const NODE_GROUP_LABEL: &str = "stellar.org/node-group";
/// Existing spot-drain annotation set by [`crate::controller::spot_drain`].
pub const SPOT_DRAIN_ANNOTATION: &str = "stellar.org/spot-drain";

/// Interruption / drain signal from a scheduled node group or spot metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptionSignal {
    pub node_name: String,
    pub node_group: Option<String>,
    /// Expected interruption instant when the provider publishes one.
    pub expected_at: Option<DateTime<Utc>>,
    /// True when the signal includes a precise timestamp (not just "now").
    pub precise: bool,
    pub source: InterruptionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InterruptionSource {
    ScheduledNodeGroup,
    SpotMetadata,
    Simulated,
}

/// A single pod that must leave the interrupting node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationMove {
    pub workload: String,
    pub from_node: String,
    pub to_node: String,
    pub tier: WorkloadTier,
    pub destination_class: CapacityClass,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub moves: Vec<MigrationMove>,
    pub skipped_critical_spot_destinations: usize,
    pub started_at: Option<DateTime<Utc>>,
    pub expected_interruption_at: Option<DateTime<Utc>>,
}

impl MigrationPlan {
    /// Lead time from migration start to expected interruption, if known.
    pub fn lead_time(&self) -> Option<Duration> {
        match (self.started_at, self.expected_interruption_at) {
            (Some(start), Some(expected)) if expected >= start => (expected - start).to_std().ok(),
            _ => None,
        }
    }

    pub fn meets_two_minute_lead(&self) -> bool {
        self.lead_time()
            .map(|d| d >= MIN_PREEMPTIVE_LEAD)
            .unwrap_or(false)
    }

    pub fn critical_moved_to_spot(&self) -> usize {
        self.moves
            .iter()
            .filter(|m| {
                m.tier == WorkloadTier::Critical && m.destination_class == CapacityClass::Spot
            })
            .count()
    }
}

/// Instant at which preemptive migration should begin for a precise signal.
pub fn migration_start_deadline(expected_at: DateTime<Utc>) -> DateTime<Utc> {
    expected_at - chrono::Duration::seconds(MIN_PREEMPTIVE_LEAD.as_secs() as i64)
}

/// Whether preemptive migration should start now.
///
/// When the signal is precise, we start as soon as `now` reaches
/// `expected_at - 2min`. Imprecise signals (immediate metadata notice)
/// start immediately.
pub fn should_begin_preemptive_migration(now: DateTime<Utc>, signal: &InterruptionSignal) -> bool {
    match (signal.precise, signal.expected_at) {
        (true, Some(expected)) => now >= migration_start_deadline(expected),
        (_, Some(expected)) => now >= expected || now + chrono::Duration::seconds(120) >= expected,
        _ => true,
    }
}

/// Plan migrations off the interrupting node without placing critical work on spot.
pub fn plan_preemptive_migration(
    signal: &InterruptionSignal,
    now: DateTime<Utc>,
    occupants: &[WorkloadRequest],
    current: &[PodPlacement],
    nodes: &[NodeResources],
) -> Option<MigrationPlan> {
    if !should_begin_preemptive_migration(now, signal) {
        return None;
    }

    let remaining: Vec<NodeResources> = nodes
        .iter()
        .filter(|n| n.name != signal.node_name)
        .cloned()
        .collect();

    let to_move: Vec<WorkloadRequest> = occupants
        .iter()
        .filter(|w| {
            current
                .iter()
                .any(|p| p.pod_name == w.name && p.node_name == signal.node_name)
        })
        .cloned()
        .collect();

    let existing_elsewhere: Vec<PodPlacement> = current
        .iter()
        .filter(|p| p.node_name != signal.node_name)
        .cloned()
        .collect();

    let report = place_workloads(&to_move, &remaining, &existing_elsewhere);
    let mut plan = MigrationPlan {
        started_at: Some(now),
        expected_interruption_at: signal.expected_at,
        ..Default::default()
    };

    for placement in report.placements {
        if !is_capacity_allowed(placement.tier, placement.capacity_class) {
            plan.skipped_critical_spot_destinations += 1;
            continue;
        }
        plan.moves.push(MigrationMove {
            workload: placement.workload,
            from_node: signal.node_name.clone(),
            to_node: placement.node_name,
            tier: placement.tier,
            destination_class: placement.capacity_class,
        });
    }
    Some(plan)
}

/// Parse a scheduled-interrupt annotation value (RFC3339).
pub fn parse_interrupt_at(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Build a signal from node labels/annotations used by drain controllers.
pub fn signal_from_node_metadata(
    node_name: &str,
    labels: &std::collections::HashMap<String, String>,
    annotations: &std::collections::HashMap<String, String>,
    now: DateTime<Utc>,
) -> Option<InterruptionSignal> {
    let node_group = labels.get(NODE_GROUP_LABEL).cloned();
    if let Some(ts) = annotations
        .get(SCHEDULED_INTERRUPT_ANNOTATION)
        .and_then(|v| parse_interrupt_at(v))
    {
        return Some(InterruptionSignal {
            node_name: node_name.to_string(),
            node_group,
            expected_at: Some(ts),
            precise: true,
            source: InterruptionSource::ScheduledNodeGroup,
        });
    }
    if annotations
        .get(SPOT_DRAIN_ANNOTATION)
        .is_some_and(|v| v == "true")
    {
        return Some(InterruptionSignal {
            node_name: node_name.to_string(),
            node_group,
            expected_at: Some(now + chrono::Duration::seconds(120)),
            precise: false,
            source: InterruptionSource::SpotMetadata,
        });
    }
    None
}

/// Best-effort disruption rate: moved pods / occupants that were best-effort.
pub fn best_effort_disruption_rate(occupants: &[WorkloadRequest], plan: &MigrationPlan) -> f64 {
    let eligible = occupants
        .iter()
        .filter(|w| w.tier == WorkloadTier::BestEffort)
        .count();
    if eligible == 0 {
        return 0.0;
    }
    let moved = plan
        .moves
        .iter()
        .filter(|m| m.tier == WorkloadTier::BestEffort)
        .count();
    moved as f64 / eligible as f64
}

/// Scale-oriented simulation: many best-effort pods, one interrupting spot node.
pub fn simulate_scale_interruption(
    node_count: usize,
    best_effort: usize,
    critical: usize,
) -> (super::capacity::PlacementReport, MigrationPlan) {
    use super::capacity::{
        best_effort_workload, critical_workload, hourly_cost_for_class, labeled_node,
        DEFAULT_ON_DEMAND_HOURLY_USD,
    };

    let mut nodes = Vec::new();
    for i in 0..node_count {
        let class = if i % 3 == 0 {
            CapacityClass::OnDemand
        } else {
            CapacityClass::Spot
        };
        let zone = format!("z{}", i % 3);
        nodes.push(labeled_node(
            &format!("n{i}"),
            class,
            &zone,
            16_000,
            32_768,
            hourly_cost_for_class(class, DEFAULT_ON_DEMAND_HOURLY_USD),
        ));
    }

    let mut workloads = Vec::new();
    for i in 0..critical {
        workloads.push(critical_workload(&format!("c{i}")));
    }
    for i in 0..best_effort {
        workloads.push(best_effort_workload(&format!("b{i}")));
    }

    let report = place_workloads(&workloads, &nodes, &[]);
    let interrupting = report
        .placements
        .iter()
        .find(|p| p.capacity_class == CapacityClass::Spot)
        .map(|p| p.node_name.clone())
        .unwrap_or_else(|| "n1".to_string());

    let expected = Utc::now() + chrono::Duration::minutes(5);
    let now = expected - chrono::Duration::minutes(2);
    let signal = InterruptionSignal {
        node_name: interrupting,
        node_group: Some("spot-ng-1".to_string()),
        expected_at: Some(expected),
        precise: true,
        source: InterruptionSource::Simulated,
    };
    let current: Vec<PodPlacement> = report
        .placements
        .iter()
        .map(|p| PodPlacement {
            pod_name: p.workload.clone(),
            node_name: p.node_name.clone(),
            labels: Default::default(),
        })
        .collect();
    let plan = plan_preemptive_migration(&signal, now, &workloads, &current, &nodes)
        .expect("precise 2-minute signal must start migration");
    (report, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::capacity::{
        best_effort_workload, critical_workload, defined_test_cluster, place_workloads,
    };

    fn placed_state(
        workloads: &[WorkloadRequest],
    ) -> (
        Vec<NodeResources>,
        Vec<WorkloadRequest>,
        Vec<PodPlacement>,
        super::super::capacity::PlacementReport,
    ) {
        let nodes = defined_test_cluster();
        let report = place_workloads(workloads, &nodes, &[]);
        let current = report
            .placements
            .iter()
            .map(|p| PodPlacement {
                pod_name: p.workload.clone(),
                node_name: p.node_name.clone(),
                labels: Default::default(),
            })
            .collect();
        (nodes, workloads.to_vec(), current, report)
    }

    #[test]
    fn precise_signal_starts_two_minutes_before() {
        let expected = Utc::now() + chrono::Duration::minutes(10);
        let signal = InterruptionSignal {
            node_name: "spot-a".into(),
            node_group: Some("g".into()),
            expected_at: Some(expected),
            precise: true,
            source: InterruptionSource::ScheduledNodeGroup,
        };
        let too_early = expected - chrono::Duration::minutes(5);
        assert!(!should_begin_preemptive_migration(too_early, &signal));
        let on_time = expected - chrono::Duration::minutes(2);
        assert!(should_begin_preemptive_migration(on_time, &signal));
        assert_eq!(migration_start_deadline(expected), on_time);
    }

    #[test]
    fn interruption_signal_triggers_migration() {
        let workloads: Vec<_> = (0..6)
            .map(|i| best_effort_workload(&format!("be-{i}")))
            .collect();
        let (nodes, wls, current, report) = placed_state(&workloads);
        let spot = report
            .placements
            .iter()
            .find(|p| p.capacity_class == CapacityClass::Spot)
            .expect("best-effort should land on spot");
        let expected = Utc::now() + chrono::Duration::minutes(3);
        let now = expected - chrono::Duration::minutes(2);
        let signal = InterruptionSignal {
            node_name: spot.node_name.clone(),
            node_group: Some("spot-ng".into()),
            expected_at: Some(expected),
            precise: true,
            source: InterruptionSource::ScheduledNodeGroup,
        };
        let plan =
            plan_preemptive_migration(&signal, now, &wls, &current, &nodes).expect("should start");
        assert!(!plan.moves.is_empty());
        assert!(plan.meets_two_minute_lead());
        assert_eq!(plan.lead_time().map(|d| d.as_secs()), Some(120));
        assert_eq!(plan.critical_moved_to_spot(), 0);
        let occupants_on_node: Vec<_> = wls
            .iter()
            .filter(|w| {
                current
                    .iter()
                    .any(|p| p.pod_name == w.name && p.node_name == signal.node_name)
            })
            .cloned()
            .collect();
        let disruption = best_effort_disruption_rate(&occupants_on_node, &plan);
        assert!(
            disruption >= 1.0 - f64::EPSILON,
            "preemptive migration should relocate interrupting-node best-effort pods, rate={disruption}"
        );
    }

    #[test]
    fn critical_protected_during_migration() {
        let mut workloads = vec![critical_workload("validator-1")];
        workloads.extend((0..4).map(|i| best_effort_workload(&format!("be-{i}"))));
        let (nodes, wls, current, report) = placed_state(&workloads);
        // Force a critical occupant by targeting an on-demand node that also
        // holds nothing we would send to spot.
        let od = report
            .placements
            .iter()
            .find(|p| p.tier == WorkloadTier::Critical)
            .unwrap();
        let expected = Utc::now() + chrono::Duration::minutes(4);
        let now = expected - chrono::Duration::minutes(2);
        let signal = InterruptionSignal {
            node_name: od.node_name.clone(),
            node_group: Some("od-ng".into()),
            expected_at: Some(expected),
            precise: true,
            source: InterruptionSource::Simulated,
        };
        let plan =
            plan_preemptive_migration(&signal, now, &wls, &current, &nodes).expect("should start");
        assert_eq!(plan.critical_moved_to_spot(), 0);
        for mv in &plan.moves {
            if mv.tier == WorkloadTier::Critical {
                assert_eq!(mv.destination_class, CapacityClass::OnDemand);
                assert!(
                    capacity_class_from_labels(
                        &nodes.iter().find(|n| n.name == mv.to_node).unwrap().labels
                    ) == CapacityClass::OnDemand
                );
            }
        }
    }

    #[test]
    fn scale_interruption_simulation() {
        let (report, plan) = simulate_scale_interruption(12, 80, 12);
        assert_eq!(report.critical_on_spot(), 0);
        assert!(
            report.meets_spot_target(),
            "spot ratio {}",
            report.best_effort_spot_ratio()
        );
        assert!(plan.meets_two_minute_lead());
        assert_eq!(plan.critical_moved_to_spot(), 0);
        assert!(!plan.moves.is_empty());
    }

    #[test]
    fn scheduled_node_group_annotation_parses() {
        let expected = Utc::now() + chrono::Duration::minutes(8);
        let mut annotations = std::collections::HashMap::new();
        annotations.insert(
            SCHEDULED_INTERRUPT_ANNOTATION.to_string(),
            expected.to_rfc3339(),
        );
        let mut labels = std::collections::HashMap::new();
        labels.insert(NODE_GROUP_LABEL.to_string(), "spot-ng-1".to_string());
        let signal = signal_from_node_metadata("n1", &labels, &annotations, Utc::now()).unwrap();
        assert_eq!(signal.source, InterruptionSource::ScheduledNodeGroup);
        assert!(signal.precise);
        assert_eq!(signal.node_group.as_deref(), Some("spot-ng-1"));
    }
}
