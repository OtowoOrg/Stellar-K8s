//! CloudNativePG resources.

use super::prelude::*;
use super::helpers::*;

// ============================================================================
// CloudNativePG (CNPG) Resources — unchanged
// ============================================================================

#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_cnpg_cluster(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let managed_db = match &node.spec.managed_database {
        Some(cfg) => cfg,
        None => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Cluster> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();

    let cluster = build_cnpg_cluster(node, managed_db);

    let patch = Patch::Apply(&cluster);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    info!("CNPG Cluster ensured for {}/{}", namespace, name);
    Ok(())
}

pub(crate) fn build_cnpg_cluster(node: &StellarNode, config: &ManagedDatabaseConfig) -> Cluster {
    let mut labels = standard_labels(node);
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "cnpg".to_string(),
    );
    let name = node.name_any();

    let mut cluster = Cluster {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: node.namespace(),
            labels: Some(labels),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: ClusterSpec {
            instances: config.instances,
            image_name: None,
            postgresql: Some(PostgresConfiguration {
                parameters: {
                    let mut p = BTreeMap::new();
                    p.insert("max_connections".to_string(), "100".to_string());
                    p.insert("shared_buffers".to_string(), "256MB".to_string());
                    p
                },
            }),
            external_clusters: None,
            replica: None,
            storage: StorageConfiguration {
                size: config.storage.size.clone(),
                storage_class: Some(config.storage.storage_class.clone()),
            },
            backup: config.backup.as_ref().map(|b| BackupConfiguration {
                barman_object_store: Some(BarmanObjectStore {
                    destination_path: b.destination_path.clone(),
                    endpoint_u_r_l: None,
                    s3_credentials: Some(S3Credentials {
                        access_key_id: CnpgSecretKeySelector {
                            name: b.credentials_secret_ref.clone(),
                            key: "AWS_ACCESS_KEY_ID".to_string(),
                        },
                        secret_access_key: CnpgSecretKeySelector {
                            name: b.credentials_secret_ref.clone(),
                            key: "AWS_SECRET_ACCESS_KEY".to_string(),
                        },
                    }),
                    azure_credentials: None,
                    google_credentials: None,
                    wal: Some(WalBackupConfiguration {
                        compression: Some("gzip".to_string()),
                    }),
                }),
                retention_policy: Some(b.retention_policy.clone()),
            }),
            bootstrap: Some(BootstrapConfiguration {
                initdb: Some(InitDbConfiguration {
                    database: config
                        .database_name
                        .clone()
                        .unwrap_or_else(|| "stellar".to_string()),
                    owner: config
                        .username
                        .clone()
                        .unwrap_or_else(|| "stellar".to_string()),
                    secret: None,
                }),
                recovery: None,
            }),
            monitoring: Some(MonitoringConfiguration {
                enable_pod_monitor: true,
            }),
        },
    };

    if !config.postgres_version.is_empty() {
        cluster.spec.image_name = Some(format!(
            "ghcr.io/cloudnative-pg/postgresql:{}",
            config.postgres_version
        ));
    }

    // Handle multi-region replication
    if let Some(repl_cfg) = &node.spec.replication_config {
        if repl_cfg.enabled && repl_cfg.role == ReplicationRole::Passive {
            let remote_name = format!("{}-primary", repl_cfg.remote_cluster_id);

            // Define external cluster pointing to the primary in the remote region
            let external_cluster = ExternalCluster {
                name: remote_name.clone(),
                connection_parameters: {
                    let mut p = BTreeMap::new();
                    p.insert(
                        "host".to_string(),
                        format!("{}.{}.svc", node.name_any(), repl_cfg.remote_cluster_id),
                    );
                    p.insert("user".to_string(), "stellar".to_string());
                    p.insert("dbname".to_string(), "stellar".to_string());
                    p.insert("sslmode".to_string(), "require".to_string());
                    p
                },
                password: CnpgSecretKeySelector {
                    name: format!("{}-app", node.name_any()),
                    key: "password".to_string(),
                },
            };

            cluster.spec.external_clusters = Some(vec![external_cluster]);

            // Configure bootstrap to recover from the external cluster
            if let Some(bootstrap) = &mut cluster.spec.bootstrap {
                bootstrap.initdb = None; // Cannot use initdb with recovery
                bootstrap.recovery = Some(RecoveryConfiguration {
                    source: remote_name.clone(),
                });
            }

            // Set as replica
            cluster.spec.replica = Some(ReplicaConfiguration {
                enabled: true,
                source: remote_name,
            });
        }
    }

    cluster
}

#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_cnpg_pooler(client: &Client, node: &StellarNode, dry_run: bool) -> Result<()> {
    let managed_db = match &node.spec.managed_database {
        Some(cfg) => cfg,
        None => return Ok(()),
    };

    let pgbouncer = match &managed_db.pooling {
        Some(p) if p.enabled => p,
        _ => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<Pooler> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "pooler");

    let pooler = build_cnpg_pooler(node, pgbouncer);

    let patch = Patch::Apply(&pooler);
    api.patch(&name, &patch_params(dry_run), &patch).await?;

    info!("CNPG Pooler ensured for {}/{}", namespace, name);
    Ok(())
}

pub(crate) fn build_cnpg_pooler(node: &StellarNode, config: &crate::crd::PgBouncerConfig) -> Pooler {
    let mut labels = standard_labels(node);
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        "pooler".to_string(),
    );
    let name = resource_name(node, "pooler");

    Pooler {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: node.namespace(),
            labels: Some(labels),
            owner_references: Some(vec![owner_reference(node)]),
            ..Default::default()
        },
        spec: PoolerSpec {
            cluster: PoolerCluster {
                name: node.name_any(),
            },
            instances: config.replicas,
            type_: "pgbouncer".to_string(),
            pgbouncer: PgBouncerSpec {
                pool_mode: match config.pool_mode {
                    crate::crd::PgBouncerPoolMode::Session => "session".to_string(),
                    crate::crd::PgBouncerPoolMode::Transaction => "transaction".to_string(),
                    crate::crd::PgBouncerPoolMode::Statement => "statement".to_string(),
                },
                parameters: {
                    let mut p = BTreeMap::new();
                    p.insert(
                        "max_client_conn".to_string(),
                        config.max_client_conn.to_string(),
                    );
                    p.insert(
                        "default_pool_size".to_string(),
                        config.default_pool_size.to_string(),
                    );
                    p
                },
            },
            monitoring: Some(MonitoringConfiguration {
                enable_pod_monitor: true,
            }),
        },
    }
}

#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_cnpg_resources(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
) -> Result<()> {
    if node.spec.managed_database.is_none() {
        return Ok(());
    }

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

    let pooler_api: Api<Pooler> = Api::namespaced(client.clone(), &namespace);
    let pooler_name = resource_name(node, "pooler");
    let _ = pooler_api
        .delete(&pooler_name, &delete_params(dry_run))
        .await;

    let cluster_api: Api<Cluster> = Api::namespaced(client.clone(), &namespace);
    let cluster_name = node.name_any();
    let _ = cluster_api
        .delete(&cluster_name, &delete_params(dry_run))
        .await;

    Ok(())
}

