//! Workflow audit trail with timestamped task execution log

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::workflow::task::TaskStatus;

/// A single entry in the audit trail
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub workflow_id: String,
    pub task_id: String,
    pub status: TaskStatus,
    pub attempt: u32,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    pub duration_ms: Option<u64>,
    pub metadata: serde_json::Value,
}

impl AuditEntry {
    pub fn new(workflow_id: &str, task_id: &str, status: TaskStatus, attempt: u32, message: &str) -> Self {
        Self {
            workflow_id: workflow_id.into(),
            task_id: task_id.into(),
            status,
            attempt,
            message: message.into(),
            timestamp: Utc::now(),
            duration_ms: None,
            metadata: serde_json::Value::Null,
        }
    }
}

/// Append-only audit trail for a workflow execution
pub struct AuditTrail {
    entries: Arc<RwLock<VecDeque<AuditEntry>>>,
    max_entries: usize,
}

impl AuditTrail {
    pub fn new(max_entries: usize) -> Self {
        Self { entries: Arc::new(RwLock::new(VecDeque::new())), max_entries }
    }

    pub async fn append(&self, entry: AuditEntry) {
        info!(
            workflow = %entry.workflow_id,
            task     = %entry.task_id,
            status   = ?entry.status,
            attempt  = entry.attempt,
            message  = %entry.message,
            "Workflow audit event"
        );
        let mut e = self.entries.write().await;
        if e.len() >= self.max_entries {
            e.pop_front();
        }
        e.push_back(entry);
    }

    pub async fn all_entries(&self) -> Vec<AuditEntry> {
        self.entries.read().await.iter().cloned().collect()
    }

    pub fn entries_for_task(&self, task_id: &str) -> Vec<AuditEntry> {
        // Sync helper for visualization (caller must hold the read lock externally
        // in async context; this is for test/non-async use)
        futures::executor::block_on(async {
            self.entries
                .read()
                .await
                .iter()
                .filter(|e| e.task_id == task_id)
                .cloned()
                .collect()
        })
    }

    pub async fn to_json(&self) -> serde_json::Value {
        let entries: Vec<AuditEntry> = self.all_entries().await;
        serde_json::json!({ "audit_trail": entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_append_and_query() {
        let trail = AuditTrail::new(100);
        trail.append(AuditEntry::new("wf1", "task-a", TaskStatus::Running, 1, "started")).await;
        trail.append(AuditEntry::new("wf1", "task-a", TaskStatus::Completed, 1, "done")).await;
        trail.append(AuditEntry::new("wf1", "task-b", TaskStatus::Running, 1, "started")).await;

        let all = trail.all_entries().await;
        assert_eq!(all.len(), 3);

        let task_a = trail.entries_for_task("task-a");
        assert_eq!(task_a.len(), 2);
        assert!(matches!(task_a.last().unwrap().status, TaskStatus::Completed));
    }

    #[tokio::test]
    async fn test_max_entries_cap() {
        let trail = AuditTrail::new(3);
        for i in 0..5u32 {
            trail.append(AuditEntry::new("wf1", "t1", TaskStatus::Running, i, "x")).await;
        }
        assert_eq!(trail.all_entries().await.len(), 3);
    }
}
