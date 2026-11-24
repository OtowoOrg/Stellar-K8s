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
//! Audit logging for approval workflow

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use kube::{
    api::{Api, Patch, PatchParams},
    Client,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::error::{Error, Result};

/// Audit entry for approval workflow events
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalAuditEntry {
    /// Name of the ChangeRequest
    pub change_request: String,
    /// Namespace of the ChangeRequest
    pub namespace: String,
    /// Action performed
    pub action: String,
    /// Actor who performed the action
    pub actor: String,
    /// Additional details
    pub details: String,
    /// Timestamp of the event
    pub timestamp: DateTime<Utc>,
}

/// Audit log for approval workflow
pub struct ApprovalAuditLog {
    /// In-memory ring buffer for recent entries
    entries: Arc<RwLock<VecDeque<ApprovalAuditEntry>>>,
    /// Maximum entries to keep in memory
    max_entries: usize,
    /// Kubernetes client for persisting to ConfigMap
    client: Option<Client>,
    /// Namespace for ConfigMap
    namespace: Option<String>,
    /// ConfigMap name
    configmap_name: String,
}

impl ApprovalAuditLog {
    /// Create a new audit log
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
            client: None,
            namespace: None,
            configmap_name: "approval-audit-log".to_string(),
        }
    }

    /// Create a new audit log with Kubernetes persistence
    pub fn with_persistence(client: Client, namespace: String, configmap_name: String, max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
            client: Some(client),
            namespace: Some(namespace),
            configmap_name,
        }
    }

    /// Record an audit entry
    pub async fn record(&self, entry: ApprovalAuditEntry) {
        // Add to in-memory buffer
        {
            let mut entries = self.entries.write().await;
            entries.push_back(entry.clone());
            if entries.len() > self.max_entries {
                entries.pop_front();
            }
        }

        // Persist to ConfigMap if configured
        if let (Some(client), Some(namespace)) = (&self.client, &self.namespace) {
            if let Err(e) = self.persist_to_configmap(client, namespace, &entry).await {
                debug!("Failed to persist audit entry to ConfigMap: {}", e);
            }
        }

        info!(
            "Audit: {} | {} | {} | {}",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
            entry.action,
            entry.change_request,
            entry.details
        );
    }

    /// Persist audit entry to ConfigMap
    async fn persist_to_configmap(
        &self,
        client: &Client,
        namespace: &str,
        entry: &ApprovalAuditEntry,
    ) -> Result<()> {
        let api: Api<k8s_openapi::api::core::v1::ConfigMap> = Api::namespaced(client.clone(), namespace);
        
        // Get existing ConfigMap or create new
        let cm = api.get(&self.configmap_name).await;
        
        let mut data = std::collections::BTreeMap::new();
        
        if let Ok(existing) = cm {
            if let Some(existing_data) = existing.data {
                data = existing_data;
            }
        }
        
        // Add new entry (keyed by timestamp)
        let key = format!("{}-{}", entry.timestamp.format("%Y%m%d%H%M%S%.3f"), entry.change_request);
        let value = serde_json::to_string(entry).map_err(Error::SerializationError)?;
        data.insert(key, value);
        
        // Trim old entries (keep last max_entries)
        if data.len() > self.max_entries {
            let keys_to_remove: Vec<String> = data.keys().take(data.len() - self.max_entries).cloned().collect();
            for key in keys_to_remove {
                data.remove(&key);
            }
        }
        
        let patch = serde_json::json!({
            "data": data,
        });
        
        api.patch(&self.configmap_name, &PatchParams::apply("approval-audit").force(), &Patch::Merge(&patch))
            .await
            .map_err(Error::KubeError)?;
        
        Ok(())
    }

    /// Get recent audit entries
    pub async fn get_recent(&self, limit: usize) -> Vec<ApprovalAuditEntry> {
        let entries = self.entries.read().await;
        entries.iter().rev().take(limit).cloned().collect()
    }

    /// Get audit entries for a specific ChangeRequest
    pub async fn get_for_change_request(&self, change_request: &str) -> Vec<ApprovalAuditEntry> {
        let entries = self.entries.read().await;
        entries.iter()
            .filter(|e| e.change_request == change_request)
            .cloned()
            .collect()
    }

    /// Get audit entries for a namespace
    pub async fn get_for_namespace(&self, namespace: &str) -> Vec<ApprovalAuditEntry> {
        let entries = self.entries.read().await;
        entries.iter()
            .filter(|e| e.namespace == namespace)
            .cloned()
            .collect()
    }

    /// Export audit log as JSON
    pub async fn export_json(&self) -> Result<String> {
        let entries = self.entries.read().await;
        serde_json::to_string_pretty(&*entries).map_err(Error::SerializationError)
    }

    /// Export audit log as CSV
    pub async fn export_csv(&self) -> Result<String> {
        let entries = self.entries.read().await;
        let mut csv = String::from("timestamp,change_request,namespace,action,actor,details\n");
        
        for entry in entries.iter() {
            csv.push_str(&format!(
                "{},{},{},{},{},{}\n",
                entry.timestamp.to_rfc3339(),
                entry.change_request,
                entry.namespace,
                entry.action,
                entry.actor,
                entry.details.replace(',', ";")
            ));
        }
        
        Ok(csv)
    }
}

/// Background task to clean up old audit entries
pub async fn audit_cleanup_task(audit_log: Arc<ApprovalAuditLog>, interval: Duration, max_age: Duration) {
    let mut interval_timer = tokio::time::interval(interval);
    
    loop {
        interval_timer.tick().await;
        
        let cutoff = Utc::now() - max_age;
        let mut entries = audit_log.entries.write().await;
        
        // Remove entries older than max_age
        while let Some(front) = entries.front() {
            if front.timestamp < cutoff {
                entries.pop_front();
            } else {
                break;
            }
        }
        
        debug!("Audit cleanup: {} entries remaining", entries.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_audit_log_record_and_retrieve() {
        let audit_log = ApprovalAuditLog::new(100);
        
        let entry = ApprovalAuditEntry {
            change_request: "cr-test-123".to_string(),
            namespace: "stellar".to_string(),
            action: "Approved".to_string(),
            actor: "user@example.com".to_string(),
            details: "Quorum reached".to_string(),
            timestamp: Utc::now(),
        };
        
        audit_log.record(entry.clone()).await;
        
        let recent = audit_log.get_recent(10).await;
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].change_request, "cr-test-123");
    }

    #[tokio::test]
    async fn test_audit_log_max_entries() {
        let audit_log = ApprovalAuditLog::new(3);
        
        for i in 0..5 {
            let entry = ApprovalAuditEntry {
                change_request: format!("cr-test-{}", i),
                namespace: "stellar".to_string(),
                action: "Test".to_string(),
                actor: "system".to_string(),
                details: "test".to_string(),
                timestamp: Utc::now(),
            };
            audit_log.record(entry).await;
        }
        
        let recent = audit_log.get_recent(10).await;
        assert_eq!(recent.len(), 3);
    }

    #[tokio::test]
    async fn test_audit_log_export_json() {
        let audit_log = ApprovalAuditLog::new(100);
        
        let entry = ApprovalAuditEntry {
            change_request: "cr-test-123".to_string(),
            namespace: "stellar".to_string(),
            action: "Approved".to_string(),
            actor: "user@example.com".to_string(),
            details: "Quorum reached".to_string(),
            timestamp: Utc::now(),
        };
        
        audit_log.record(entry).await;
        
        let json = audit_log.export_json().await.unwrap();
        assert!(json.contains("cr-test-123"));
        assert!(json.contains("Approved"));
    }

    #[tokio::test]
    async fn test_audit_log_export_csv() {
        let audit_log = ApprovalAuditLog::new(100);
        
        let entry = ApprovalAuditEntry {
            change_request: "cr-test-123".to_string(),
            namespace: "stellar".to_string(),
            action: "Approved".to_string(),
            actor: "user@example.com".to_string(),
            details: "Quorum reached".to_string(),
            timestamp: Utc::now(),
        };
        
        audit_log.record(entry).await;
        
        let csv = audit_log.export_csv().await.unwrap();
        assert!(csv.contains("cr-test-123"));
        assert!(csv.contains("Approved"));
    }
}
