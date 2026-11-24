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
//! Watches scheduled-node-group drain annotations and spot-drain signals,
//! then plans preemptive migration through the cost-aware placement engine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{
    api::{Api, EvictParams, ListParams},
    runtime::events::Reporter,
    Client, ResourceExt,
};
use tracing::{info, warn};

use crate::crd::{CapacityClass, WorkloadTier};
use crate::error::{Error, Result};
use crate::scheduler::affinity::PodPlacement;
use crate::scheduler::capacity::{
    capacity_class_from_node, classify_pod, hourly_cost_for_class, labeled_node,
    realized_hourly_savings, WorkloadRequest, DEFAULT_ON_DEMAND_HOURLY_USD,
};
use crate::scheduler::optimizer::NodeResources;
use crate::scheduler::preemptive_migration::{
    plan_preemptive_migration, signal_from_node_metadata, InterruptionSignal,
};
use crate::scheduler::savings::SavingsAggregator;

/// Poll interval for scheduled-node-group / spot interruption annotations.
const POLL_INTERVAL: Duration = Duration::from_secs(15);

pub struct PreemptiveSpotMigrator {
    client: Client,
    _reporter: Reporter,
    savings: Mutex<SavingsAggregator>,
}

impl PreemptiveSpotMigrator {
    pub fn new(client: Client, reporter: Reporter) -> Self {
        Self {
            client,
            _reporter: reporter,
            savings: Mutex::new(SavingsAggregator::new()),
        }
    }

    pub fn savings_snapshot(&self) -> SavingsAggregator {
        self.savings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        info!("Preemptive spot migrator watching scheduled-node-group drain signals");
        loop {
            if let Err(e) = self.reconcile_once().await {
                warn!(error = %e, "preemptive migration reconcile failed");
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn reconcile_once(&self) -> Result<()> {
        let nodes_api: Api<Node> = Api::all(self.client.clone());
        let pods_api: Api<Pod> = Api::all(self.client.clone());
        let node_list = nodes_api
            .list(&ListParams::default())
            .await
            .map_err(Error::KubeError)?;
        let pod_list = pods_api
            .list(&ListParams::default())
            .await
            .map_err(Error::KubeError)?;

        let now = Utc::now();
        let resources = nodes_to_resources(&node_list.items);
        let (workloads, placements) = pods_to_workloads(&pod_list.items);

        for node in &node_list.items {
            let labels = node
                .metadata
                .labels
                .as_ref()
                .map(|l| l.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default();
            let annotations = node
                .metadata
                .annotations
                .as_ref()
                .map(|a| a.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default();
            let Some(signal) =
                signal_from_node_metadata(&node.name_any(), &labels, &annotations, now)
            else {
                continue;
            };
            self.apply_plan(&signal, now, &workloads, &placements, &resources)
                .await?;
        }
        self.observe_live_savings(&node_list.items, &pod_list.items, now);
        Ok(())
    }

    fn observe_live_savings(&self, nodes: &[Node], pods: &[Pod], now: chrono::DateTime<Utc>) {
        let mut realized = 0.0;
        let mut be_spot = 0u64;
        let mut be_total = 0u64;
        let mut critical_on_spot = 0u64;
        for pod in pods {
            let Some(node_name) = pod.spec.as_ref().and_then(|s| s.node_name.as_deref()) else {
                continue;
            };
            let Some(node) = nodes.iter().find(|n| n.name_any() == node_name) else {
                continue;
            };
            let class = capacity_class_from_node(node);
            let tier = classify_pod(pod);
            match tier {
                WorkloadTier::BestEffort => {
                    be_total += 1;
                    if class == CapacityClass::Spot {
                        be_spot += 1;
                        realized += realized_hourly_savings(class, DEFAULT_ON_DEMAND_HOURLY_USD);
                    }
                }
                WorkloadTier::Critical => {
                    if class == CapacityClass::Spot {
                        critical_on_spot += 1;
                    }
                }
            }
        }
        let ratio = if be_total == 0 {
            0.0
        } else {
            be_spot as f64 / be_total as f64
        };
        {
            let mut agg = self.savings.lock().unwrap_or_else(|e| e.into_inner());
            agg.set_hourly_run_rate(now, realized, be_spot, be_total);
        }
        #[cfg(feature = "metrics")]
        crate::controller::metrics::set_placement_cost_metrics(realized, ratio, critical_on_spot);
        let _ = (ratio, critical_on_spot);
    }

    async fn apply_plan(
        &self,
        signal: &InterruptionSignal,
        now: chrono::DateTime<Utc>,
        workloads: &[WorkloadRequest],
        placements: &[PodPlacement],
        resources: &[NodeResources],
    ) -> Result<()> {
        let Some(plan) = plan_preemptive_migration(signal, now, workloads, placements, resources)
        else {
            return Ok(());
        };
        info!(
            node = %signal.node_name,
            moves = plan.moves.len(),
            lead_secs = ?plan.lead_time().map(|d| d.as_secs()),
            "Starting preemptive migration ahead of scheduled interruption"
        );
        for mv in &plan.moves {
            if mv.tier == WorkloadTier::Critical && mv.destination_class == CapacityClass::Spot {
                warn!(workload = %mv.workload, "refusing to migrate critical workload onto spot");
                continue;
            }
            evict_pod(&self.client, &mv.workload).await;
        }
        Ok(())
    }
}

async fn evict_pod(client: &Client, name: &str) {
    let pods: Api<Pod> = Api::all(client.clone());
    let Ok(list) = pods.list(&ListParams::default()).await else {
        return;
    };
    if let Some(pod) = list.items.into_iter().find(|p| p.name_any() == name) {
        let ns = pod.namespace().unwrap_or_else(|| "default".to_string());
        let api: Api<Pod> = Api::namespaced(client.clone(), &ns);
        if let Err(e) = api.evict(&pod.name_any(), &EvictParams::default()).await {
            warn!(pod = %name, error = %e, "preemptive eviction failed");
        }
    }
}

fn nodes_to_resources(nodes: &[Node]) -> Vec<NodeResources> {
    nodes
        .iter()
        .map(|n| {
            let class = capacity_class_from_node(n);
            let zone = n
                .metadata
                .labels
                .as_ref()
                .and_then(|l| l.get("topology.kubernetes.io/zone"))
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            let mut res = labeled_node(
                &n.name_any(),
                class,
                &zone,
                16_000,
                32_768,
                hourly_cost_for_class(class, DEFAULT_ON_DEMAND_HOURLY_USD),
            );
            if let Some(labels) = n.metadata.labels.as_ref() {
                res.labels = labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            }
            res
        })
        .collect()
}

fn pods_to_workloads(pods: &[Pod]) -> (Vec<WorkloadRequest>, Vec<PodPlacement>) {
    let mut workloads = Vec::new();
    let mut placements = Vec::new();
    for pod in pods {
        let Some(node_name) = pod.spec.as_ref().and_then(|s| s.node_name.clone()) else {
            continue;
        };
        let name = pod.name_any();
        let labels: HashMap<String, String> = pod
            .metadata
            .labels
            .as_ref()
            .map(|l| l.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        workloads.push(WorkloadRequest {
            name: name.clone(),
            tier: classify_pod(pod),
            cpu_milli: 250,
            memory_mb: 256,
            labels: labels.clone(),
            affinity: Vec::new(),
            anti_affinity_selector: None,
            topology_key: Some("topology.kubernetes.io/zone".to_string()),
            topology_max_skew: 1,
        });
        placements.push(PodPlacement {
            pod_name: name,
            node_name,
            labels,
        });
    }
    (workloads, placements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::capacity::{best_effort_workload, defined_test_cluster, place_workloads};
    use crate::scheduler::preemptive_migration::{
        should_begin_preemptive_migration, InterruptionSource,
    };

    #[test]
    fn scheduled_group_signal_drives_eviction_plan() {
        let nodes = defined_test_cluster();
        let workloads: Vec<_> = (0..8)
            .map(|i| best_effort_workload(&format!("be-{i}")))
            .collect();
        let report = place_workloads(&workloads, &nodes, &[]);
        let current: Vec<PodPlacement> = report
            .placements
            .iter()
            .map(|p| PodPlacement {
                pod_name: p.workload.clone(),
                node_name: p.node_name.clone(),
                labels: Default::default(),
            })
            .collect();
        let spot = report
            .placements
            .iter()
            .find(|p| p.capacity_class == CapacityClass::Spot)
            .unwrap();
        let expected = Utc::now() + chrono::Duration::minutes(5);
        let now = expected - chrono::Duration::minutes(2);
        let signal = InterruptionSignal {
            node_name: spot.node_name.clone(),
            node_group: Some("spot-ng".into()),
            expected_at: Some(expected),
            precise: true,
            source: InterruptionSource::ScheduledNodeGroup,
        };
        assert!(should_begin_preemptive_migration(now, &signal));
        let plan = plan_preemptive_migration(&signal, now, &workloads, &current, &nodes).unwrap();
        assert!(!plan.moves.is_empty());
        assert_eq!(plan.critical_moved_to_spot(), 0);
    }
}
