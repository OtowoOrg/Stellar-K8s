use futures::{StreamExt, TryStreamExt};
use kube::{
    api::Api,
    client::Client,
    runtime::{watcher, WatchStreamExt},
    ResourceExt,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::controller::audit_log::{AdminAction, AuditEntry};
use crate::controller::audit_recorder::AuditRecorder;
use crate::crd::StellarNode;
use crate::error::Result;

/// Worker that watches for StellarNode changes and emits audit logs.
pub struct AuditWorker {
    client: Client,
    recorder: Arc<AuditRecorder>,
    /// Cache of resource versions to detect actual changes and avoid redundant audits.
    cache: Arc<RwLock<HashMap<String, Value>>>,
}

impl AuditWorker {
    pub fn new(client: Client, recorder: Arc<AuditRecorder>) -> Self {
        Self {
            client,
            recorder,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start the audit worker loop.
    pub async fn run(self) -> Result<()> {
        info!("Starting audit worker for StellarNode resources");

        let api: Api<StellarNode> = Api::all(self.client.clone());
        let wc = watcher::Config::default();

        let mut stream = watcher(api, wc).default_backoff().boxed();

        while let Some(event) = stream
            .try_next()
            .await
            .map_err(|e| crate::error::Error::InternalError(format!("Audit watcher error: {e}")))?
        {
            match event {
                watcher::Event::Apply(node) | watcher::Event::InitApply(node) => {
                    let name = node.name_any();
                    let current_val = serde_json::to_value(&node).unwrap_or_default();

                    let mut cache = self.cache.write().await;
                    if let Some(old) = cache.get(&name) {
                        if old != &current_val {
                            let diff = json_patch::diff(old, &current_val);
                            if !diff.0.is_empty() {
                                let actor = self.extract_actor(&node);

                                let entry = AuditEntry::new(
                                    AdminAction::NodeUpdate,
                                    actor,
                                    &name,
                                    node.namespace().unwrap_or_default(),
                                    Some("StellarNode updated"),
                                )
                                .with_diff(serde_json::to_value(diff).unwrap_or_default());

                                self.recorder.record(entry).await;
                            }
                        }
                    } else {
                        // New node created or first time seeing it
                        let actor = self.extract_actor(&node);
                        let entry = AuditEntry::new(
                            AdminAction::NodeCreate,
                            actor,
                            name.clone(),
                            node.namespace().unwrap_or_default(),
                            None,
                        )
                        .with_diff(current_val.clone());

                        self.recorder.record(entry).await;
                    }
                    cache.insert(name, current_val);
                }
                watcher::Event::Delete(node) => {
                    let name = node.name_any();
                    let actor = self.extract_actor(&node);

                    let entry = AuditEntry::new(
                        AdminAction::NodeDelete,
                        actor,
                        &name,
                        node.namespace().unwrap_or_default(),
                        Some("StellarNode deleted"),
                    );

                    self.recorder.record(entry).await;
                    self.cache.write().await.remove(&name);
                }
                watcher::Event::Init | watcher::Event::InitDone => {}
            }
        }

        Ok(())
    }

    fn extract_actor(&self, node: &StellarNode) -> String {
        // Attempt 1: Check for custom audit annotation (populated by admission webhook)
        if let Some(user) = node.annotations().get("audit.stellar.org/last-modified-by") {
            return user.clone();
        }

        // Attempt 2: Check managedFields
        if let Some(managed) = &node.metadata.managed_fields {
            // Find the last manager that wasn't the operator itself
            for field in managed.iter().rev() {
                if let Some(manager) = &field.manager {
                    if manager != "stellar-operator" && manager != "kube-controller-manager" {
                        return manager.clone();
                    }
                }
            }
        }

        "system:unknown".to_string()
    }
}
