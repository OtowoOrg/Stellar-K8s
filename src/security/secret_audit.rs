//! Immutable audit trail for secret access and mutations.

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;

/// Secret audit event types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SecretAuditAction {
    Encrypt,
    Decrypt,
    Rotate,
    Sync,
    Rollback,
    AccessAnomaly,
}

/// Immutable audit entry with hash chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretAuditEntry {
    pub timestamp: DateTime<Utc>,
    pub action: SecretAuditAction,
    pub secret_name: String,
    pub namespace: String,
    pub actor: String,
    pub version: u32,
    pub success: bool,
    pub previous_hash: String,
    pub entry_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// Tamper-resistant audit log with hash chain integrity.
pub struct SecretAuditLog {
    entries: Mutex<Vec<SecretAuditEntry>>,
    signing_key: SigningKey,
    last_hash: Mutex<String>,
}

impl SecretAuditLog {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            signing_key: SigningKey::generate(&mut OsRng),
            last_hash: Mutex::new("genesis".to_string()),
        }
    }

    pub fn record(
        &self,
        action: SecretAuditAction,
        secret_name: &str,
        namespace: &str,
        actor: &str,
        version: u32,
        success: bool,
        details: Option<String>,
    ) -> SecretAuditEntry {
        let prev_hash = self.last_hash.lock().unwrap().clone();
        let timestamp = Utc::now();

        let payload = format!(
            "{:?}|{}|{}|{}|{}|{}|{}|{}",
            action, secret_name, namespace, actor, version, success, prev_hash, timestamp
        );
        let entry_hash = hex::encode(Sha256::digest(payload.as_bytes()));

        let entry = SecretAuditEntry {
            timestamp,
            action,
            secret_name: secret_name.to_string(),
            namespace: namespace.to_string(),
            actor: actor.to_string(),
            version,
            success,
            previous_hash: prev_hash,
            entry_hash: entry_hash.clone(),
            details,
        };

        *self.last_hash.lock().unwrap() = entry_hash;
        self.entries.lock().unwrap().push(entry.clone());
        entry
    }

    pub fn entries(&self) -> Vec<SecretAuditEntry> {
        self.entries.lock().unwrap().clone()
    }

    pub fn verify_chain(&self) -> bool {
        let entries = self.entries.lock().unwrap();
        let mut expected_prev = "genesis".to_string();

        for entry in entries.iter() {
            if entry.previous_hash != expected_prev {
                return false;
            }
            let payload = format!(
                "{:?}|{}|{}|{}|{}|{}|{}|{}",
                entry.action,
                entry.secret_name,
                entry.namespace,
                entry.actor,
                entry.version,
                entry.success,
                entry.previous_hash,
                entry.timestamp
            );
            let hash = hex::encode(Sha256::digest(payload.as_bytes()));
            if hash != entry.entry_hash {
                return false;
            }
            expected_prev = entry.entry_hash.clone();
        }
        true
    }

    pub fn sign_log(&self) -> String {
        let entries = self.entries.lock().unwrap();
        let bytes = serde_json::to_vec(&*entries).unwrap_or_default();
        let digest = Sha256::digest(&bytes);
        hex::encode(self.signing_key.sign(digest.as_slice()).to_bytes())
    }

    /// Detect anomalous access patterns (e.g. rapid decrypt attempts).
    pub fn detect_anomalies(&self, window_secs: i64, threshold: usize) -> Vec<String> {
        let entries = self.entries.lock().unwrap();
        let cutoff = Utc::now() - chrono::Duration::seconds(window_secs);
        let recent_decrypts: Vec<_> = entries
            .iter()
            .filter(|e| e.action == SecretAuditAction::Decrypt && e.timestamp > cutoff)
            .collect();

        if recent_decrypts.len() > threshold {
            vec![format!(
                "Anomaly: {} decrypt attempts in {}s (threshold: {})",
                recent_decrypts.len(),
                window_secs,
                threshold
            )]
        } else {
            Vec::new()
        }
    }
}

impl Default for SecretAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_chain_integrity() {
        let log = SecretAuditLog::new();
        log.record(
            SecretAuditAction::Encrypt,
            "seed",
            "stellar",
            "operator",
            1,
            true,
            None,
        );
        log.record(
            SecretAuditAction::Rotate,
            "seed",
            "stellar",
            "operator",
            2,
            true,
            None,
        );
        assert!(log.verify_chain());
        assert_eq!(log.entries().len(), 2);
    }

    #[test]
    fn detect_decrypt_anomaly() {
        let log = SecretAuditLog::new();
        for _ in 0..10 {
            log.record(
                SecretAuditAction::Decrypt,
                "seed",
                "stellar",
                "unknown",
                1,
                true,
                None,
            );
        }
        let anomalies = log.detect_anomalies(60, 5);
        assert_eq!(anomalies.len(), 1);
    }
}
