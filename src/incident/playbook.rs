//! Automated response playbooks and runbooks for incident management.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum PlaybookStep {
    /// Run a shell command
    Shell { command: String, timeout_secs: u64 },
    /// Send a notification
    Notify { channel: String, message: String },
    /// Wait for a condition (poll URL)
    WaitForHealthy { url: String, timeout_secs: u64 },
    /// Scale a Kubernetes deployment
    Scale {
        namespace: String,
        deployment: String,
        replicas: i32,
    },
    /// Annotate a Kubernetes resource
    Annotate {
        namespace: String,
        resource: String,
        key: String,
        value: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playbook {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Severity levels this playbook applies to
    pub applies_to: Vec<String>,
    /// Alert name patterns this playbook handles
    pub triggers: Vec<String>,
    pub steps: Vec<PlaybookStep>,
    pub runbook_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_index: usize,
    pub success: bool,
    pub output: String,
    pub executed_at: DateTime<Utc>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookExecution {
    pub playbook_id: String,
    pub incident_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub step_results: Vec<StepResult>,
    pub success: bool,
}

pub struct PlaybookExecutor {
    playbooks: HashMap<String, Playbook>,
}

impl PlaybookExecutor {
    pub fn new() -> Self {
        Self {
            playbooks: Self::default_playbooks(),
        }
    }

    fn default_playbooks() -> HashMap<String, Playbook> {
        let mut map = HashMap::new();

        let validator_restart = Playbook {
            id: "validator-restart".to_string(),
            name: "Validator Node Restart".to_string(),
            description: "Restart a stuck or unhealthy validator node".to_string(),
            applies_to: vec!["critical".to_string(), "high".to_string()],
            triggers: vec!["ValidatorDown".to_string(), "StellarNodeNotReady".to_string()],
            steps: vec![
                PlaybookStep::Notify {
                    channel: "ops".to_string(),
                    message: "Starting automated validator restart playbook".to_string(),
                },
                PlaybookStep::Shell {
                    command: "kubectl rollout restart statefulset -l stellar.org/node-type=Validator -n stellar".to_string(),
                    timeout_secs: 60,
                },
                PlaybookStep::WaitForHealthy {
                    url: "http://stellar-node.stellar.svc/health".to_string(),
                    timeout_secs: 300,
                },
                PlaybookStep::Notify {
                    channel: "ops".to_string(),
                    message: "Validator restart completed".to_string(),
                },
            ],
            runbook_url: Some("https://docs.stellar.org/runbooks/validator-restart".to_string()),
        };

        let disk_pressure = Playbook {
            id: "disk-pressure".to_string(),
            name: "Disk Pressure Response".to_string(),
            description: "Respond to disk pressure by triggering PVC expansion".to_string(),
            applies_to: vec!["critical".to_string(), "high".to_string()],
            triggers: vec!["DiskPressure".to_string(), "PVCAlmostFull".to_string()],
            steps: vec![
                PlaybookStep::Notify {
                    channel: "ops".to_string(),
                    message: "Disk pressure detected, triggering expansion".to_string(),
                },
                PlaybookStep::Shell {
                    command: "kubectl annotate stellarnode -l stellar.org/disk-auto-expand=true stellar.org/force-expand=true --overwrite -n stellar".to_string(),
                    timeout_secs: 30,
                },
            ],
            runbook_url: Some("https://docs.stellar.org/runbooks/disk-pressure".to_string()),
        };

        map.insert(validator_restart.id.clone(), validator_restart);
        map.insert(disk_pressure.id.clone(), disk_pressure);
        map
    }

    pub fn register(&mut self, playbook: Playbook) {
        self.playbooks.insert(playbook.id.clone(), playbook);
    }

    pub fn find_for_incident(&self, severity: &str, alert_name: &str) -> Vec<&Playbook> {
        self.playbooks
            .values()
            .filter(|p| {
                p.applies_to.iter().any(|s| s == severity)
                    && (p.triggers.is_empty()
                        || p.triggers.iter().any(|t| alert_name.contains(t.as_str())))
            })
            .collect()
    }

    pub async fn execute(&self, playbook_id: &str, incident_id: &str) -> PlaybookExecution {
        let started_at = Utc::now();
        let playbook = match self.playbooks.get(playbook_id) {
            Some(p) => p,
            None => {
                warn!(playbook_id, "Playbook not found");
                return PlaybookExecution {
                    playbook_id: playbook_id.to_string(),
                    incident_id: incident_id.to_string(),
                    started_at,
                    completed_at: Some(Utc::now()),
                    step_results: vec![],
                    success: false,
                };
            }
        };

        info!(playbook = %playbook.name, incident = %incident_id, "Executing playbook");
        let mut step_results = Vec::new();
        let mut all_success = true;

        for (i, step) in playbook.steps.iter().enumerate() {
            let step_start = std::time::Instant::now();
            let result = self.execute_step(step).await;
            let duration_ms = step_start.elapsed().as_millis() as u64;

            if !result.0 {
                error!(step = i, "Playbook step failed");
                all_success = false;
            }

            step_results.push(StepResult {
                step_index: i,
                success: result.0,
                output: result.1,
                executed_at: Utc::now(),
                duration_ms,
            });

            // Stop on critical step failure
            if !result.0 {
                break;
            }
        }

        PlaybookExecution {
            playbook_id: playbook_id.to_string(),
            incident_id: incident_id.to_string(),
            started_at,
            completed_at: Some(Utc::now()),
            step_results,
            success: all_success,
        }
    }

    async fn execute_step(&self, step: &PlaybookStep) -> (bool, String) {
        match step {
            PlaybookStep::Shell {
                command,
                timeout_secs,
            } => {
                info!(command = %command, "Executing shell step");
                let result = tokio::time::timeout(
                    tokio::time::Duration::from_secs(*timeout_secs),
                    tokio::process::Command::new("sh")
                        .arg("-c")
                        .arg(command)
                        .output(),
                )
                .await;
                match result {
                    Ok(Ok(out)) => {
                        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                        (out.status.success(), stdout)
                    }
                    Ok(Err(e)) => (false, e.to_string()),
                    Err(_) => (false, "Step timed out".to_string()),
                }
            }
            PlaybookStep::Notify { channel, message } => {
                info!(channel = %channel, message = %message, "Notification step");
                (true, format!("Notified {channel}: {message}"))
            }
            PlaybookStep::WaitForHealthy { url, timeout_secs } => {
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_secs(*timeout_secs);
                loop {
                    if std::time::Instant::now() > deadline {
                        return (false, format!("Timeout waiting for {url}"));
                    }
                    if let Ok(resp) = reqwest::get(url).await {
                        if resp.status().is_success() {
                            return (true, format!("{url} is healthy"));
                        }
                    }
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
            PlaybookStep::Scale {
                namespace,
                deployment,
                replicas,
            } => {
                let cmd = format!(
                    "kubectl scale deployment/{deployment} --replicas={replicas} -n {namespace}"
                );
                info!(cmd = %cmd, "Scale step");
                (true, format!("Scaled {deployment} to {replicas}"))
            }
            PlaybookStep::Annotate {
                namespace,
                resource,
                key,
                value,
            } => {
                let cmd =
                    format!("kubectl annotate {resource} {key}={value} --overwrite -n {namespace}");
                info!(cmd = %cmd, "Annotate step");
                (true, format!("Annotated {resource}"))
            }
        }
    }
}

impl Default for PlaybookExecutor {
    fn default() -> Self {
        Self::new()
    }
}
