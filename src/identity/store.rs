//! Identity Context Store
//!
//! Manages identity storage and retrieval with caching

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use tracing::debug;

use crate::error::Result;
use super::types::{Identity, IdentityId, IdentityContext, AuthMethod};

/// Identity store configuration
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IdentityStoreConfig {
    /// Cache TTL in seconds
    pub cache_ttl_secs: u64,
    /// Maximum cache size
    pub max_cache_size: usize,
}

impl Default for IdentityStoreConfig {
    fn default() -> Self {
        Self {
            cache_ttl_secs: 3600,
            max_cache_size: 10_000,
        }
    }
}

/// Identity Store
pub struct IdentityStore {
    config: IdentityStoreConfig,
    identities: tokio::sync::RwLock<HashMap<String, CachedIdentity>>,
    contexts: tokio::sync::RwLock<HashMap<String, IdentityContext>>,
}

/// Cached identity with TTL
#[derive(Clone, Debug)]
struct CachedIdentity {
    identity: Identity,
    cached_at: DateTime<Utc>,
}

impl IdentityStore {
    /// Create a new identity store
    pub async fn new(config: IdentityStoreConfig) -> Result<Self> {
        debug!("Initializing Identity Store");
        Ok(Self {
            config,
            identities: tokio::sync::RwLock::new(HashMap::new()),
            contexts: tokio::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Store identity
    pub async fn store_identity(&self, identity: Identity) -> Result<()> {
        debug!("Storing identity: {}", identity.id.0);

        let mut identities = self.identities.write().await;

        // Check cache size
        if identities.len() >= self.config.max_cache_size {
            // Remove oldest entry
            if let Some(oldest_key) = identities
                .iter()
                .min_by_key(|(_, v)| v.cached_at)
                .map(|(k, _)| k.clone())
            {
                identities.remove(&oldest_key);
            }
        }

        identities.insert(
            identity.id.0.clone(),
            CachedIdentity {
                identity,
                cached_at: Utc::now(),
            },
        );

        Ok(())
    }

    /// Get identity
    pub async fn get_identity(&self, identity_id: &IdentityId) -> Result<Option<Identity>> {
        let identities = self.identities.read().await;

        if let Some(cached) = identities.get(&identity_id.0) {
            let age = Utc::now() - cached.cached_at;
            if age.num_seconds() as u64 <= self.config.cache_ttl_secs {
                return Ok(Some(cached.identity.clone()));
            }
        }

        Ok(None)
    }

    /// Delete identity
    pub async fn delete_identity(&self, identity_id: &IdentityId) -> Result<()> {
        debug!("Deleting identity: {}", identity_id.0);

        let mut identities = self.identities.write().await;
        identities.remove(&identity_id.0);

        Ok(())
    }

    /// Store identity context
    pub async fn store_context(&self, context_id: String, context: IdentityContext) -> Result<()> {
        debug!("Storing identity context: {}", context_id);

        let mut contexts = self.contexts.write().await;
        contexts.insert(context_id, context);

        Ok(())
    }

    /// Get identity context
    pub async fn get_context(&self, context_id: &str) -> Result<Option<IdentityContext>> {
        let contexts = self.contexts.read().await;
        Ok(contexts.get(context_id).cloned())
    }

    /// Delete identity context
    pub async fn delete_context(&self, context_id: &str) -> Result<()> {
        debug!("Deleting identity context: {}", context_id);

        let mut contexts = self.contexts.write().await;
        contexts.remove(context_id);

        Ok(())
    }

    /// List all identities
    pub async fn list_identities(&self) -> Result<Vec<Identity>> {
        let identities = self.identities.read().await;
        Ok(identities
            .values()
            .map(|c| c.identity.clone())
            .collect())
    }

    /// Clear expired entries
    pub async fn cleanup_expired(&self) -> Result<usize> {
        debug!("Cleaning up expired identities");

        let mut identities = self.identities.write().await;
        let initial_count = identities.len();

        identities.retain(|_, cached| {
            let age = Utc::now() - cached.cached_at;
            age.num_seconds() as u64 <= self.config.cache_ttl_secs
        });

        let removed = initial_count - identities.len();
        debug!("Cleaned up {} expired identities", removed);

        Ok(removed)
    }

    /// Get store statistics
    pub async fn get_statistics(&self) -> Result<StoreStatistics> {
        let identities = self.identities.read().await;
        let contexts = self.contexts.read().await;

        Ok(StoreStatistics {
            total_identities: identities.len(),
            total_contexts: contexts.len(),
            cache_size_bytes: std::mem::size_of_val(&*identities),
            timestamp: Utc::now(),
        })
    }
}

/// Store statistics
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoreStatistics {
    pub total_identities: usize,
    pub total_contexts: usize,
    pub cache_size_bytes: usize,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_identity_store_creation() {
        let config = IdentityStoreConfig::default();
        let store = IdentityStore::new(config).await.unwrap();
        assert_eq!(store.config.cache_ttl_secs, 3600);
    }

    #[tokio::test]
    async fn test_store_and_retrieve_identity() {
        let config = IdentityStoreConfig::default();
        let store = IdentityStore::new(config).await.unwrap();

        let identity = Identity::new(
            IdentityId::new("user123"),
            "google".to_string(),
            "user@example.com".to_string(),
        );

        store.store_identity(identity.clone()).await.unwrap();
        let retrieved = store.get_identity(&identity.id).await.unwrap();

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, identity.id);
    }

    #[tokio::test]
    async fn test_delete_identity() {
        let config = IdentityStoreConfig::default();
        let store = IdentityStore::new(config).await.unwrap();

        let identity = Identity::new(
            IdentityId::new("user123"),
            "google".to_string(),
            "user@example.com".to_string(),
        );

        store.store_identity(identity.clone()).await.unwrap();
        store.delete_identity(&identity.id).await.unwrap();

        let retrieved = store.get_identity(&identity.id).await.unwrap();
        assert!(retrieved.is_none());
    }
}
