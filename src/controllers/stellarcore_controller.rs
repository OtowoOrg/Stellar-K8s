use crate::database::cnpg::{CNPGManager, DatabaseConfig, StorageConfig, BackupConfig, PoolerConfig, S3Config, S3CredentialsConfig};

pub async fn reconcile(
    stellar_core: Arc,
    ctx: Arc,
) -> Result {
    let client = ctx.client.clone();
    let namespace = stellar_core.namespace().unwrap();
    let name = stellar_core.name_any();

    // Reconcile database
    if let Some(db_spec) = &stellar_core.spec.database {
        if db_spec.r#type == "cloudnativepg" {
            let cnpg_manager = CNPGManager::new(client.clone());
            
            let db_config = DatabaseConfig {
                instances: db_spec.instances.unwrap_or(3),
                storage: StorageConfig {
                    size: db_spec.storage.as_ref()
                        .map(|s| s.size.clone())
                        .unwrap_or_else(|| "10Gi".to_string()),
                    storage_class: db_spec.storage.as_ref()
                        .and_then(|s| s.storage_class.clone()),
                },
                backup: BackupConfig {
                    enabled: db_spec.backup.as_ref()
                        .map(|b| b.enabled)
                        .unwrap_or(true),
                    retention_policy: db_spec.backup.as_ref()
                        .map(|b| b.retention_policy.clone())
                        .unwrap_or_else(|| "30d".to_string()),
                    schedule: db_spec.backup.as_ref()
                        .map(|b| b.schedule.clone())
                        .unwrap_or_else(|| "0 0 * * *".to_string()),
                    s3: db_spec.backup.as_ref()
                        .and_then(|b| b.s3.as_ref())
                        .map(|s3| S3Config {
                            bucket: s3.bucket.clone(),
                            region: s3.region.clone(),
                            endpoint_url: s3.endpoint_url.clone().unwrap_or_default(),
                            credentials: S3CredentialsConfig {
                                secret_name: s3.credentials.as_ref()
                                    .map(|c| c.secret_name.clone())
                                    .unwrap_or_else(|| "s3-credentials".to_string()),
                            },
                        }),
                },
                pooler: PoolerConfig {
                    enabled: db_spec.pooler.as_ref()
                        .map(|p| p.enabled)
                        .unwrap_or(true),
                    instances: db_spec.pooler.as_ref()
                        .map(|p| p.instances)
                        .unwrap_or(2),
                    pool_mode: db_spec.pooler.as_ref()
                        .map(|p| p.pool_mode.clone())
                        .unwrap_or_else(|| "transaction".to_string()),
                    max_client_conn: db_spec.pooler.as_ref()
                        .map(|p| p.max_client_conn)
                        .unwrap_or(1000),
                },
            };

            cnpg_manager.reconcile_database(&name, &namespace, &db_config).await?;
        }
    }

    // ... rest of reconciliation logic ...

    Ok(Action::requeue(Duration::from_secs(300)))
}