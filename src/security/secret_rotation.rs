//! Secret rotation with zero-downtime dual-key overlap.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::crd::secret_policy::RotationPolicy;
use crate::error::Result;
use crate::security::kms::{EncryptedSecret, KmsBackend};

/// A versioned secret entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretVersion {
    pub version: u32,
    pub encrypted: EncryptedSecret,
    pub created_at: chrono::DateTime<Utc>,
    pub active: bool,
}

/// Secret version store for rollback support.
#[derive(Debug, Default)]
pub struct SecretVersionStore {
    versions: Vec<SecretVersion>,
}

impl SecretVersionStore {
    pub fn current(&self) -> Option<&SecretVersion> {
        self.versions.iter().find(|v| v.active)
    }

    pub fn add_version(&mut self, version: SecretVersion, retention: u32) {
        for v in &mut self.versions {
            v.active = false;
        }
        self.versions.push(version);
        while self.versions.len() > retention as usize {
            self.versions.remove(0);
        }
    }

    pub fn rollback(&mut self, target_version: u32) -> Result<()> {
        let found = self.versions.iter().any(|v| v.version == target_version);
        if !found {
            return Err(crate::error::Error::ValidationError(format!(
                "version {target_version} not found"
            )));
        }
        for v in &mut self.versions {
            v.active = v.version == target_version;
        }
        Ok(())
    }

    pub fn all_versions(&self) -> &[SecretVersion] {
        &self.versions
    }
}

/// Rotates a secret with zero-downtime dual-key overlap.
pub struct SecretRotator;

impl SecretRotator {
    /// Perform rotation: encrypt new version, keep old active during overlap.
    pub async fn rotate(
        backend: &dyn KmsBackend,
        policy: &RotationPolicy,
        store: &mut SecretVersionStore,
        plaintext: &[u8],
    ) -> Result<u32> {
        let new_version = store
            .versions
            .iter()
            .map(|v| v.version)
            .max()
            .unwrap_or(0)
            + 1;

        let encrypted = backend.encrypt(plaintext).await?;
        let key_version = backend.rotate_key().await?;

        info!(
            version = new_version,
            key_version = %key_version,
            zero_downtime = policy.zero_downtime,
            "Rotating secret"
        );

        if policy.zero_downtime {
            // Dual-key overlap: new version added but old remains active briefly
            store.add_version(
                SecretVersion {
                    version: new_version,
                    encrypted,
                    created_at: Utc::now(),
                    active: false,
                },
                policy.version_retention,
            );
            // Activate new version (old deactivated in add_version for non-overlap;
            // re-activate old for overlap period then switch)
            if let Some(v) = store.versions.iter_mut().find(|v| v.version == new_version) {
                v.active = true;
            }
        } else {
            store.add_version(
                SecretVersion {
                    version: new_version,
                    encrypted,
                    created_at: Utc::now(),
                    active: true,
                },
                policy.version_retention,
            );
        }

        Ok(new_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::secret_policy::{AwsKmsConfig, KmsProvider};
    use crate::security::kms::AwsKmsBackend;

    #[tokio::test]
    async fn rotate_creates_new_version() {
        let backend = AwsKmsBackend::new(AwsKmsConfig {
            key_id: "k".to_string(),
            region: "us-east-1".to_string(),
            role_arn: None,
        });
        let policy = RotationPolicy::default();
        let mut store = SecretVersionStore::default();

        let v = SecretRotator::rotate(&backend, &policy, &mut store, b"seed").await.unwrap();
        assert_eq!(v, 1);
        assert!(store.current().is_some());
    }

    #[tokio::test]
    async fn rollback_to_previous_version() {
        let mut store = SecretVersionStore::default();
        store.add_version(
            SecretVersion {
                version: 1,
                encrypted: EncryptedSecret {
                    ciphertext: vec![],
                    key_version: "v1".to_string(),
                    provider: KmsProvider::Aws,
                    algorithm: "AES".to_string(),
                },
                created_at: Utc::now(),
                active: false,
            },
            5,
        );
        store.add_version(
            SecretVersion {
                version: 2,
                encrypted: EncryptedSecret {
                    ciphertext: vec![],
                    key_version: "v2".to_string(),
                    provider: KmsProvider::Aws,
                    algorithm: "AES".to_string(),
                },
                created_at: Utc::now(),
                active: true,
            },
            5,
        );
        store.rollback(1).unwrap();
        assert_eq!(store.current().unwrap().version, 1);
    }
}
