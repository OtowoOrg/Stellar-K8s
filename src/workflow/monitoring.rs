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
//! Workflow monitoring and metrics

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use tracing::debug;

use crate::error::Result;
use super::WorkflowResult;

/// Execution metrics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionMetrics {
    pub total_tasks: usize,
    pub succeeded_tasks: usize,
    pub failed_tasks: usize,
    pub skipped_tasks: usize,
    pub total_duration_secs: u64,
    pub average_task_duration_secs: u64,
}

/// Workflow Monitor
pub struct WorkflowMonitor {
    executions: tokio::sync::RwLock<Vec<WorkflowResult>>,
}

impl WorkflowMonitor {
    pub async fn new() -> Result<Self> {
        debug!("Initializing Workflow Monitor");
        Ok(Self {
            executions: tokio::sync::RwLock::new(Vec::new()),
        })
    }

    pub async fn record_execution(&self, result: &WorkflowResult) -> Result<()> {
        let mut executions = self.executions.write().await;
        executions.push(result.clone());
        Ok(())
    }

    pub async fn get_metrics(&self) -> Result<ExecutionMetrics> {
        let executions = self.executions.read().await;

        let total_tasks: usize = executions.iter().map(|e| e.task_results.len()).sum();
        let succeeded_tasks: usize = executions
            .iter()
            .flat_map(|e| e.task_results.values())
            .filter(|t| t.status == super::task::TaskStatus::Succeeded)
            .count();
        let failed_tasks: usize = executions
            .iter()
            .flat_map(|e| e.task_results.values())
            .filter(|t| t.status == super::task::TaskStatus::Failed)
            .count();
        let skipped_tasks: usize = executions
            .iter()
            .flat_map(|e| e.task_results.values())
            .filter(|t| t.status == super::task::TaskStatus::Skipped)
            .count();

        let total_duration_secs: u64 = executions.iter().map(|e| e.duration_secs).sum();
        let average_task_duration_secs = if total_tasks > 0 {
            total_duration_secs / total_tasks as u64
        } else {
            0
        };

        Ok(ExecutionMetrics {
            total_tasks,
            succeeded_tasks,
            failed_tasks,
            skipped_tasks,
            total_duration_secs,
            average_task_duration_secs,
        })
    }

    pub async fn get_execution_history(&self, limit: usize) -> Result<Vec<WorkflowResult>> {
        let executions = self.executions.read().await;
        Ok(executions.iter().rev().take(limit).cloned().collect())
    }
}
