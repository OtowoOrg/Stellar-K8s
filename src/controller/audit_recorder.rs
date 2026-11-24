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
