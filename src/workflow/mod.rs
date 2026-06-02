//! Advanced Workflow Orchestration with DAG Execution Engine
//!
//! Provides comprehensive workflow orchestration with Directed Acyclic Graph (DAG) execution,
//! task dependencies, error handling, and monitoring.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  Workflow Orchestration System                           │
//! ├─────────────────────────────────────────────────────────┤
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐   │
//! │  │ DAG Builder  │  │ DAG Executor │  │ Task Manager │   │
//! │  └──────────────┘  └──────────────┘  └──────────────┘   │
//! │         │                 │                 │             │
//! │         └─────────────────┴─────────────────┘             │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Dependency Resolver     │                      │
//! │         │ (Topological Sort)      │                      │
//! │         └────────────┬────────────┘                      │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Execution Engine        │                      │
//! │         │ (Parallel/Sequential)   │                      │
//! │         └────────────┬────────────┘                      │
//! │                      │                                    │
//! │         ┌────────────▼────────────┐                      │
//! │         │ Monitoring & Metrics    │                      │
//! │         │ (Status/Performance)    │                      │
//! │         └────────────────────────┘                      │
//! └─────────────────────────────────────────────────────────┘
//! ```

pub mod dag;
pub mod task;
pub mod executor;
pub mod dependency;
pub mod monitoring;
pub mod audit;
pub mod crd;
pub mod templates;
pub mod visualization;

pub use dag::{DAG, DAGNode};
pub use task::{Task, TaskStatus, TaskResult};
pub use executor::{DAGExecutor, ExecutionMode};
pub use dependency::{DependencyResolver, TopologicalSort};
pub use monitoring::{WorkflowMonitor, ExecutionMetrics};
pub use audit::{AuditEntry, AuditTrail};
pub use crd::{RetrySpec, TaskAction, TaskSpec, WorkflowPhase, WorkflowSpec, WorkflowStatus};
pub use templates::{disaster_recovery_workflow, migration_workflow, upgrade_workflow};
pub use visualization::{audit_to_mermaid, to_dot, to_mermaid};

use std::sync::Arc;
use tracing::info;

/// Workflow Orchestration System Configuration
#[derive(Clone, Debug)]
pub struct WorkflowConfig {
    pub max_parallel_tasks: usize,
    pub task_timeout_secs: u64,
    pub enable_retry: bool,
    pub max_retries: u32,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            max_parallel_tasks: 10,
            task_timeout_secs: 3600,
            enable_retry: true,
            max_retries: 3,
        }
    }
}

/// Workflow Orchestration System
pub struct WorkflowOrchestrationSystem {
    config: WorkflowConfig,
    executor: Arc<DAGExecutor>,
    monitor: Arc<WorkflowMonitor>,
    dependency_resolver: Arc<DependencyResolver>,
}

impl WorkflowOrchestrationSystem {
    /// Create a new workflow orchestration system
    pub async fn new(config: WorkflowConfig) -> crate::error::Result<Self> {
        info!("Initializing Workflow Orchestration System");

        let executor = Arc::new(DAGExecutor::new(config.clone()).await?);
        let monitor = Arc::new(WorkflowMonitor::new().await?);
        let dependency_resolver = Arc::new(DependencyResolver::new());

        Ok(Self {
            config,
            executor,
            monitor,
            dependency_resolver,
        })
    }

    /// Execute a DAG
    pub async fn execute_dag(&self, dag: DAG) -> crate::error::Result<WorkflowResult> {
        info!("Executing workflow DAG: {}", dag.id);

        // Resolve dependencies
        let execution_order = self.dependency_resolver.resolve(&dag).await?;

        // Execute DAG
        let result = self.executor.execute(&dag, execution_order).await?;

        // Record metrics
        self.monitor.record_execution(&result).await?;

        Ok(result)
    }

    /// Get executor
    pub fn executor(&self) -> Arc<DAGExecutor> {
        self.executor.clone()
    }

    /// Get monitor
    pub fn monitor(&self) -> Arc<WorkflowMonitor> {
        self.monitor.clone()
    }

    /// Get dependency resolver
    pub fn dependency_resolver(&self) -> Arc<DependencyResolver> {
        self.dependency_resolver.clone()
    }
}

/// Workflow execution result
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkflowResult {
    pub workflow_id: String,
    pub status: WorkflowStatus,
    pub task_results: std::collections::HashMap<String, TaskResult>,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: chrono::DateTime<chrono::Utc>,
    pub duration_secs: u64,
}

/// Workflow status
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum WorkflowStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_workflow_system_creation() {
        let config = WorkflowConfig::default();
        let system = WorkflowOrchestrationSystem::new(config).await.unwrap();
        
        assert_eq!(system.config.max_parallel_tasks, 10);
    }
}
