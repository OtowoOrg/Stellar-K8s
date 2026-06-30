//! API key management and authentication.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// An API key with associated metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    /// SHA-256 hash of the raw key value (never store plaintext)
    pub key_hash: String,
    pub owner: String,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub active: bool,
    /// Custom rate-limit override (requests/sec); None = use default
    pub rate_limit_rps: Option<u32>,
}

impl ApiKey {
    /// Create a new API key, returning both the [`ApiKey`] record and the
    /// raw key string (shown once to the caller).
    pub fn generate(owner: impl Into<String>, scopes: Vec<String>) -> (Self, String) {
        let raw = format!("sk_{:x}", rand::random::<u128>());
        let key_hash = hex::encode(Sha256::digest(raw.as_bytes()));
        let id = format!("key_{:x}", rand::random::<u64>());
        let key = ApiKey {
            id,
            key_hash,
            owner: owner.into(),
            scopes,
            created_at: Utc::now().to_rfc3339(),
            expires_at: None,
            active: true,
            rate_limit_rps: None,
        };
        (key, raw)
    }

    /// Verify a raw key string against this record.
    pub fn verify(&self, raw: &str) -> bool {
        let hash = hex::encode(Sha256::digest(raw.as_bytes()));
        self.active && hash == self.key_hash
    }

    pub fn is_expired(&self) -> bool {
        if let Some(exp) = &self.expires_at {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(exp) {
                return Utc::now() > dt;
            }
        }
        false
    }
}

/// Thread-safe in-memory API key store.
#[derive(Clone, Default)]
pub struct ApiKeyStore {
    keys: Arc<RwLock<HashMap<String, ApiKey>>>,
}

impl ApiKeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace a key.
    pub async fn upsert(&self, key: ApiKey) {
        self.keys.write().await.insert(key.id.clone(), key);
    }

    /// Revoke a key by ID.
    pub async fn revoke(&self, id: &str) -> bool {
        let mut guard = self.keys.write().await;
        if let Some(k) = guard.get_mut(id) {
            k.active = false;
            return true;
        }
        false
    }

    /// Authenticate a raw key string.  Returns the matching [`ApiKey`] if
    /// valid and not expired.
    pub async fn authenticate(&self, raw: &str) -> Option<ApiKey> {
        let guard = self.keys.read().await;
        guard
            .values()
            .find(|k| k.verify(raw) && !k.is_expired())
            .cloned()
    }

    /// List all keys (without exposing hashes).
    pub async fn list(&self) -> Vec<ApiKeyInfo> {
        self.keys
            .read()
            .await
            .values()
            .map(|k| ApiKeyInfo {
                id: k.id.clone(),
                owner: k.owner.clone(),
                scopes: k.scopes.clone(),
                active: k.active,
                created_at: k.created_at.clone(),
                expires_at: k.expires_at.clone(),
            })
            .collect()
    }
}

/// Public-safe key info (no hash).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub id: String,
    pub owner: String,
    pub scopes: Vec<String>,
    pub active: bool,
    pub created_at: String,
    pub expires_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn generate_and_authenticate() {
        let store = ApiKeyStore::new();
        let (key, raw) = ApiKey::generate("test-owner", vec!["read".into()]);
        store.upsert(key).await;
        let found = store.authenticate(&raw).await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().owner, "test-owner");
    }

    #[tokio::test]
    async fn revoke_key() {
        let store = ApiKeyStore::new();
        let (key, raw) = ApiKey::generate("owner", vec![]);
        let id = key.id.clone();
        store.upsert(key).await;
        store.revoke(&id).await;
        assert!(store.authenticate(&raw).await.is_none());
    }
}
