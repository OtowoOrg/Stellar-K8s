//! Workflow CRD with DAG task specification
//!
//! Defines the Workflow custom resource that Kubernetes users create
//! to express multi-step DAG operations on StellarNode clusters.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Workflow CRD spec — mirrors what users write in YAML
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowSpec {
    /// Human-readable description
    pub description: String,
    /// Optional cron schedule for recurring workflows
    pub schedule: Option<String>,
    /// Maximum concurrency for parallel task groups
    pub max_parallelism: usize,
    /// Global timeout for the entire workflow (seconds)
    pub timeout_secs: u64,
    /// Tasks making up the DAG
    pub tasks: Vec<TaskSpec>,
    /// Labels applied to all tasks
    pub labels: HashMap<String, String>,
}

/// Specification for a single task node in the DAG
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    pub name: String,
    /// IDs of tasks that must complete before this task runs
    pub depends_on: Vec<String>,
    pub action: TaskAction,
    pub retry: RetrySpec,
    /// Optional condition (expression string) — task is skipped when false
    pub condition: Option<String>,
    pub timeout_secs: u64,
}

/// The action this task performs
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TaskAction {
    /// Run a shell command in the Stellar operator context
    Shell { command: String, args: Vec<String> },
    /// Invoke a Kubernetes Job
    KubernetesJob { image: String, command: Vec<String>, namespace: String },
    /// Call a REST endpoint
    HttpCall { url: String, method: String, body: Option<String> },
    /// Upgrade a StellarNode to a new version
    StellarNodeUpgrade { node_name: String, target_version: String },
    /// Wait for a condition to become true
    WaitForCondition { condition: String, poll_interval_secs: u64 },
    /// No-op (useful for grouping/joining dependencies)
    Noop,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetrySpec {
    pub max_attempts: u32,
    pub initial_backoff_secs: u64,
    pub backoff_multiplier: f64,
    pub max_backoff_secs: u64,
}

impl Default for RetrySpec {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_secs: 2,
            backoff_multiplier: 2.0,
            max_backoff_secs: 60,
        }
    }
}

impl RetrySpec {
    pub fn backoff_for_attempt(&self, attempt: u32) -> u64 {
        let backoff = self.initial_backoff_secs as f64
            * self.backoff_multiplier.powi(attempt.saturating_sub(1) as i32);
        (backoff as u64).min(self.max_backoff_secs)
    }
}

/// Observed state of the Workflow (stored in .status)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkflowStatus {
    pub phase: WorkflowPhase,
    pub start_time: Option<DateTime<Utc>>,
    pub completion_time: Option<DateTime<Utc>>,
    pub tasks_total: usize,
    pub tasks_succeeded: usize,
    pub tasks_failed: usize,
    pub tasks_running: usize,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkflowPhase {
    #[default]
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff() {
        let retry = RetrySpec::default();
        assert_eq!(retry.backoff_for_attempt(1), 2);
        assert_eq!(retry.backoff_for_attempt(2), 4);
        assert_eq!(retry.backoff_for_attempt(3), 8);
    }

    #[test]
    fn test_backoff_capped_at_max() {
        let retry = RetrySpec { max_attempts: 10, initial_backoff_secs: 2, backoff_multiplier: 4.0, max_backoff_secs: 30 };
        // 2 * 4^10 = huge, but capped at 30
        assert_eq!(retry.backoff_for_attempt(10), 30);
    }
}
