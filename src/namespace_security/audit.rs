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
//! Audit logging for namespace security baseline

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use kube::{
    api::{Api, Patch, PatchParams},
    Client,
};
use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::error::{Error, Result};

/// Audit entry for baseline events
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub namespace: String,
    pub action: String,
    pub resource_type: String,
    pub resource_name: String,
    pub success: bool,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    pub dry_run: bool,
}

/// Audit configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditConfig {
    /// Maximum entries to keep in memory
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    /// Enable Kubernetes ConfigMap persistence
    #[serde(default)]
    pub persist_to_configmap: bool,
    /// ConfigMap name
    #[serde(default = "default_configmap_name")]
    pub configmap_name: String,
    /// ConfigMap namespace
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Enable webhook notifications
    #[serde(default)]
    pub enable_webhook: bool,
    /// Webhook URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
}

fn default_max_entries() -> usize { 10000 }
fn default_configmap_name() -> String { "security-baseline-audit".to_string() }
fn default_namespace() -> String { "observability".to_string() }

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            max_entries: default_max_entries(),
            persist_to_configmap: true,
            configmap_name: default_configmap_name(),
            namespace: default_namespace(),
            enable_webhook: false,
            webhook_url: None,
        }
    }
}

/// Baseline audit log
pub struct BaselineAuditLog {
    config: AuditConfig,
    /// In-memory ring buffer for recent entries
    entries: Arc<RwLock<VecDeque<AuditEntry>>>,
    /// Kubernetes client for persistence
    client: Option<Client>,
    /// Prometheus metrics
    metrics: Arc<AuditMetrics>,
}

/// Prometheus metrics for audit log
pub struct AuditMetrics {
    pub entries_total: Family<AuditLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub entries_by_action: Family<AuditActionLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
    pub entries_by_result: Family<AuditResultLabels, Counter<u64, std::sync::atomic::AtomicU64>>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct AuditLabels {
    pub namespace: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct AuditActionLabels {
    pub action: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct AuditResultLabels {
    pub result: String, // success, failure
}

impl AuditMetrics {
    pub fn new(registry: &mut Registry) -> Self {
        let metrics = Self {
            entries_total: Family::default(),
            entries_by_action: Family::default(),
            entries_by_result: Family::default(),
        };

        registry.register(
            "security_baseline_audit_entries_total",
            "Total audit entries recorded",
            metrics.entries_total.clone(),
        ).unwrap();

        registry.register(
            "security_baseline_audit_entries_by_action",
            "Audit entries by action type",
            metrics.entries_by_action.clone(),
        ).unwrap();

        registry.register(
            "security_baseline_audit_entries_by_result",
            "Audit entries by result",
            metrics.entries_by_result.clone(),
        ).unwrap();

        metrics
    }
}

impl BaselineAuditLog {
    /// Create a new audit log
    pub fn new(config: AuditConfig, registry: &mut Registry) -> Self {
        let max_entries = config.max_entries;
        let metrics = Arc::new(AuditMetrics::new(registry));
        Self {
            config,
            entries: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            client: None,
            metrics,
        }
    }

    /// Create with Kubernetes client for persistence
    pub fn with_client(mut self, client: Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Record an audit entry
    pub async fn record(&self, entry: AuditEntry) {
        // Add to in-memory buffer
        {
            let mut entries = self.entries.write().await;
            entries.push_back(entry.clone());
            if entries.len() > self.config.max_entries {
                entries.pop_front();
            }
        }

        // Update metrics
        self.metrics.entries_total
            .get_or_create(&AuditLabels {
                namespace: entry.namespace.clone(),
            })
            .inc();

        self.metrics.entries_by_action
            .get_or_create(&AuditActionLabels {
                action: entry.action.clone(),
            })
            .inc();

        self.metrics.entries_by_result
            .get_or_create(&AuditResultLabels {
                result: if entry.success { "success" } else { "failure" }.to_string(),
            })
            .inc();

        // Persist to ConfigMap if enabled
        if self.config.persist_to_configmap {
            if let Some(client) = &self.client {
                if let Err(e) = self.persist_to_configmap(client, &entry).await {
                    debug!("Failed to persist audit entry: {}", e);
                }
            }
        }

        // Send webhook if enabled
        if self.config.enable_webhook {
            if let Some(url) = &self.config.webhook_url {
                self.send_webhook(url, &entry).await;
            }
        }

        info!(
            "AUDIT: {} | {} | {} | {} | success={} dry_run={}",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
            entry.namespace,
            entry.action,
            entry.resource_type,
            entry.success,
            entry.dry_run
        );
    }

    /// Persist audit entry to ConfigMap
    async fn persist_to_configmap(&self, client: &Client, entry: &AuditEntry) -> Result<()> {
        let api: Api<k8s_openapi::api::core::v1::ConfigMap> = 
            Api::namespaced(client.clone(), &self.config.namespace);
        
        let cm = api.get(&self.config.configmap_name).await;
        
        let mut data = std::collections::BTreeMap::new();
        
        if let Ok(existing) = cm {
            if let Some(existing_data) = existing.data {
                data = existing_data;
            }
        }
        
        // Add new entry
        let key = format!("{}-{}", entry.timestamp.format("%Y%m%d%H%M%S%.3f"), entry.resource_name);
        let value = serde_json::to_string(entry).map_err(Error::SerializationError)?;
        data.insert(key, value);
        
        // Trim old entries
        if data.len() > self.config.max_entries {
            let keys_to_remove: Vec<String> = data.keys().take(data.len() - self.config.max_entries).cloned().collect();
            for key in keys_to_remove {
                data.remove(&key);
            }
        }
        
        let patch = serde_json::json!({
            "data": data,
        });
        
        api.patch(&self.config.configmap_name, &PatchParams::apply("baseline-audit").force(), &Patch::Merge(&patch))
            .await
            .map_err(Error::KubeError)?;
        
        Ok(())
    }

    /// Send webhook notification
    async fn send_webhook(&self, url: &str, entry: &AuditEntry) {
        let client = reqwest::Client::new();
        if let Err(e) = client.post(url).json(entry).send().await {
            debug!("Failed to send audit webhook: {}", e);
        }
    }

    /// Get recent audit entries
    pub async fn get_recent(&self, limit: usize) -> Vec<AuditEntry> {
        let entries = self.entries.read().await;
        entries.iter().rev().take(limit).cloned().collect()
    }

    /// Get audit entries for a namespace
    pub async fn get_for_namespace(&self, namespace: &str) -> Vec<AuditEntry> {
        let entries = self.entries.read().await;
        entries.iter()
            .filter(|e| e.namespace == namespace)
            .cloned()
            .collect()
    }

    /// Get audit entries for an action type
    pub async fn get_for_action(&self, action: &str) -> Vec<AuditEntry> {
        let entries = self.entries.read().await;
        entries.iter()
            .filter(|e| e.action == action)
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
        let mut csv = String::from("timestamp,namespace,action,resource_type,resource_name,success,message,dry_run\n");
        
        for entry in entries.iter() {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                entry.timestamp.to_rfc3339(),
                entry.namespace,
                entry.action,
                entry.resource_type,
                entry.resource_name,
                entry.success,
                entry.message.replace(',', ";"),
                entry.dry_run
            ));
        }
        
        Ok(csv)
    }

    /// Get audit statistics
    pub async fn get_stats(&self) -> AuditStats {
        let entries = self.entries.read().await;
        let mut stats = AuditStats::default();
        stats.total_entries = entries.len();
        
        for entry in entries.iter() {
            if entry.success {
                stats.success_count += 1;
            } else {
                stats.failure_count += 1;
            }
            
            if entry.dry_run {
                stats.dry_run_count += 1;
            }
            
            *stats.actions.entry(entry.action.clone()).or_insert(0) += 1;
            *stats.namespaces.entry(entry.namespace.clone()).or_insert(0) += 1;
        }
        
        stats
    }
}

/// Audit statistics
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditStats {
    pub total_entries: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub dry_run_count: usize,
    pub actions: BTreeMap<String, usize>,
    pub namespaces: BTreeMap<String, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn test_audit_log_record() {
        let mut registry = Registry::default();
        let config = AuditConfig::default();
        let audit_log = BaselineAuditLog::new(config, &mut registry);
        
        let entry = AuditEntry {
            namespace: "test-ns".to_string(),
            action: "AutoRemediation".to_string(),
            resource_type: "NetworkPolicy".to_string(),
            resource_name: "default-deny-ingress".to_string(),
            success: true,
            message: "Created default deny ingress".to_string(),
            timestamp: Utc::now(),
            dry_run: false,
        };
        
        audit_log.record(entry).await;
        
        let recent = audit_log.get_recent(10).await;
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].namespace, "test-ns");
    }

    #[tokio::test]
    async fn test_audit_log_stats() {
        let mut registry = Registry::default();
        let config = AuditConfig::default();
        let audit_log = BaselineAuditLog::new(config, &mut registry);
        
        for i in 0..5 {
            let entry = AuditEntry {
                namespace: format!("ns-{}", i % 2),
                action: if i % 2 == 0 { "AutoRemediation" } else { "Evaluation" }.to_string(),
                resource_type: "NetworkPolicy".to_string(),
                resource_name: format!("np-{}", i),
                success: i % 3 != 0,
                message: "test".to_string(),
                timestamp: Utc::now(),
                dry_run: i % 2 == 0,
            };
            audit_log.record(entry).await;
        }
        
        let stats = audit_log.get_stats().await;
        assert_eq!(stats.total_entries, 5);
        assert_eq!(stats.success_count, 3); // 0, 1, 3, 4 succeed (0,1,2,3,4 -> 2 fails)
        assert_eq!(stats.dry_run_count, 3); // 0, 2, 4 are dry_run
    }

    #[tokio::test]
    async fn test_audit_log_export() {
        let mut registry = Registry::default();
        let config = AuditConfig::default();
        let audit_log = BaselineAuditLog::new(config, &mut registry);
        
        let entry = AuditEntry {
            namespace: "test-ns".to_string(),
            action: "AutoRemediation".to_string(),
            resource_type: "NetworkPolicy".to_string(),
            resource_name: "default-deny-ingress".to_string(),
            success: true,
            message: "Created default deny ingress".to_string(),
            timestamp: Utc::now(),
            dry_run: false,
        };
        
        audit_log.record(entry).await;
        
        let json = audit_log.export_json().await.unwrap();
        assert!(json.contains("test-ns"));
        assert!(json.contains("AutoRemediation"));
        
        let csv = audit_log.export_csv().await.unwrap();
        assert!(csv.contains("test-ns"));
        assert!(csv.contains("AutoRemediation"));
    }
}
