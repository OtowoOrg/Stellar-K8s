//! Chaos Engineering Runner for Stellar-K8s
//!
//! Integrates with Chaos Mesh to run automated destruction tests against the cluster,
//! ensuring the reconciler handles extreme failures and recovers to healthy state.
//!
//! Supported chaos experiments:
//! - Pod Kill: Randomly terminate pods to test recovery
//! - Network Delay: Introduce latency to simulate network issues
//! - IO Stress: Stress disk I/O to test performance degradation
//! - CPU Stress: Stress CPU to test resource constraints
//! - Memory Pressure: Simulate memory pressure scenarios

use crate::error::{Error, Result};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

/// Chaos experiment types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChaosExperimentType {
    PodKill,
    NetworkDelay,
    IoStress,
    CpuStress,
    MemoryPressure,
}

impl std::fmt::Display for ChaosExperimentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PodKill => write!(f, "PodKill"),
            Self::NetworkDelay => write!(f, "NetworkDelay"),
            Self::IoStress => write!(f, "IoStress"),
            Self::CpuStress => write!(f, "CpuStress"),
            Self::MemoryPressure => write!(f, "MemoryPressure"),
        }
    }
}

/// Configuration for a chaos experiment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosExperimentConfig {
    pub experiment_type: ChaosExperimentType,
    pub namespace: String,
    pub target_label_selector: String,
    pub duration_secs: u64,
    pub delay_ms: Option<u32>,    // For NetworkDelay
    pub jitter_ms: Option<u32>,   // For NetworkDelay
    pub io_workers: Option<u32>,  // For IoStress
    pub cpu_workers: Option<u32>, // For CpuStress
    pub memory_mb: Option<u32>,   // For MemoryPressure
}

/// Results from a chaos experiment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosExperimentResult {
    pub experiment_type: ChaosExperimentType,
    pub start_time: i64,
    pub end_time: i64,
    pub duration_secs: u64,
    pub pods_affected: u32,
    pub recovery_time_secs: Option<u64>,
    pub system_recovered: bool,
    pub error_message: Option<String>,
}

/// Chaos runner for executing experiments
pub struct ChaosRunner {
    client: Client,
}

impl ChaosRunner {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Run a chaos experiment and track recovery
    pub async fn run_experiment(
        &self,
        config: ChaosExperimentConfig,
    ) -> Result<ChaosExperimentResult> {
        let start_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        info!(
            "Starting chaos experiment: {} in namespace {}",
            config.experiment_type, config.namespace
        );

        // Execute the experiment
        let pods_affected = self.execute_experiment(&config).await?;

        // Wait for experiment duration
        tokio::time::sleep(Duration::from_secs(config.duration_secs)).await;

        // Monitor recovery
        let recovery_time = self.monitor_recovery(&config).await?;
        let system_recovered = recovery_time.is_some();

        let end_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = ChaosExperimentResult {
            experiment_type: config.experiment_type,
            start_time,
            end_time,
            duration_secs: config.duration_secs,
            pods_affected,
            recovery_time_secs: recovery_time,
            system_recovered,
            error_message: None,
        };

        info!("Chaos experiment completed: {:?}", result);

        Ok(result)
    }

    /// Execute the chaos experiment
    async fn execute_experiment(&self, config: &ChaosExperimentConfig) -> Result<u32> {
        match config.experiment_type {
            ChaosExperimentType::PodKill => self.execute_pod_kill(config).await,
            ChaosExperimentType::NetworkDelay => self.execute_network_delay(config).await,
            ChaosExperimentType::IoStress => self.execute_io_stress(config).await,
            ChaosExperimentType::CpuStress => self.execute_cpu_stress(config).await,
            ChaosExperimentType::MemoryPressure => self.execute_memory_pressure(config).await,
        }
    }

    /// Execute pod kill experiment
    async fn execute_pod_kill(&self, config: &ChaosExperimentConfig) -> Result<u32> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &config.namespace);
        let lp = ListParams::default().labels(&config.target_label_selector);
        let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

        let mut killed_count = 0;
        for pod in pod_list.items {
            let pod_name = pod.name_any();
            debug!("Killing pod: {}", pod_name);
            pods.delete(&pod_name, &Default::default())
                .await
                .map_err(Error::KubeError)?;
            killed_count += 1;
        }

        info!("Pod kill experiment: killed {} pods", killed_count);
        Ok(killed_count)
    }

    /// Execute network delay experiment (simulated via annotation)
    async fn execute_network_delay(&self, config: &ChaosExperimentConfig) -> Result<u32> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &config.namespace);
        let lp = ListParams::default().labels(&config.target_label_selector);
        let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

        let mut affected_count = 0;
        for pod in pod_list.items {
            let pod_name = pod.name_any();
            let mut pod_patch = pod.clone();

            let mut annotations = pod_patch.annotations().clone();
            annotations.insert(
                "chaos.mesh/network-delay".to_string(),
                format!(
                    "delay={}ms,jitter={}ms",
                    config.delay_ms.unwrap_or(100),
                    config.jitter_ms.unwrap_or(10)
                ),
            );
            pod_patch.metadata.annotations = Some(annotations);

            pods.patch(
                &pod_name,
                &PatchParams::apply("stellar-operator").force(),
                &Patch::Apply(&pod_patch),
            )
            .await
            .map_err(Error::KubeError)?;

            affected_count += 1;
        }

        info!(
            "Network delay experiment: affected {} pods with {}ms delay",
            affected_count,
            config.delay_ms.unwrap_or(100)
        );
        Ok(affected_count)
    }

    /// Execute IO stress experiment (simulated via annotation)
    async fn execute_io_stress(&self, config: &ChaosExperimentConfig) -> Result<u32> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &config.namespace);
        let lp = ListParams::default().labels(&config.target_label_selector);
        let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

        let mut affected_count = 0;
        for pod in pod_list.items {
            let pod_name = pod.name_any();
            let mut pod_patch = pod.clone();

            let mut annotations = pod_patch.annotations().clone();
            annotations.insert(
                "chaos.mesh/io-stress".to_string(),
                format!("workers={}", config.io_workers.unwrap_or(4)),
            );
            pod_patch.metadata.annotations = Some(annotations);

            pods.patch(
                &pod_name,
                &PatchParams::apply("stellar-operator").force(),
                &Patch::Apply(&pod_patch),
            )
            .await
            .map_err(Error::KubeError)?;

            affected_count += 1;
        }

        info!(
            "IO stress experiment: affected {} pods with {} workers",
            affected_count,
            config.io_workers.unwrap_or(4)
        );
        Ok(affected_count)
    }

    /// Execute CPU stress experiment (simulated via annotation)
    async fn execute_cpu_stress(&self, config: &ChaosExperimentConfig) -> Result<u32> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &config.namespace);
        let lp = ListParams::default().labels(&config.target_label_selector);
        let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

        let mut affected_count = 0;
        for pod in pod_list.items {
            let pod_name = pod.name_any();
            let mut pod_patch = pod.clone();

            let mut annotations = pod_patch.annotations().clone();
            annotations.insert(
                "chaos.mesh/cpu-stress".to_string(),
                format!("workers={}", config.cpu_workers.unwrap_or(2)),
            );
            pod_patch.metadata.annotations = Some(annotations);

            pods.patch(
                &pod_name,
                &PatchParams::apply("stellar-operator").force(),
                &Patch::Apply(&pod_patch),
            )
            .await
            .map_err(Error::KubeError)?;

            affected_count += 1;
        }

        info!(
            "CPU stress experiment: affected {} pods with {} workers",
            affected_count,
            config.cpu_workers.unwrap_or(2)
        );
        Ok(affected_count)
    }

    /// Execute memory pressure experiment (simulated via annotation)
    async fn execute_memory_pressure(&self, config: &ChaosExperimentConfig) -> Result<u32> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &config.namespace);
        let lp = ListParams::default().labels(&config.target_label_selector);
        let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

        let mut affected_count = 0;
        for pod in pod_list.items {
            let pod_name = pod.name_any();
            let mut pod_patch = pod.clone();

            let mut annotations = pod_patch.annotations().clone();
            annotations.insert(
                "chaos.mesh/memory-pressure".to_string(),
                format!("memory={}mb", config.memory_mb.unwrap_or(512)),
            );
            pod_patch.metadata.annotations = Some(annotations);

            pods.patch(
                &pod_name,
                &PatchParams::apply("stellar-operator").force(),
                &Patch::Apply(&pod_patch),
            )
            .await
            .map_err(Error::KubeError)?;

            affected_count += 1;
        }

        info!(
            "Memory pressure experiment: affected {} pods with {}mb pressure",
            affected_count,
            config.memory_mb.unwrap_or(512)
        );
        Ok(affected_count)
    }

    /// Monitor system recovery after chaos experiment
    async fn monitor_recovery(&self, config: &ChaosExperimentConfig) -> Result<Option<u64>> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &config.namespace);
        let start_time = SystemTime::now();
        let max_wait = Duration::from_secs(600); // 10 minutes max wait

        loop {
            let lp = ListParams::default().labels(&config.target_label_selector);
            let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

            let all_ready = pod_list.items.iter().all(|pod| {
                pod.status
                    .as_ref()
                    .and_then(|s| s.conditions.as_ref())
                    .map(|conds| {
                        conds
                            .iter()
                            .any(|c| c.type_ == "Ready" && c.status == "True")
                    })
                    .unwrap_or(false)
            });

            if all_ready {
                let recovery_time = start_time.elapsed().unwrap().as_secs();
                info!("System recovered in {} seconds", recovery_time);
                return Ok(Some(recovery_time));
            }

            if start_time.elapsed().unwrap() > max_wait {
                warn!(
                    "System did not recover within {} seconds",
                    max_wait.as_secs()
                );
                return Ok(None);
            }

            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chaos_experiment_type_display() {
        assert_eq!(ChaosExperimentType::PodKill.to_string(), "PodKill");
        assert_eq!(
            ChaosExperimentType::NetworkDelay.to_string(),
            "NetworkDelay"
        );
        assert_eq!(ChaosExperimentType::IoStress.to_string(), "IoStress");
    }

    #[test]
    fn test_chaos_experiment_result_creation() {
        let result = ChaosExperimentResult {
            experiment_type: ChaosExperimentType::PodKill,
            start_time: 1000,
            end_time: 2000,
            duration_secs: 60,
            pods_affected: 5,
            recovery_time_secs: Some(30),
            system_recovered: true,
            error_message: None,
        };

        assert_eq!(result.pods_affected, 5);
        assert!(result.system_recovered);
        assert_eq!(result.recovery_time_secs, Some(30));
    }
}
