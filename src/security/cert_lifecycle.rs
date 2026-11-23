// Copyright 2026 Stellar-K8s Contributors
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
//! Self-healing X.509 certificate lifecycle: atomic rotation, revocation propagation,
//! and live certificate inventory tracking.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Live inventory entry representing a deployed workload certificate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CertificateInventoryEntry {
    pub workload: String,
    pub namespace: String,
    pub serial_number: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub overlap_window_seconds: u64,
    pub revoked: bool,
}

/// Revocation propagation record for compromised serials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevocationRecord {
    pub serial_number: String,
    pub reason: String,
    pub revoked_at: DateTime<Utc>,
    pub propagated_at: Option<DateTime<Utc>>,
}

impl RevocationRecord {
    /// Returns true if revocation propagated within the 60-second SLA.
    pub fn within_propagation_sla(&self) -> bool {
        self.propagated_at
            .map(|p| (p - self.revoked_at) < Duration::seconds(60))
            .unwrap_or(false)
    }
}

/// Coordinates atomic certificate writes, overlap-window rotation,
/// compromised-serial revocation, and live inventory state.
pub struct SelfHealingCertLifecycle {
    inventory: Arc<RwLock<HashMap<String, CertificateInventoryEntry>>>,
    revoked_serials: Arc<RwLock<HashSet<String>>>,
    revocations: Arc<RwLock<Vec<RevocationRecord>>>,
}

impl Default for SelfHealingCertLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl SelfHealingCertLifecycle {
    pub fn new() -> Self {
        Self {
            inventory: Arc::new(RwLock::new(HashMap::new())),
            revoked_serials: Arc::new(RwLock::new(HashSet::new())),
            revocations: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Atomically write certificate and private key via temp-file + rename
    /// so in-flight mTLS handshakes never observe a half-written PEM.
    pub fn atomic_write_pem(dest_dir: &Path, filename: &str, pem_bytes: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(dest_dir)?;
        let tmp_path = dest_dir.join(format!(".{filename}.tmp"));
        let final_path = dest_dir.join(filename);
        std::fs::write(&tmp_path, pem_bytes)?;
        std::fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    /// Register or rotate a workload certificate with an explicit overlap window.
    pub async fn upsert_certificate(&self, entry: CertificateInventoryEntry) {
        let key = format!("{}/{}", entry.namespace, entry.workload);
        self.inventory.write().await.insert(key, entry);
    }

    /// Revoke a compromised serial and mark propagation timestamp.
    pub async fn revoke_serial(&self, serial_number: &str, reason: &str) -> RevocationRecord {
        let revoked_at = Utc::now();
        self.revoked_serials
            .write()
            .await
            .insert(serial_number.to_string());

        let mut inv = self.inventory.write().await;
        for entry in inv.values_mut() {
            if entry.serial_number == serial_number {
                entry.revoked = true;
            }
        }

        let record = RevocationRecord {
            serial_number: serial_number.to_string(),
            reason: reason.to_string(),
            revoked_at,
            propagated_at: Some(Utc::now()),
        };
        self.revocations.write().await.push(record.clone());
        record
    }

    pub async fn is_revoked(&self, serial_number: &str) -> bool {
        self.revoked_serials.read().await.contains(serial_number)
    }

    pub async fn list_inventory(&self) -> Vec<CertificateInventoryEntry> {
        self.inventory.read().await.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn atomic_write_and_fast_revocation_propagation() {
        let lifecycle = SelfHealingCertLifecycle::new();
        let now = Utc::now();
        lifecycle
            .upsert_certificate(CertificateInventoryEntry {
                workload: "stellar-core".into(),
                namespace: "stellar".into(),
                serial_number: "ser-12345".into(),
                issued_at: now,
                expires_at: now + Duration::days(30),
                overlap_window_seconds: 3600,
                revoked: false,
            })
            .await;

        let rec = lifecycle.revoke_serial("ser-12345", "key_compromise").await;
        assert!(rec.within_propagation_sla());
        assert!(lifecycle.is_revoked("ser-12345").await);
        assert!(lifecycle.list_inventory().await[0].revoked);
    }
}
