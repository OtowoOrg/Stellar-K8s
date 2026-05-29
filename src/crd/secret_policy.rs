//! SecretPolicy CRD for advanced secret management with external KMS integration.

use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum KmsProvider {
    Aws,
    Azure,
    Gcp,
    Vault,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AwsKmsConfig {
    pub key_id: String,
    pub region: String,
    #[serde(default)]
    pub role_arn: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AzureKeyVaultConfig {
    pub vault_url: String,
    pub key_name: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GcpKmsConfig {
    pub key_ring: String,
    pub crypto_key: String,
    pub location: String,
    pub project_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RotationPolicy {
    #[serde(default = "default_rotation_interval")]
    pub interval: String,
    #[serde(default = "default_true")]
    pub zero_downtime: bool,
    #[serde(default = "default_version_retention")]
    pub version_retention: u32,
}

fn default_rotation_interval() -> String {
    "720h".to_string()
}
fn default_true() -> bool {
    true
}
fn default_version_retention() -> u32 {
    5
}

impl Default for RotationPolicy {
    fn default() -> Self {
        Self {
            interval: default_rotation_interval(),
            zero_downtime: true,
            version_retention: default_version_retention(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretPolicySyncConfig {
    pub target_clusters: Vec<String>,
    #[serde(default = "default_sync_interval")]
    pub sync_interval: String,
    #[serde(default)]
    pub conflict_resolution: SyncConflictResolution,
}

fn default_sync_interval() -> String {
    "5m".to_string()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SyncConflictResolution {
    #[default]
    PrimaryWins,
    LastWriteWins,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretAuditConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub sink: Option<String>,
    #[serde(default = "default_true")]
    pub anomaly_detection: bool,
}

impl Default for SecretAuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sink: None,
            anomaly_detection: true,
        }
    }
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "SecretPolicy",
    namespaced,
    status = "SecretPolicyStatus",
    printcolumn = r#"{"name":"Provider", "type":"string", "jsonPath":".spec.provider"}"#,
    printcolumn = r#"{"name":"Phase", "type":"string", "jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Version", "type":"integer", "jsonPath":".status.currentVersion"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct SecretPolicySpec {
    pub secret_name: String,
    pub provider: KmsProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws: Option<AwsKmsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azure: Option<AzureKeyVaultConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcp: Option<GcpKmsConfig>,
    #[serde(default)]
    pub rotation: RotationPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SecretPolicySyncConfig>,
    #[serde(default)]
    pub audit: SecretAuditConfig,
    #[serde(default = "default_true")]
    pub encrypt_in_transit: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretPolicyStatus {
    pub phase: SecretPolicyPhase,
    pub current_version: u32,
    pub last_rotation: Option<DateTime<Utc>>,
    pub last_sync: Option<DateTime<Utc>>,
    pub conditions: Vec<SecretPolicyCondition>,
    #[serde(default)]
    pub audit_entries_count: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SecretPolicyPhase {
    #[default]
    Pending,
    Active,
    Rotating,
    Syncing,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretPolicyCondition {
    #[serde(rename = "type")]
    pub type_: String,
    pub status: String,
    pub last_transition_time: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl SecretPolicySpec {
    pub fn validate(&self) -> Result<(), String> {
        match self.provider {
            KmsProvider::Aws if self.aws.is_none() => {
                Err("aws config required when provider=aws".to_string())
            }
            KmsProvider::Azure if self.azure.is_none() => {
                Err("azure config required when provider=azure".to_string())
            }
            KmsProvider::Gcp if self.gcp.is_none() => {
                Err("gcp config required when provider=gcp".to_string())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_provider_config() {
        let spec = SecretPolicySpec {
            secret_name: "s".to_string(),
            provider: KmsProvider::Aws,
            aws: Some(AwsKmsConfig {
                key_id: "key".to_string(),
                region: "us-east-1".to_string(),
                role_arn: None,
            }),
            azure: None,
            gcp: None,
            rotation: RotationPolicy::default(),
            sync: None,
            audit: SecretAuditConfig::default(),
            encrypt_in_transit: true,
        };
        assert!(spec.validate().is_ok());
    }
}
