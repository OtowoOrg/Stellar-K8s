//! Task definition and execution

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Task status
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TaskStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
    Cancelled,
}

/// Task result
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub status: TaskStatus,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub duration_secs: u64,
    pub retry_count: u32,
}

/// Task
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub task_type: String,
    pub config: serde_json::Value,
}

impl Task {
    pub fn new(id: String, name: String, task_type: String) -> Self {
        Self {
            id,
            name,
            task_type,
            config: serde_json::json!({}),
        }
    }
}
