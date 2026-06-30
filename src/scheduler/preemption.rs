//! Preemption and rescheduling logic.

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::optimizer::NodeResources;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodInfo {
    pub name: String,
    pub namespace: String,
    pub priority: i32,
    pub cpu_request_milli: u64,
    pub memory_request_mb: u64,
    pub node_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreedResources {
    pub cpu_milli: u64,
    pub memory_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreemptionCandidate {
    pub node_name: String,
    pub pods_to_evict: Vec<String>,
    pub freed_resources: FreedResources,
    pub preemption_score: f64,
}

pub struct PreemptionManager;

impl PreemptionManager {
    /// Find nodes where low-priority pods can be evicted to fit the incoming pod.
    pub fn find_preemption_candidates(
        nodes: &[NodeResources],
        existing_pods: &[PodInfo],
        req_cpu_milli: u64,
        req_memory_mb: u64,
        incoming_priority: i32,
    ) -> Vec<PreemptionCandidate> {
        let mut candidates = Vec::new();

        for node in nodes {
            // Only consider nodes where we can free enough resources
            let node_pods: Vec<&PodInfo> = existing_pods
                .iter()
                .filter(|p| p.node_name == node.name && p.priority < incoming_priority)
                .collect();

            if node_pods.is_empty() {
                continue;
            }

            // Sort by priority ascending (evict lowest priority first)
            let mut sorted_pods = node_pods.clone();
            sorted_pods.sort_by_key(|p| p.priority);

            let mut freed_cpu = node.free_cpu();
            let mut freed_mem = node.free_memory_mb();
            let mut to_evict = Vec::new();

            for pod in sorted_pods {
                if freed_cpu >= req_cpu_milli && freed_mem >= req_memory_mb {
                    break;
                }
                freed_cpu += pod.cpu_request_milli;
                freed_mem += pod.memory_request_mb;
                to_evict.push(pod.name.clone());
            }

            if freed_cpu >= req_cpu_milli && freed_mem >= req_memory_mb {
                let score = 1.0 - (to_evict.len() as f64 / node_pods.len() as f64);
                candidates.push(PreemptionCandidate {
                    node_name: node.name.clone(),
                    pods_to_evict: to_evict,
                    freed_resources: FreedResources {
                        cpu_milli: freed_cpu,
                        memory_mb: freed_mem,
                    },
                    preemption_score: score,
                });
            }
        }

        // Sort by score descending (prefer candidates that evict fewest pods)
        candidates.sort_by(|a, b| b.preemption_score.partial_cmp(&a.preemption_score).unwrap());
        candidates
    }

    /// Log preemption decision (actual eviction done via kube API in controller).
    pub fn log_preemption(candidate: &PreemptionCandidate) {
        if candidate.pods_to_evict.is_empty() {
            warn!(node = %candidate.node_name, "Preemption candidate has no pods to evict");
            return;
        }
        info!(
            node = %candidate.node_name,
            pods = ?candidate.pods_to_evict,
            freed_cpu = candidate.freed_resources.cpu_milli,
            freed_mem = candidate.freed_resources.memory_mb,
            "Preemption decision"
        );
    }
}
