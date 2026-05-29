//! DAG Executor

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::Utc;
use tracing::{debug, info};

use crate::error::Result;
use super::dag::DAG;
use super::task::TaskResult;
use super::{WorkflowResult, WorkflowStatus};

/// Execution mode
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Sequential,
    Parallel,
}

/// DAG Executor
pub struct DAGExecutor {
    config: super::WorkflowConfig,
}

impl DAGExecutor {
    pub async fn new(config: super::WorkflowConfig) -> Result<Self> {
        debug!("Initializing DAG Executor");
        Ok(Self { config })
    }

    pub async fn execute(
        &self,
        dag: &DAG,
        execution_order: Vec<String>,
    ) -> Result<WorkflowResult> {
        info!("Executing DAG: {} with {} tasks", dag.id, execution_order.len());

        let start_time = Utc::now();
        let mut task_results = HashMap::new();

        for task_id in execution_order {
            if let Some(node) = dag.get_node(&task_id) {
                debug!("Executing task: {}", node.name);

                let result = TaskResult {
                    task_id: node.id.clone(),
                    status: super::task::TaskStatus::Succeeded,
                    output: Some(serde_json::json!({"status": "completed"})),
                    error: None,
                    start_time: Utc::now(),
                    end_time: Utc::now(),
                    duration_secs: 0,
                    retry_count: 0,
                };

                task_results.insert(task_id, result);
            }
        }

        let end_time = Utc::now();
        let duration_secs = (end_time - start_time).num_seconds() as u64;

        Ok(WorkflowResult {
            workflow_id: dag.id.clone(),
            status: WorkflowStatus::Succeeded,
            task_results,
            start_time,
            end_time,
            duration_secs,
        })
    }
}
