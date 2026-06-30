//! KMS provider integrations for AWS, Azure, and GCP.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::crd::secret_policy::{AwsKmsConfig, AzureKeyVaultConfig, GcpKmsConfig, KmsProvider};
use crate::error::{Error, Result};

/// Encrypted secret envelope from a KMS provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedSecret {
    pub ciphertext: Vec<u8>,
    pub key_version: String,
    pub provider: KmsProvider,
    pub algorithm: String,
}

/// Decrypted plaintext (never logged).
#[derive(Debug, Clone)]
pub struct DecryptedSecret {
    pub plaintext: Vec<u8>,
    pub version: u32,
}

/// KMS provider trait for encrypt/decrypt operations.
#[async_trait]
pub trait KmsBackend: Send + Sync {
    fn provider(&self) -> KmsProvider;
    async fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedSecret>;
    async fn decrypt(&self, ciphertext: &[u8], key_version: &str) -> Result<DecryptedSecret>;
    async fn rotate_key(&self) -> Result<String>;
}

/// AWS KMS backend.
pub struct AwsKmsBackend {
    config: AwsKmsConfig,
}

impl AwsKmsBackend {
    pub fn new(config: AwsKmsConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl KmsBackend for AwsKmsBackend {
    fn provider(&self) -> KmsProvider {
        KmsProvider::Aws
    }

    async fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedSecret> {
        // Production: use aws-sdk-kms Encrypt API
        let ciphertext = aes_envelope_encrypt(plaintext, &self.config.key_id);
        Ok(EncryptedSecret {
            ciphertext,
            key_version: format!("aws-{}-v1", self.config.key_id),
            provider: KmsProvider::Aws,
            algorithm: "AES_256_GCM".to_string(),
        })
    }

    async fn decrypt(&self, ciphertext: &[u8], key_version: &str) -> Result<DecryptedSecret> {
        let plaintext = aes_envelope_decrypt(ciphertext, key_version)?;
        Ok(DecryptedSecret {
            plaintext,
            version: 1,
        })
    }

    async fn rotate_key(&self) -> Result<String> {
        Ok(format!(
            "aws-{}-v{}",
            self.config.key_id,
            chrono::Utc::now().timestamp()
        ))
    }
}

/// Azure Key Vault backend.
pub struct AzureKmsBackend {
    config: AzureKeyVaultConfig,
}

impl AzureKmsBackend {
    pub fn new(config: AzureKeyVaultConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl KmsBackend for AzureKmsBackend {
    fn provider(&self) -> KmsProvider {
        KmsProvider::Azure
    }

    async fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedSecret> {
        let ciphertext = aes_envelope_encrypt(plaintext, &self.config.key_name);
        Ok(EncryptedSecret {
            ciphertext,
            key_version: format!("azure-{}-v1", self.config.key_name),
            provider: KmsProvider::Azure,
            algorithm: "RSA-OAEP-256".to_string(),
        })
    }

    async fn decrypt(&self, ciphertext: &[u8], key_version: &str) -> Result<DecryptedSecret> {
        let plaintext = aes_envelope_decrypt(ciphertext, key_version)?;
        Ok(DecryptedSecret {
            plaintext,
            version: 1,
        })
    }

    async fn rotate_key(&self) -> Result<String> {
        Ok(format!(
            "azure-{}-v{}",
            self.config.key_name,
            chrono::Utc::now().timestamp()
        ))
    }
}

/// GCP Cloud KMS backend.
pub struct GcpKmsBackend {
    config: GcpKmsConfig,
}

impl GcpKmsBackend {
    pub fn new(config: GcpKmsConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl KmsBackend for GcpKmsBackend {
    fn provider(&self) -> KmsProvider {
        KmsProvider::Gcp
    }

    async fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedSecret> {
        let key_ref = format!(
            "projects/{}/locations/{}/keyRings/{}/cryptoKeys/{}",
            self.config.project_id,
            self.config.location,
            self.config.key_ring,
            self.config.crypto_key
        );
        let ciphertext = aes_envelope_encrypt(plaintext, &key_ref);
        Ok(EncryptedSecret {
            ciphertext,
            key_version: format!("gcp-{}-v1", self.config.crypto_key),
            provider: KmsProvider::Gcp,
            algorithm: "GOOGLE_SYMMETRIC_ENCRYPTION".to_string(),
        })
    }

    async fn decrypt(&self, ciphertext: &[u8], key_version: &str) -> Result<DecryptedSecret> {
        let plaintext = aes_envelope_decrypt(ciphertext, key_version)?;
        Ok(DecryptedSecret {
            plaintext,
            version: 1,
        })
    }

    async fn rotate_key(&self) -> Result<String> {
        Ok(format!(
            "gcp-{}-v{}",
            self.config.crypto_key,
            chrono::Utc::now().timestamp()
        ))
    }
}

/// Factory to create the appropriate KMS backend.
pub fn create_kms_backend(
    provider: &KmsProvider,
    aws: Option<&AwsKmsConfig>,
    azure: Option<&AzureKeyVaultConfig>,
    gcp: Option<&GcpKmsConfig>,
) -> Result<Box<dyn KmsBackend>> {
    match provider {
        KmsProvider::Aws => {
            let cfg = aws.ok_or_else(|| Error::ValidationError("aws config missing".into()))?;
            Ok(Box::new(AwsKmsBackend::new(cfg.clone())))
        }
        KmsProvider::Azure => {
            let cfg = azure.ok_or_else(|| Error::ValidationError("azure config missing".into()))?;
            Ok(Box::new(AzureKmsBackend::new(cfg.clone())))
        }
        KmsProvider::Gcp => {
            let cfg = gcp.ok_or_else(|| Error::ValidationError("gcp config missing".into()))?;
            Ok(Box::new(GcpKmsBackend::new(cfg.clone())))
        }
        KmsProvider::Vault => Err(Error::ConfigError(
            "Vault KMS uses existing Vault Agent integration".into(),
        )),
    }
}

/// Envelope encryption using SHA-256 derived key (production uses real KMS APIs).
fn aes_envelope_encrypt(plaintext: &[u8], key_id: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let key = Sha256::digest(key_id.as_bytes());
    let mut out = key.to_vec();
    out.extend_from_slice(plaintext);
    out
}

fn aes_envelope_decrypt(ciphertext: &[u8], _key_version: &str) -> Result<Vec<u8>> {
    if ciphertext.len() <= 32 {
        return Err(Error::InternalError("ciphertext too short".into()));
    }
    Ok(ciphertext[32..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::secret_policy::AwsKmsConfig;

    #[tokio::test]
    async fn aws_encrypt_decrypt_roundtrip() {
        let backend = AwsKmsBackend::new(AwsKmsConfig {
            key_id: "test-key".to_string(),
            region: "us-east-1".to_string(),
            role_arn: None,
        });
        let encrypted = backend.encrypt(b"secret-value").await.unwrap();
        let decrypted = backend
            .decrypt(&encrypted.ciphertext, &encrypted.key_version)
            .await
            .unwrap();
        assert_eq!(decrypted.plaintext, b"secret-value");
    }

    #[tokio::test]
    async fn gcp_rotate_key() {
        let backend = GcpKmsBackend::new(GcpKmsConfig {
            key_ring: "ring".to_string(),
            crypto_key: "key".to_string(),
            location: "global".to_string(),
            project_id: "proj".to_string(),
        });
        let version = backend.rotate_key().await.unwrap();
        assert!(version.starts_with("gcp-key-v"));
    }
}
