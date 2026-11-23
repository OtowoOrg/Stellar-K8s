// Copyright 2026 Stellar-K8s Contributors
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
//! Cost-aware workload placement across Spot and On-Demand capacity.
//!
//! Bin-packs `BestEffort` workloads onto Spot nodes while guaranteeing `Critical`
//! workloads never land on Spot, and triggers preemptive migration >= 2 minutes
//! ahead of scheduled node-group drain events.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Workload availability tier.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadTier {
    Critical,
    Standard,
    BestEffort,
}

/// Node pool capacity class.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapacityClass {
    OnDemand,
    Spot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementNode {
    pub name: String,
    pub capacity_class: CapacityClass,
    pub available_cpu_millis: u64,
    pub hourly_cost_usd: f64,
    pub draining_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadRequest {
    pub name: String,
    pub tier: WorkloadTier,
    pub requested_cpu_millis: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlacementDecision {
    pub workload_name: String,
    pub node_name: String,
    pub capacity_class: CapacityClass,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreemptiveMigrationPlan {
    pub node_name: String,
    pub drain_at: DateTime<Utc>,
    pub migrate_before: DateTime<Utc>,
    pub lead_time_seconds: i64,
}

/// Cost-aware placement controller for Spot vs On-Demand capacity.
pub struct SpotPlacementController;

impl SpotPlacementController {
    /// Place a workload onto an eligible node.
    ///
    /// - `Critical` workloads NEVER land on `Spot` capacity.
    /// - `BestEffort` workloads prefer `Spot` capacity first (bin-packing).
    pub fn schedule_workload(
        workload: &WorkloadRequest,
        nodes: &mut [PlacementNode],
    ) -> Result<PlacementDecision, String> {
        let mut eligible: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                n.draining_at.is_none()
                    && n.available_cpu_millis >= workload.requested_cpu_millis
                    && !(workload.tier == WorkloadTier::Critical
                        && n.capacity_class == CapacityClass::Spot)
            })
            .map(|(idx, _)| idx)
            .collect();

        if eligible.is_empty() {
            return Err(format!("No eligible node for workload {}", workload.name));
        }

        eligible.sort_by(|&a, &b| {
            let na = &nodes[a];
            let nb = &nodes[b];
            match workload.tier {
                WorkloadTier::BestEffort => {
                    let spot_cmp = (nb.capacity_class == CapacityClass::Spot)
                        .cmp(&(na.capacity_class == CapacityClass::Spot));
                    spot_cmp.then_with(|| na.available_cpu_millis.cmp(&nb.available_cpu_millis))
                }
                WorkloadTier::Critical | WorkloadTier::Standard => {
                    na.hourly_cost_usd
                        .partial_cmp(&nb.hourly_cost_usd)
                        .unwrap_or(std::cmp::Ordering::Equal)
                }
            }
        });

        let chosen_idx = eligible[0];
        nodes[chosen_idx].available_cpu_millis -= workload.requested_cpu_millis;
        Ok(PlacementDecision {
            workload_name: workload.name.clone(),
            node_name: nodes[chosen_idx].name.clone(),
            capacity_class: nodes[chosen_idx].capacity_class,
        })
    }

    /// Plan preemptive migration at least 2 minutes before a scheduled drain.
    pub fn plan_preemptive_migration(
        node_name: &str,
        drain_at: DateTime<Utc>,
    ) -> PreemptiveMigrationPlan {
        let lead = Duration::minutes(2);
        PreemptiveMigrationPlan {
            node_name: node_name.to_string(),
            drain_at,
            migrate_before: drain_at - lead,
            lead_time_seconds: lead.num_seconds(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_never_lands_on_spot_and_best_effort_packs_spot() {
        let mut nodes = vec![
            PlacementNode {
                name: "spot-1".into(),
                capacity_class: CapacityClass::Spot,
                available_cpu_millis: 4000,
                hourly_cost_usd: 0.04,
                draining_at: None,
            },
            PlacementNode {
                name: "od-1".into(),
                capacity_class: CapacityClass::OnDemand,
                available_cpu_millis: 4000,
                hourly_cost_usd: 0.16,
                draining_at: None,
            },
        ];

        let crit = SpotPlacementController::schedule_workload(
            &WorkloadRequest {
                name: "validator-0".into(),
                tier: WorkloadTier::Critical,
                requested_cpu_millis: 1000,
            },
            &mut nodes,
        )
        .unwrap();
        assert_eq!(crit.capacity_class, CapacityClass::OnDemand);

        let be = SpotPlacementController::schedule_workload(
            &WorkloadRequest {
                name: "batch-indexer".into(),
                tier: WorkloadTier::BestEffort,
                requested_cpu_millis: 1000,
            },
            &mut nodes,
        )
        .unwrap();
        assert_eq!(be.capacity_class, CapacityClass::Spot);

        let mig = SpotPlacementController::plan_preemptive_migration(
            "spot-1",
            Utc::now() + Duration::minutes(5),
        );
        assert!(mig.lead_time_seconds >= 120);
    }
}
