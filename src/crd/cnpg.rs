use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// CloudNativePG Cluster Custom Resource
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "postgresql.cnpg.io",
    version = "v1",
    kind = "Cluster",
    namespaced
)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSpec {
    pub instances: i32,
    pub image_name: Option<String>,
    pub postgresql: Option<PostgresConfiguration>,
    pub storage: StorageConfiguration,
    pub backup: Option<BackupConfiguration>,
    pub bootstrap: Option<BootstrapConfiguration>,
    pub monitoring: Option<MonitoringConfiguration>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct PostgresConfiguration {
    pub parameters: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct StorageConfiguration {
    pub size: String,
    pub storage_class: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackupConfiguration {
    pub barman_object_store: Option<BarmanObjectStore>,
    pub retention_policy: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BarmanObjectStore {
    pub destination_path: String,
    pub endpoint_u_r_l: Option<String>,
    pub s3_credentials: Option<S3Credentials>,
    pub azure_credentials: Option<AzureCredentials>,
    pub google_credentials: Option<GoogleCredentials>,
    pub wal: Option<WalBackupConfiguration>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WalBackupConfiguration {
    pub compression: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct S3Credentials {
    pub access_key_id: SecretKeySelector,
    pub secret_access_key: SecretKeySelector,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct AzureCredentials {
    pub connection_string: SecretKeySelector,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct GoogleCredentials {
    pub application_credentials: SecretKeySelector,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct SecretKeySelector {
    pub name: String,
    pub key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapConfiguration {
    pub initdb: Option<InitDbConfiguration>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct InitDbConfiguration {
    pub database: String,
    pub owner: String,
    pub secret: Option<SecretSelector>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct SecretSelector {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct MonitoringConfiguration {
    pub enable_pod_monitor: bool,
}

/// CloudNativePG Pooler Custom Resource
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "postgresql.cnpg.io",
    version = "v1",
    kind = "Pooler",
    namespaced
)]
#[serde(rename_all = "camelCase")]
pub struct PoolerSpec {
    pub cluster: PoolerCluster,
    pub instances: i32,
    #[serde(rename = "type")]
    pub type_: String, // pgbouncer
    pub pgbouncer: PgBouncerSpec,
    pub monitoring: Option<MonitoringConfiguration>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct PoolerCluster {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PgBouncerSpec {
    pub pool_mode: String,
    pub parameters: BTreeMap<String, String>,
}
