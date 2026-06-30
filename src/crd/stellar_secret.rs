//! StellarSecret Custom Resource Definition
//!
//! Advanced secret management with dynamic credential generation, automatic rotation,
//! KMS encryption, multi-backend support, audit logging, and zero-downtime rotation.

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::secret_policy::{AwsKmsConfig, AzureKeyVaultConfig, GcpKmsConfig, KmsProvider};
use super::types::Condition;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SecretBackend {
    Vault,
    Aws,
    Azure,
    Local,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DynamicSecretConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_ttl")]
    pub ttl: String,
    #[serde(default)]
    pub database: Option<DatabaseCredentialTarget>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseCredentialTarget {
    pub host: String,
    pub database: String,
    pub username: String,
    pub role: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RotationConfig {
    #[serde(default = "default_rotation_interval")]
    pub interval: String,
    #[serde(default = "default_true")]
    pub zero_downtime: bool,
    #[serde(default = "default_version_retention")]
    pub version_retention: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretInjectionTarget {
    pub kind: InjectionKind,
    pub name: String,
    pub key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InjectionKind {
    EnvVar,
    File,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretAuditPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub sink: Option<String>,
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "StellarSecret",
    namespaced,
    status = "StellarSecretStatus",
    shortname = "ssec",
    printcolumn = r#"{"name":"Backend","type":"string","jsonPath":".spec.backend"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Version","type":"integer","jsonPath":".status.currentVersion"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct StellarSecretSpec {
    pub secret_name: String,
    pub backend: SecretBackend,
    #[serde(default)]
    pub provider: Option<KmsProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws: Option<AwsKmsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azure: Option<AzureKeyVaultConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcp: Option<GcpKmsConfig>,
    #[serde(default)]
    pub dynamic: DynamicSecretConfig,
    #[serde(default)]
    pub rotation: RotationConfig,
    #[serde(default)]
    pub targets: Vec<SecretInjectionTarget>,
    #[serde(default)]
    pub audit: SecretAuditPolicy,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StellarSecretStatus {
    pub phase: SecretPhase,
    pub current_version: u32,
    #[serde(default)]
    pub versions: Vec<SecretVersionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rotation: Option<DateTime<Utc>>,
    #[serde(default)]
    pub audit_entries_count: u64,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SecretPhase {
    #[default]
    Pending,
    Active,
    Rotating,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretVersionRecord {
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub active: bool,
}

fn default_true() -> bool {
    true
}
fn default_ttl() -> String {
    "1h".to_string()
}
fn default_rotation_interval() -> String {
    "720h".to_string()
}
fn default_version_retention() -> u32 {
    5
}

impl StellarSecretSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.secret_name.is_empty() {
            return Err("secret_name is required".to_string());
        }
        match self.backend {
            SecretBackend::Aws if self.aws.is_none() => {
                Err("aws config required when backend=aws".to_string())
            }
            SecretBackend::Azure if self.azure.is_none() => {
                Err("azure config required when backend=azure".to_string())
            }
            _ => Ok(()),
        }
    }

    pub fn rotation_interval_days(&self) -> u32 {
        if self.rotation.interval.ends_with('h') {
            self.rotation
                .interval
                .trim_end_matches('h')
                .parse::<u32>()
                .map(|h| h / 24)
                .unwrap_or(30)
        } else {
            30
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_vault_backend() {
        let spec = StellarSecretSpec {
            secret_name: "db-creds".into(),
            backend: SecretBackend::Vault,
            provider: Some(KmsProvider::Vault),
            aws: None,
            azure: None,
            gcp: None,
            dynamic: DynamicSecretConfig::default(),
            rotation: RotationConfig::default(),
            targets: vec![],
            audit: SecretAuditPolicy::default(),
        };
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn rotation_interval_defaults_to_30_days() {
        let spec = StellarSecretSpec {
            secret_name: "s".into(),
            backend: SecretBackend::Local,
            provider: None,
            aws: None,
            azure: None,
            gcp: None,
            dynamic: DynamicSecretConfig::default(),
            rotation: RotationConfig::default(),
            targets: vec![],
            audit: SecretAuditPolicy::default(),
        };
        assert_eq!(spec.rotation_interval_days(), 30);
    }
}
