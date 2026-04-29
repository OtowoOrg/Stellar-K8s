use std::sync::Arc;

use crate::controller::audit_log::{AuditEntry, AuditLog};
use crate::controller::audit_sink::AuditSink;


/// Records audit entries to the in-memory log and multiple external sinks.
#[derive(Clone)]
pub struct AuditRecorder {
    log: Arc<AuditLog>,
    sinks: Vec<Arc<dyn AuditSink>>,
    kms_key_ref: Option<String>,
}

impl AuditRecorder {
    pub fn new(
        log: Arc<AuditLog>,
        sinks: Vec<Arc<dyn AuditSink>>,
        kms_key_ref: Option<String>,
    ) -> Self {
        Self {
            log,
            sinks,
            kms_key_ref,
        }
    }

    pub async fn record(&self, mut entry: AuditEntry) {
        // 1. Encrypt sensitive fields if KMS is configured
        if let Some(key_ref) = &self.kms_key_ref {
            use crate::controller::audit_sink::encrypt_audit_entry;
            if let Ok(encrypted) = encrypt_audit_entry(entry.clone(), key_ref).await {
                entry = encrypted;
            }
        }

        // 2. Record to in-memory log
        self.log.record(entry.clone());

        // 3. Persist to all external sinks
        for sink in &self.sinks {
            let _ = sink.persist(entry.clone()).await;
        }
    }

    pub fn log(&self) -> Arc<AuditLog> {
        Arc::clone(&self.log)
    }
}
