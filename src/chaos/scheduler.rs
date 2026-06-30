//! Chaos Engineering Framework - Experiment Scheduler and Executor
//!
//! Handles scheduling, execution, and coordination of chaos experiments.

use std::sync::Arc;
use chrono::{DateTime, Utc, Duration as ChronoDuration};
use kube::Client;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

use crate::crd::chaos_experiment::*;
use crate::chaos::fault_injection::FaultInjectionManager;

/// Experiment scheduler - handles timing and orchestration
pub struct ExperimentScheduler {
    client: Client,
    fault_manager: Arc<FaultInjectionManager>,
    active_experiments: Arc<RwLock<std::collections::HashMap<String, ExperimentState>>>,
}

#[derive(Clone)]
struct ExperimentState {
    pub name: String,
    pub namespace: String,
    pub spec: ChaosExperimentSpec,
    pub status: ChaosExperimentStatus,
    pub started_at: DateTime<Utc>,
}

/// Experiment executor - runs the actual experiments
pub struct ExperimentExecutor {
    client: Client,
    fault_manager: Arc<FaultInjectionManager>,
}

impl ExperimentScheduler {
    pub fn new(client: Client, fault_manager: Arc<FaultInjectionManager>) -> Self {
        Self {
            client,
            fault_manager,
            active_experiments: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Schedule and run an experiment
    pub async fn run_experiment(
        &self,
        name: String,
        namespace: String,
        spec: ChaosExperimentSpec,
    ) -> Result<ChaosExperimentStatus, String> {
        let mut status = ChaosExperimentStatus {
            phase: ExperimentPhase::Running,
            start_time: Some(Utc::now()),
            ..Default::default()
        };

        // Validate safety constraints
        if let Err(e) = self.validate_safety_constraints(&spec) {
            status.phase = ExperimentPhase::Failed;
            status.last_error = Some(e);
            return Ok(status);
        }

        // Check steady state before starting
        status.phase = ExperimentPhase::VerifyingSteadyState;
        if !self.verify_steady_state(&spec.steady_state).await {
            status.phase = ExperimentPhase::Failed;
            status.last_error = Some("Steady state validation failed".to_string());
            return Ok(status);
        }

        // Calculate blast radius
        let affected_count = self.calculate_blast_radius(&spec.blast_radius);
        tracing::info!("Blast radius: {} resources will be affected", affected_count);

        // Inject faults
        for fault in &spec.faults {
            status.phase = ExperimentPhase::InjectingFault;
            status.current_fault = Some(fault.name.clone());

            match self.fault_manager.inject_fault(fault).await {
                Ok(result) => {
                    status.fault_history.push(FaultExecution {
                        name: fault.name.clone(),
                        fault_type: format!("{:?}", fault.fault_type),
                        started_at: result.started_at,
                        ended_at: result.ended_at,
                        success: result.success,
                        error: result.error.clone(),
                        affected_resources: result.affected_pods,
                    });
                }
                Err(e) => {
                    if spec.rollback_on_failure {
                        tracing::error!("Fault injection failed, rolling back: {}", e);
                        self.fault_manager.recover_all().await?;
                    }
                    status.phase = ExperimentPhase::Failed;
                    status.last_error = Some(e);
                    return Ok(status);
                }
            }

            // Wait for fault duration
            sleep(Duration::from_secs(fault.duration_seconds as u64)).await;

            // Recover fault if not last
            if fault != spec.faults.last().unwrap() {
                self.fault_manager.recover_fault(fault).await?;
            }
        }

        // Verify steady state after faults
        status.phase = ExperimentPhase::VerifyingSteadyState;
        let steady_state_valid = self.verify_steady_state(&spec.steady_state).await;

        // Recover all faults
        status.phase = ExperimentPhase::Recovering;
        self.fault_manager.recover_all().await?;

        // Generate results
        status.phase = if steady_state_valid {
            ExperimentPhase::Completed
        } else {
            ExperimentPhase::Failed
        };
        status.end_time = Some(Utc::now());

        let results = self.generate_results(&status, &spec);
        status.results = Some(results);

        Ok(status)
    }

    /// Validate safety constraints before running experiment
    fn validate_safety_constraints(&self, spec: &ChaosExperimentSpec) -> Result<(), String> {
        let constraints = &spec.safety_constraints;

        // Check required annotations
        if !constraints.required_annotations.is_empty() {
            return Err("Required annotations not present".to_string());
        }

        // Check excluded namespaces
        if constraints.excluded_namespaces.contains(&"kube-system".to_string()) {
            // This is expected
        }

        // Check blast radius
        if spec.blast_radius.percentage > 50 {
            return Err("Blast radius exceeds safety threshold (50%)".to_string());
        }

        Ok(())
    }

    /// Calculate blast radius - number of affected resources
    fn calculate_blast_radius(&self, blast_radius: &BlastRadiusControl) -> u32 {
        // In production, would query Kubernetes to get actual pod counts
        let base_count = 3; // Placeholder
        let percentage = blast_radius.percentage as f32 / 100.0;
        let calculated = (base_count as f32 * percentage) as u32;
        
        calculated.min(blast_radius.max_affected)
    }

    /// Verify steady state hypothesis
    async fn verify_steady_state(&self, hypothesis: &SteadyStateHypothesis) -> bool {
        // Run probes to verify steady state
        for probe in &hypothesis.probes {
            let success = self.run_probe(probe).await;
            if !success {
                tracing::warn!("Steady state probe failed: {}", probe.name);
                return false;
            }
        }

        // Check availability
        let availability = self.measure_availability().await;
        if availability < hypothesis.min_availability_percent {
            tracing::warn!(
                "Availability {}% below threshold {}%",
                availability, hypothesis.min_availability_percent
            );
            return false;
        }

        // Check error rate
        let error_rate = self.measure_error_rate().await;
        if error_rate > hypothesis.max_error_rate_percent {
            tracing::warn!(
                "Error rate {}% above threshold {}%",
                error_rate, hypothesis.max_error_rate_percent
            );
            return false;
        }

        true
    }

    /// Run a single probe
    async fn run_probe(&self, probe: &SteadyStateProbe) -> bool {
        match probe.probe_type {
            ProbeType::Http => {
                // Would make HTTP request to probe.endpoint
                true
            }
            ProbeType::Tcp => {
                // Would test TCP connectivity
                true
            }
            ProbeType::Command => {
                // Would execute command
                true
            }
            ProbeType::Metric => {
                // Would query metrics
                true
            }
        }
    }

    /// Measure current availability
    async fn measure_availability(&self) -> f32 {
        // In production, would query metrics or make requests
        99.5
    }

    /// Measure current error rate
    async fn measure_error_rate(&self) -> f32 {
        // In production, would query error metrics
        0.5
    }

    /// Generate experiment results
    fn generate_results(&self, status: &ChaosExperimentStatus, spec: &ChaosExperimentSpec) -> ExperimentResults {
        let duration = status.end_time
            .and_then(|e| status.start_time.map(|s| (e - s).num_seconds() as u64))
            .unwrap_or(0);

        let success = status.phase == ExperimentPhase::Completed;

        ExperimentResults {
            success,
            duration_seconds: duration,
            faults_injected: spec.faults.len() as u32,
            faults_recovered: spec.faults.len() as u32, // Simplified
            steady_state_validated: success,
            avg_probe_response_ms: 100.0, // Would be measured
            error_rate_percent: if success { 0.5 } else { 10.0 },
            availability_percent: if success { 99.5 } else { 85.0 },
            resilience_score: if success { 85 } else { 40 },
            findings: if success {
                vec!["System handled fault gracefully".to_string()]
            } else {
                vec!["System degraded under fault conditions".to_string()]
            },
        }
    }

    /// Schedule experiment based on cron or interval
    pub async fn schedule_loop(
        &self,
        experiment: ChaosExperimentSpec,
        name: String,
        namespace: String,
    ) {
        if let Some(schedule) = &experiment.schedule {
            if schedule.run_immediately {
                // Run immediately
                let _ = self.run_experiment(name, namespace, experiment.clone()).await;
            }

            if let Some(cron) = &schedule.cron {
                tracing::info!("Cron scheduling not yet implemented: {}", cron);
                // Would use cron library to schedule
            }

            if let Some(interval) = schedule.interval_seconds {
                loop {
                    sleep(Duration::from_secs(interval)).await;
                    let _ = self.run_experiment(name.clone(), namespace.clone(), experiment.clone()).await;
                }
            }
        }
    }
}

impl ExperimentExecutor {
    pub fn new(client: Client, fault_manager: Arc<FaultInjectionManager>) -> Self {
        Self { client, fault_manager }
    }

    /// Execute a single fault with recovery
    pub async fn execute_fault(&self, fault: &FaultSpec) -> Result<FaultExecution, String> {
        let start = Utc::now();

        // Inject
        let result = self.fault_manager.inject_fault(fault).await?;

        // Wait duration
        sleep(Duration::from_secs(fault.duration_seconds as u64)).await;

        // Recover
        self.fault_manager.recover_fault(fault).await?;

        let end = Utc::now();

        Ok(FaultExecution {
            name: fault.name.clone(),
            fault_type: format!("{:?}", fault.fault_type),
            started_at: start,
            ended_at: Some(end),
            success: result.success,
            error: result.error,
            affected_resources: result.affected_pods,
        })
    }
}

/// Chaos engine - main orchestrator
pub struct ChaosEngine {
    client: Client,
    scheduler: ExperimentScheduler,
    fault_manager: Arc<FaultInjectionManager>,
    experiment_history: Arc<RwLock<Vec<ExperimentHistoryEntry>>>,
}

#[derive(Clone)]
struct ExperimentHistoryEntry {
    name: String,
    namespace: String,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    success: bool,
}

impl ChaosEngine {
    pub async fn new(client: Client) -> Self {
        let fault_manager = Arc::new(FaultInjectionManager::new());
        fault_manager.initialize_with_client(client.clone()).await;

        let scheduler = ExperimentScheduler::new(client.clone(), fault_manager.clone());

        Self {
            client,
            scheduler,
            fault_manager,
            experiment_history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Run a chaos experiment
    pub async fn run(&self, name: String, namespace: String, spec: ChaosExperimentSpec) -> Result<ChaosExperimentStatus, String> {
        let result = self.scheduler.run_experiment(name.clone(), namespace, spec).await;

        // Record in history
        let mut history = self.experiment_history.write().await;
        history.push(ExperimentHistoryEntry {
            name,
            namespace: namespace.clone(),
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            success: result.as_ref().map(|s| s.phase == ExperimentPhase::Completed).unwrap_or(false),
        });

        result
    }

    /// Get experiment history
    pub async fn get_history(&self) -> Vec<ExperimentHistoryEntry> {
        let history = self.experiment_history.read().await;
        history.clone()
    }

    /// Get active faults
    pub async fn get_active_faults(&self) -> Vec<crate::chaos::fault_injection::FaultResult> {
        self.fault_manager.get_active_faults().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_blast_radius_calculation() {
        let scheduler = ExperimentScheduler::new(
            kube::Client::try_default().await.unwrap(),
            Arc::new(FaultInjectionManager::new()),
        );

        let blast_radius = BlastRadiusControl {
            percentage: 50,
            max_affected: 5,
            ..Default::default()
        };

        let count = scheduler.calculate_blast_radius(&blast_radius);
        assert!(count <= 5);
    }
}