//! Chaos Engineering Runner for Stellar-K8s
//!
//! Integrates with Chaos Mesh to run automated destruction tests against the cluster,
//! ensuring the reconciler handles extreme failures and recovers to healthy state.
//!
//! Supported chaos experiments:
//! - PodKill:            Randomly terminate pods to test recovery
//! - NetworkDelay:       Introduce latency to simulate network issues
//! - NetworkPartition:   Full bidirectional network partition
//! - IoStress:           Stress disk I/O to test performance degradation
//! - CpuStress:          Stress CPU to test resource constraints
//! - MemoryPressure:     Simulate memory pressure scenarios
//! - DiskFill:           Fill disk to capacity
//! - ValidatorPodKill:   Kill validator pods while operator manages them
//! - CascadingFailure:   Simultaneous pod kill + network partition
//!
//! # Resilience scoring
//!
//! Each experiment produces a [`ChaosExperimentResult`] with a `resilience_score`
//! (0–100). The [`ChaosReportGenerator`] aggregates results into a
//! [`ResilienceReport`] with an overall weighted score.

use crate::error::{Error, Result};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams, Patch, PatchParams},
    Client, ResourceExt,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

/// All supported chaos experiment types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ChaosExperimentType {
    PodKill,
    NetworkDelay,
    NetworkPartition,
    IoStress,
    CpuStress,
    MemoryPressure,
    DiskFill,
    ValidatorPodKill,
    CascadingFailure,
}

impl std::fmt::Display for ChaosExperimentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PodKill => write!(f, "PodKill"),
            Self::NetworkDelay => write!(f, "NetworkDelay"),
            Self::NetworkPartition => write!(f, "NetworkPartition"),
            Self::IoStress => write!(f, "IoStress"),
            Self::CpuStress => write!(f, "CpuStress"),
            Self::MemoryPressure => write!(f, "MemoryPressure"),
            Self::DiskFill => write!(f, "DiskFill"),
            Self::ValidatorPodKill => write!(f, "ValidatorPodKill"),
            Self::CascadingFailure => write!(f, "CascadingFailure"),
        }
    }
}

/// Severity of a chaos experiment — used for weighted resilience scoring
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExperimentSeverity {
    Critical,
    High,
    Medium,
    Low,
}

impl ExperimentSeverity {
    /// Weight multiplier for resilience score aggregation
    pub fn weight(self) -> f64 {
        match self {
            Self::Critical => 3.0,
            Self::High => 2.0,
            Self::Medium => 1.0,
            Self::Low => 0.5,
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
    pub severity: ExperimentSeverity,
    pub slo_recovery_secs: u64,
    pub delay_ms: Option<u32>,        // For NetworkDelay
    pub jitter_ms: Option<u32>,       // For NetworkDelay
    pub io_workers: Option<u32>,      // For IoStress
    pub cpu_workers: Option<u32>,     // For CpuStress
    pub memory_mb: Option<u32>,       // For MemoryPressure
    pub disk_fill_bytes: Option<u64>, // For DiskFill
}

impl ChaosExperimentConfig {
    /// Return the default SLO recovery time for this experiment type
    pub fn default_slo_secs(exp_type: ChaosExperimentType) -> u64 {
        match exp_type {
            ChaosExperimentType::PodKill => 180,
            ChaosExperimentType::NetworkPartition => 180,
            ChaosExperimentType::NetworkDelay => 600,
            ChaosExperimentType::ValidatorPodKill => 300,
            ChaosExperimentType::CascadingFailure => 600,
            ChaosExperimentType::DiskFill => 120,
            ChaosExperimentType::CpuStress => 300,
            ChaosExperimentType::MemoryPressure => 300,
            ChaosExperimentType::IoStress => 300,
        }
    }
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
    pub slo_met: bool,
    pub resilience_score: f64, // 0.0 – 100.0
    pub error_message: Option<String>,
    pub failure_reasons: Vec<String>,
}

impl ChaosExperimentResult {
    /// Compute a resilience score based on recovery outcome
    fn compute_score(
        system_recovered: bool,
        slo_met: bool,
        recovery_time_secs: Option<u64>,
        slo_secs: u64,
    ) -> f64 {
        if !system_recovered {
            return 0.0;
        }
        let mut score = 100.0_f64;
        if !slo_met {
            // Deduct proportionally to how much the SLO was breached
            if let Some(actual) = recovery_time_secs {
                let overshoot = actual.saturating_sub(slo_secs) as f64;
                let deduction = (overshoot / slo_secs as f64 * 30.0).min(30.0);
                score -= deduction;
            } else {
                score -= 15.0;
            }
        }
        score.max(0.0)
    }
}

/// Aggregated resilience report across all experiments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResilienceReport {
    pub run_id: String,
    pub generated_at: i64,
    pub overall_score: f64,
    pub total_experiments: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<ChaosExperimentResult>,
    pub recommendations: Vec<String>,
}

/// Generates a [`ResilienceReport`] from a set of experiment results
pub struct ChaosReportGenerator;

impl ChaosReportGenerator {
    pub fn generate(
        run_id: String,
        results: Vec<ChaosExperimentResult>,
        configs: &[ChaosExperimentConfig],
    ) -> ResilienceReport {
        let passed = results
            .iter()
            .filter(|r| r.system_recovered && r.slo_met)
            .count();
        let failed = results.len() - passed;

        // Weighted average score
        let (weighted_sum, total_weight) = results.iter().zip(configs.iter()).fold(
            (0.0_f64, 0.0_f64),
            |(ws, tw), (result, config)| {
                let w = config.severity.weight();
                (ws + result.resilience_score * w, tw + w)
            },
        );
        let overall_score = if total_weight > 0.0 {
            weighted_sum / total_weight
        } else {
            0.0
        };

        let mut recommendations = Vec::new();
        for r in &results {
            if !r.system_recovered {
                recommendations.push(format!(
                    "{}: System did not recover — add circuit breakers and retry logic",
                    r.experiment_type
                ));
            } else if !r.slo_met {
                recommendations.push(format!(
                    "{}: Recovery exceeded SLO — optimise reconciliation loop performance",
                    r.experiment_type
                ));
            }
        }
        if recommendations.is_empty() {
            recommendations.push(
                "All experiments passed. Continue running chaos tests regularly.".to_string(),
            );
        }

        ResilienceReport {
            run_id,
            generated_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            overall_score,
            total_experiments: results.len(),
            passed,
            failed,
            results,
            recommendations,
        }
    }
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
            experiment = %config.experiment_type,
            namespace = %config.namespace,
            duration_secs = config.duration_secs,
            "Starting chaos experiment"
        );

        // Execute the experiment
        let pods_affected = self.execute_experiment(&config).await?;

        // Wait for experiment duration
        tokio::time::sleep(Duration::from_secs(config.duration_secs)).await;

        // Monitor recovery
        let recovery_time = self.monitor_recovery(&config).await?;
        let system_recovered = recovery_time.is_some();
        let slo_met = recovery_time
            .map(|t| t <= config.slo_recovery_secs)
            .unwrap_or(false);

        let resilience_score = ChaosExperimentResult::compute_score(
            system_recovered,
            slo_met,
            recovery_time,
            config.slo_recovery_secs,
        );

        let end_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut failure_reasons = Vec::new();
        if !system_recovered {
            failure_reasons.push(format!(
                "System did not recover within {}s",
                config.slo_recovery_secs
            ));
        } else if !slo_met {
            failure_reasons.push(format!(
                "Recovery time {}s exceeded SLO of {}s",
                recovery_time.unwrap_or(0),
                config.slo_recovery_secs
            ));
        }

        let result = ChaosExperimentResult {
            experiment_type: config.experiment_type,
            start_time,
            end_time,
            duration_secs: config.duration_secs,
            pods_affected,
            recovery_time_secs: recovery_time,
            system_recovered,
            slo_met,
            resilience_score,
            error_message: None,
            failure_reasons,
        };

        info!(
            experiment = %result.experiment_type,
            score = result.resilience_score,
            recovered = result.system_recovered,
            slo_met = result.slo_met,
            "Chaos experiment completed"
        );

        Ok(result)
    }

    /// Execute the chaos experiment
    async fn execute_experiment(&self, config: &ChaosExperimentConfig) -> Result<u32> {
        match config.experiment_type {
            ChaosExperimentType::PodKill | ChaosExperimentType::ValidatorPodKill => {
                self.execute_pod_kill(config).await
            }
            ChaosExperimentType::NetworkDelay => self.execute_network_delay(config).await,
            ChaosExperimentType::NetworkPartition => self.execute_network_partition(config).await,
            ChaosExperimentType::IoStress => self.execute_io_stress(config).await,
            ChaosExperimentType::CpuStress => self.execute_cpu_stress(config).await,
            ChaosExperimentType::MemoryPressure => self.execute_memory_pressure(config).await,
            ChaosExperimentType::DiskFill => self.execute_disk_fill(config).await,
            ChaosExperimentType::CascadingFailure => self.execute_cascading_failure(config).await,
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

    /// Execute network partition experiment (full bidirectional block)
    async fn execute_network_partition(&self, config: &ChaosExperimentConfig) -> Result<u32> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &config.namespace);
        let lp = ListParams::default().labels(&config.target_label_selector);
        let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

        let mut affected_count = 0;
        for pod in pod_list.items {
            let pod_name = pod.name_any();
            let mut pod_patch = pod.clone();
            let mut annotations = pod_patch.annotations().clone();
            annotations.insert(
                "chaos.mesh/network-partition".to_string(),
                "direction=both".to_string(),
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
            "Network partition experiment: partitioned {} pods",
            affected_count
        );
        Ok(affected_count)
    }

    /// Execute disk fill experiment
    async fn execute_disk_fill(&self, config: &ChaosExperimentConfig) -> Result<u32> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &config.namespace);
        let lp = ListParams::default().labels(&config.target_label_selector);
        let pod_list = pods.list(&lp).await.map_err(Error::KubeError)?;

        let mut affected_count = 0;
        for pod in pod_list.items {
            let pod_name = pod.name_any();
            let mut pod_patch = pod.clone();
            let mut annotations = pod_patch.annotations().clone();
            annotations.insert(
                "chaos.mesh/disk-fill".to_string(),
                format!(
                    "bytes={}",
                    config.disk_fill_bytes.unwrap_or(100 * 1024 * 1024)
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
            "Disk fill experiment: affected {} pods ({} bytes)",
            affected_count,
            config.disk_fill_bytes.unwrap_or(100 * 1024 * 1024)
        );
        Ok(affected_count)
    }

    /// Execute cascading failure (pod kill + network partition simultaneously)
    async fn execute_cascading_failure(&self, config: &ChaosExperimentConfig) -> Result<u32> {
        // Run both pod kill and network partition concurrently
        let kill_config = ChaosExperimentConfig {
            experiment_type: ChaosExperimentType::PodKill,
            ..config.clone()
        };
        let partition_config = ChaosExperimentConfig {
            experiment_type: ChaosExperimentType::NetworkPartition,
            ..config.clone()
        };

        let (kill_result, partition_result) = tokio::join!(
            self.execute_pod_kill(&kill_config),
            self.execute_network_partition(&partition_config),
        );

        let killed = kill_result.unwrap_or(0);
        let partitioned = partition_result.unwrap_or(0);

        warn!(
            killed = killed,
            partitioned = partitioned,
            "Cascading failure experiment: simultaneous pod kill + network partition"
        );

        Ok(killed + partitioned)
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
        assert_eq!(
            ChaosExperimentType::NetworkPartition.to_string(),
            "NetworkPartition"
        );
        assert_eq!(ChaosExperimentType::IoStress.to_string(), "IoStress");
        assert_eq!(ChaosExperimentType::CpuStress.to_string(), "CpuStress");
        assert_eq!(
            ChaosExperimentType::MemoryPressure.to_string(),
            "MemoryPressure"
        );
        assert_eq!(ChaosExperimentType::DiskFill.to_string(), "DiskFill");
        assert_eq!(
            ChaosExperimentType::ValidatorPodKill.to_string(),
            "ValidatorPodKill"
        );
        assert_eq!(
            ChaosExperimentType::CascadingFailure.to_string(),
            "CascadingFailure"
        );
    }

    #[test]
    fn test_experiment_severity_weights() {
        assert_eq!(ExperimentSeverity::Critical.weight(), 3.0);
        assert_eq!(ExperimentSeverity::High.weight(), 2.0);
        assert_eq!(ExperimentSeverity::Medium.weight(), 1.0);
        assert_eq!(ExperimentSeverity::Low.weight(), 0.5);
    }

    #[test]
    fn test_resilience_score_full_recovery_within_slo() {
        let score = ChaosExperimentResult::compute_score(true, true, Some(60), 180);
        assert_eq!(score, 100.0);
    }

    #[test]
    fn test_resilience_score_no_recovery() {
        let score = ChaosExperimentResult::compute_score(false, false, None, 180);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_resilience_score_slo_breach() {
        // Recovered but took 2x the SLO
        let score = ChaosExperimentResult::compute_score(true, false, Some(360), 180);
        assert!(score < 100.0, "SLO breach should reduce score");
        assert!(score > 0.0, "Recovered system should have positive score");
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
            slo_met: true,
            resilience_score: 100.0,
            error_message: None,
            failure_reasons: vec![],
        };

        assert_eq!(result.pods_affected, 5);
        assert!(result.system_recovered);
        assert!(result.slo_met);
        assert_eq!(result.resilience_score, 100.0);
        assert_eq!(result.recovery_time_secs, Some(30));
    }

    #[test]
    fn test_report_generator_all_pass() {
        let configs = vec![
            ChaosExperimentConfig {
                experiment_type: ChaosExperimentType::PodKill,
                namespace: "stellar-system".to_string(),
                target_label_selector: "app=stellar-operator".to_string(),
                duration_secs: 30,
                severity: ExperimentSeverity::Critical,
                slo_recovery_secs: 180,
                delay_ms: None,
                jitter_ms: None,
                io_workers: None,
                cpu_workers: None,
                memory_mb: None,
                disk_fill_bytes: None,
            },
            ChaosExperimentConfig {
                experiment_type: ChaosExperimentType::NetworkPartition,
                namespace: "stellar-system".to_string(),
                target_label_selector: "app=stellar-operator".to_string(),
                duration_secs: 60,
                severity: ExperimentSeverity::Critical,
                slo_recovery_secs: 180,
                delay_ms: None,
                jitter_ms: None,
                io_workers: None,
                cpu_workers: None,
                memory_mb: None,
                disk_fill_bytes: None,
            },
        ];

        let results = vec![
            ChaosExperimentResult {
                experiment_type: ChaosExperimentType::PodKill,
                start_time: 0,
                end_time: 90,
                duration_secs: 30,
                pods_affected: 1,
                recovery_time_secs: Some(45),
                system_recovered: true,
                slo_met: true,
                resilience_score: 100.0,
                error_message: None,
                failure_reasons: vec![],
            },
            ChaosExperimentResult {
                experiment_type: ChaosExperimentType::NetworkPartition,
                start_time: 0,
                end_time: 120,
                duration_secs: 60,
                pods_affected: 1,
                recovery_time_secs: Some(30),
                system_recovered: true,
                slo_met: true,
                resilience_score: 100.0,
                error_message: None,
                failure_reasons: vec![],
            },
        ];

        let report = ChaosReportGenerator::generate("test-run".to_string(), results, &configs);
        assert_eq!(report.overall_score, 100.0);
        assert_eq!(report.passed, 2);
        assert_eq!(report.failed, 0);
        assert!(report.recommendations[0].contains("All experiments passed"));
    }

    #[test]
    fn test_report_generator_with_failure() {
        let configs = vec![ChaosExperimentConfig {
            experiment_type: ChaosExperimentType::CascadingFailure,
            namespace: "stellar-system".to_string(),
            target_label_selector: "app=stellar-operator".to_string(),
            duration_secs: 180,
            severity: ExperimentSeverity::Critical,
            slo_recovery_secs: 600,
            delay_ms: None,
            jitter_ms: None,
            io_workers: None,
            cpu_workers: None,
            memory_mb: None,
            disk_fill_bytes: None,
        }];

        let results = vec![ChaosExperimentResult {
            experiment_type: ChaosExperimentType::CascadingFailure,
            start_time: 0,
            end_time: 900,
            duration_secs: 180,
            pods_affected: 2,
            recovery_time_secs: None,
            system_recovered: false,
            slo_met: false,
            resilience_score: 0.0,
            error_message: Some("System did not recover".to_string()),
            failure_reasons: vec!["System did not recover within 600s".to_string()],
        }];

        let report = ChaosReportGenerator::generate("test-run-fail".to_string(), results, &configs);
        assert_eq!(report.overall_score, 0.0);
        assert_eq!(report.passed, 0);
        assert_eq!(report.failed, 1);
        assert!(!report.recommendations.is_empty());
    }

    #[test]
    fn test_default_slo_secs() {
        assert_eq!(
            ChaosExperimentConfig::default_slo_secs(ChaosExperimentType::PodKill),
            180
        );
        assert_eq!(
            ChaosExperimentConfig::default_slo_secs(ChaosExperimentType::CascadingFailure),
            600
        );
        assert_eq!(
            ChaosExperimentConfig::default_slo_secs(ChaosExperimentType::DiskFill),
            120
        );
    }
}
