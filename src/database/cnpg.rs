use kube::{Api, Client, ResourceExt};
use kube::runtime::controller::Action;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CNPGCluster {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta,
    pub spec: CNPGClusterSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CNPGClusterSpec {
    pub instances: i32,
    pub storage: StorageConfiguration,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<Bootstrap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<BackupConfiguration>,
    pub postgresql: PostgreSQLConfiguration,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitoring: Option<MonitoringConfiguration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfiguration {
    pub size: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "storageClass")]
    pub storage_class: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bootstrap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initdb: Option<InitDB>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<Recovery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitDB {
    pub database: String,
    pub owner: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<SecretKeySelector>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recovery {
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretKeySelector {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConfiguration {
    #[serde(rename = "barmanObjectStore")]
    pub barman_object_store: BarmanObjectStore,
    #[serde(rename = "retentionPolicy")]
    pub retention_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarmanObjectStore {
    #[serde(rename = "destinationPath")]
    pub destination_path: String,
    #[serde(rename = "endpointURL")]
    pub endpoint_url: String,
    pub s3_credentials: S3Credentials,
    pub wal: WalConfiguration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Credentials {
    #[serde(rename = "accessKeyId")]
    pub access_key_id: SecretKeySelector,
    #[serde(rename = "secretAccessKey")]
    pub secret_access_key: SecretKeySelector,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalConfiguration {
    pub compression: String,
    pub encryption: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgreSQLConfiguration {
    pub parameters: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfiguration {
    #[serde(rename = "enablePodMonitor")]
    pub enable_pod_monitor: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pooler {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta,
    pub spec: PoolerSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolerSpec {
    pub cluster: ClusterRef,
    pub instances: i32,
    #[serde(rename = "type")]
    pub pooler_type: String,
    pub pgbouncer: PgBouncerConfiguration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRef {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PgBouncerConfiguration {
    #[serde(rename = "poolMode")]
    pub pool_mode: String,
    pub parameters: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledBackup {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta,
    pub spec: ScheduledBackupSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledBackupSpec {
    pub schedule: String,
    #[serde(rename = "backupOwnerReference")]
    pub backup_owner_reference: String,
    pub cluster: ClusterRef,
}

pub struct CNPGManager {
    client: Client,
}

impl CNPGManager {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn reconcile_database(
        &self,
        name: &str,
        namespace: &str,
        db_config: &DatabaseConfig,
    ) -> Result<()> {
        // Create PostgreSQL Cluster
        self.create_cluster(name, namespace, db_config).await?;

        // Create Pooler if enabled
        if db_config.pooler.enabled {
            self.create_pooler(name, namespace, &db_config.pooler).await?;
        }

        // Create Scheduled Backup if enabled
        if db_config.backup.enabled {
            self.create_scheduled_backup(name, namespace, &db_config.backup).await?;
        }

        Ok(())
    }

    async fn create_cluster(
        &self,
        name: &str,
        namespace: &str,
        db_config: &DatabaseConfig,
    ) -> Result<()> {
        let cluster = CNPGCluster {
            api_version: "postgresql.cnpg.io/v1".to_string(),
            kind: "Cluster".to_string(),
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(format!("{}-db", name)),
                namespace: Some(namespace.to_string()),
                labels: Some(std::collections::BTreeMap::from([
                    ("app".to_string(), name.to_string()),
                    ("component".to_string(), "database".to_string()),
                ])),
                ..Default::default()
            },
            spec: CNPGClusterSpec {
                instances: db_config.instances,
                storage: StorageConfiguration {
                    size: db_config.storage.size.clone(),
                    storage_class: db_config.storage.storage_class.clone(),
                },
                bootstrap: Some(Bootstrap {
                    initdb: Some(InitDB {
                        database: format!("{}_db", name),
                        owner: format!("{}_user", name),
                        secret: Some(SecretKeySelector {
                            name: format!("{}-db-credentials", name),
                        }),
                    }),
                    recovery: None,
                }),
                backup: if db_config.backup.enabled {
                    Some(BackupConfiguration {
                        barman_object_store: BarmanObjectStore {
                            destination_path: format!(
                                "s3://{}/{}",
                                db_config.backup.s3.as_ref().unwrap().bucket,
                                name
                            ),
                            endpoint_url: db_config.backup.s3.as_ref().unwrap().endpoint_url.clone(),
                            s3_credentials: S3Credentials {
                                access_key_id: SecretKeySelector {
                                    name: db_config.backup.s3.as_ref().unwrap()
                                        .credentials.secret_name.clone(),
                                },
                                secret_access_key: SecretKeySelector {
                                    name: db_config.backup.s3.as_ref().unwrap()
                                        .credentials.secret_name.clone(),
                                },
                                region: Some(db_config.backup.s3.as_ref().unwrap().region.clone()),
                            },
                            wal: WalConfiguration {
                                compression: "gzip".to_string(),
                                encryption: "AES256".to_string(),
                            },
                        },
                        retention_policy: db_config.backup.retention_policy.clone(),
                    })
                } else {
                    None
                },
                postgresql: PostgreSQLConfiguration {
                    parameters: std::collections::HashMap::from([
                        ("max_connections".to_string(), "200".to_string()),
                        ("shared_buffers".to_string(), "256MB".to_string()),
                        ("effective_cache_size".to_string(), "1GB".to_string()),
                        ("maintenance_work_mem".to_string(), "64MB".to_string()),
                        ("checkpoint_completion_target".to_string(), "0.9".to_string()),
                        ("wal_buffers".to_string(), "16MB".to_string()),
                        ("default_statistics_target".to_string(), "100".to_string()),
                        ("random_page_cost".to_string(), "1.1".to_string()),
                        ("effective_io_concurrency".to_string(), "200".to_string()),
                        ("work_mem".to_string(), "4MB".to_string()),
                        ("min_wal_size".to_string(), "1GB".to_string()),
                        ("max_wal_size".to_string(), "4GB".to_string()),
                    ]),
                },
                monitoring: Some(MonitoringConfiguration {
                    enable_pod_monitor: true,
                }),
            },
        };

        let api: Api<CNPGCluster> = Api::namespaced(self.client.clone(), namespace);
        api.create(&Default::default(), &cluster).await?;

        Ok(())
    }

    async fn create_pooler(
        &self,
        name: &str,
        namespace: &str,
        pooler_config: &PoolerConfig,
    ) -> Result<()> {
        let pooler = Pooler {
            api_version: "postgresql.cnpg.io/v1".to_string(),
            kind: "Pooler".to_string(),
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(format!("{}-pooler", name)),
                namespace: Some(namespace.to_string()),
                labels: Some(std::collections::BTreeMap::from([
                    ("app".to_string(), name.to_string()),
                    ("component".to_string(), "pooler".to_string()),
                ])),
                ..Default::default()
            },
            spec: PoolerSpec {
                cluster: ClusterRef {
                    name: format!("{}-db", name),
                },
                instances: pooler_config.instances,
                pooler_type: "rw".to_string(),
                pgbouncer: PgBouncerConfiguration {
                    pool_mode: pooler_config.pool_mode.clone(),
                    parameters: std::collections::HashMap::from([
                        ("max_client_conn".to_string(), pooler_config.max_client_conn.to_string()),
                        ("default_pool_size".to_string(), "25".to_string()),
                        ("reserve_pool_size".to_string(), "5".to_string()),
                    ]),
                },
            },
        };

        let api: Api<Pooler> = Api::namespaced(self.client.clone(), namespace);
        api.create(&Default::default(), &pooler).await?;

        Ok(())
    }

    async fn create_scheduled_backup(
        &self,
        name: &str,
        namespace: &str,
        backup_config: &BackupConfig,
    ) -> Result<()> {
        let scheduled_backup = ScheduledBackup {
            api_version: "postgresql.cnpg.io/v1".to_string(),
            kind: "ScheduledBackup".to_string(),
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(format!("{}-backup", name)),
                namespace: Some(namespace.to_string()),
                labels: Some(std::collections::BTreeMap::from([
                    ("app".to_string(), name.to_string()),
                    ("component".to_string(), "backup".to_string()),
                ])),
                ..Default::default()
            },
            spec: ScheduledBackupSpec {
                schedule: backup_config.schedule.clone(),
                backup_owner_reference: "self".to_string(),
                cluster: ClusterRef {
                    name: format!("{}-db", name),
                },
            },
        };

        let api: Api<ScheduledBackup> = Api::namespaced(self.client.clone(), namespace);
        api.create(&Default::default(), &scheduled_backup).await?;

        Ok(())
    }
}

// Configuration structs
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub instances: i32,
    pub storage: StorageConfig,
    pub backup: BackupConfig,
    pub pooler: PoolerConfig,
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub size: String,
    pub storage_class: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BackupConfig {
    pub enabled: bool,
    pub retention_policy: String,
    pub schedule: String,
    pub s3: Option<S3Config>,
}

#[derive(Debug, Clone)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    pub endpoint_url: String,
    pub credentials: S3CredentialsConfig,
}

#[derive(Debug, Clone)]
pub struct S3CredentialsConfig {
    pub secret_name: String,
}

#[derive(Debug, Clone)]
pub struct PoolerConfig {
    pub enabled: bool,
    pub instances: i32,
    pub pool_mode: String,
    pub max_client_conn: i32,
}
