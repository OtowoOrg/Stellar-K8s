//! Unit tests for the S3 ledger snapshot backup CronJob builder.

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::EnvVarSource;
    use kube::api::ObjectMeta;

    use super::super::resources::build_backup_cronjob;
    use crate::crd::types::{
        BackupScheduleConfig, HistoryMode, NodeType, ResourceRequirements, ResourceSpec,
        RetentionPolicy, RolloutStrategy, StellarNetwork, StorageConfig, ValidatorConfig,
    };
    use crate::crd::{StellarNode, StellarNodeSpec};

    /// Construct a minimal `StellarNode` suitable for unit-testing resource
    /// builders.  No Kubernetes API calls are made.
    fn make_validator_node(name: &str, backup: Option<BackupScheduleConfig>) -> StellarNode {
        StellarNode {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                uid: Some(format!("test-uid-{name}")),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Validator,
                network: StellarNetwork::Testnet,
                version: "v21.0.0".to_string(),
                history_mode: HistoryMode::Recent,
                resources: ResourceRequirements {
                    requests: ResourceSpec {
                        cpu: "500m".to_string(),
                        memory: "1Gi".to_string(),
                    },
                    limits: ResourceSpec {
                        cpu: "2".to_string(),
                        memory: "4Gi".to_string(),
                    },
                },
                storage: StorageConfig {
                    storage_class: "standard".to_string(),
                    size: "100Gi".to_string(),
                    retention_policy: RetentionPolicy::Delete,
                    annotations: None,
                },
                validator_config: Some(ValidatorConfig {
                    seed_secret_ref: "validator-seed".to_string(),
                    seed_secret_source: None,
                    quorum_set: None,
                    enable_history_archive: false,
                    history_archive_urls: vec![],
                    catchup_complete: false,
                    key_source: Default::default(),
                    kms_config: None,
                    vl_source: None,
                    hsm_config: None,
                }),
                horizon_config: None,
                soroban_config: None,
                replicas: 1,
                min_available: None,
                max_unavailable: None,
                suspended: false,
                alerting: false,
                database: None,
                managed_database: None,
                autoscaling: None,
                vpa_config: None,
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
                strategy: RolloutStrategy::default(),
                maintenance_mode: false,
                network_policy: None,
                dr_config: None,
                topology_spread_constraints: None,
                cve_handling: None,
                read_replica_config: None,
                backup_schedule: backup,
                oci_snapshot: None,
                service_mesh: None,
                resource_meta: None,
                read_pool_endpoint: None,
            },
            status: None,
        }
    }

    fn full_backup_config() -> BackupScheduleConfig {
        BackupScheduleConfig {
            enabled: true,
            bucket: "stellar-ledger-backups".to_string(),
            region: "us-east-1".to_string(),
            endpoint: None,
            prefix: Some("testnet/validator".to_string()),
            credentials_secret: "s3-credentials".to_string(),
            schedule: "0 */6 * * *".to_string(),
            compression: true,
            ledger_path: None,
            retention_count: 10,
            image: None,
        }
    }

    // -------------------------------------------------------------------------
    // Metadata
    // -------------------------------------------------------------------------

    #[test]
    fn cronjob_name_uses_node_name_and_backup_suffix() {
        let node = make_validator_node("my-validator", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        assert_eq!(
            cronjob.metadata.name.as_deref(),
            Some("my-validator-backup"),
        );
    }

    #[test]
    fn cronjob_carries_owner_reference_to_stellar_node() {
        let node = make_validator_node("my-validator", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        let owner_refs = cronjob
            .metadata
            .owner_references
            .as_ref()
            .expect("owner_references should be set");

        assert_eq!(owner_refs.len(), 1);
        assert_eq!(owner_refs[0].name, "my-validator");
        assert_eq!(owner_refs[0].kind, "StellarNode");
        assert!(owner_refs[0].controller.unwrap_or(false));
    }

    #[test]
    fn cronjob_has_standard_stellar_labels() {
        let node = make_validator_node("labelled-node", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        let labels = cronjob
            .metadata
            .labels
            .as_ref()
            .expect("labels should be set");

        assert_eq!(
            labels
                .get("app.kubernetes.io/managed-by")
                .map(String::as_str),
            Some("stellar-operator"),
        );
        assert_eq!(
            labels.get("app.kubernetes.io/instance").map(String::as_str),
            Some("labelled-node"),
        );
    }

    // -------------------------------------------------------------------------
    // CronJob spec
    // -------------------------------------------------------------------------

    #[test]
    fn cronjob_schedule_matches_config() {
        let node = make_validator_node("sched-test", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        let spec = cronjob.spec.expect("spec should be present");
        assert_eq!(spec.schedule, "0 */6 * * *");
    }

    #[test]
    fn cronjob_uses_forbid_concurrency_policy() {
        let node = make_validator_node("concurrency-test", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        let spec = cronjob.spec.expect("spec should be present");
        assert_eq!(spec.concurrency_policy.as_deref(), Some("Forbid"));
    }

    #[test]
    fn cronjob_pod_restart_policy_is_on_failure() {
        let node = make_validator_node("restart-test", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        let pod_spec = cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .expect("pod spec should be present");

        assert_eq!(pod_spec.restart_policy.as_deref(), Some("OnFailure"));
    }

    // -------------------------------------------------------------------------
    // Volumes – the node PVC must be mounted read-only
    // -------------------------------------------------------------------------

    #[test]
    fn cronjob_mounts_pvc_read_only() {
        let node = make_validator_node("pvc-test", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        let pod_spec = cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap();

        let volumes = pod_spec.volumes.expect("volumes should be present");
        let pvc_vol = volumes
            .iter()
            .find(|v| v.name == "ledger-data")
            .expect("ledger-data volume must exist");

        let pvc_src = pvc_vol
            .persistent_volume_claim
            .as_ref()
            .expect("volume source must be a PVC");

        assert_eq!(pvc_src.claim_name, "pvc-test-data");
        assert_eq!(pvc_src.read_only, Some(true));
    }

    #[test]
    fn cronjob_has_tmp_emptydir_volume() {
        let node = make_validator_node("tmp-vol-test", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        let pod_spec = cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap();

        let volumes = pod_spec.volumes.expect("volumes should be present");
        assert!(
            volumes.iter().any(|v| v.name == "tmp-storage"),
            "tmp-storage EmptyDir volume must be present"
        );
    }

    // -------------------------------------------------------------------------
    // Container – environment variables
    // -------------------------------------------------------------------------

    fn get_env_value<'a>(
        env: &'a [k8s_openapi::api::core::v1::EnvVar],
        name: &str,
    ) -> Option<&'a str> {
        env.iter()
            .find(|e| e.name == name)
            .and_then(|e| e.value.as_deref())
    }

    fn get_env_secret_ref<'a>(
        env: &'a [k8s_openapi::api::core::v1::EnvVar],
        name: &str,
    ) -> Option<&'a k8s_openapi::api::core::v1::SecretKeySelector> {
        env.iter()
            .find(|e| e.name == name)
            .and_then(|e| e.value_from.as_ref())
            .and_then(|vf: &EnvVarSource| vf.secret_key_ref.as_ref())
    }

    #[test]
    fn cronjob_env_has_correct_bucket_and_region() {
        let node = make_validator_node("env-test", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        let container = &cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .containers[0];

        let env = container.env.as_ref().expect("env vars should be set");

        assert_eq!(
            get_env_value(env, "S3_BUCKET"),
            Some("stellar-ledger-backups")
        );
        assert_eq!(get_env_value(env, "AWS_DEFAULT_REGION"), Some("us-east-1"));
        assert_eq!(get_env_value(env, "S3_PREFIX"), Some("testnet/validator"));
        assert_eq!(get_env_value(env, "S3_RETENTION_COUNT"), Some("10"));
    }

    #[test]
    fn cronjob_env_credentials_sourced_from_secret() {
        let node = make_validator_node("creds-test", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        let container = &cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .containers[0];

        let env = container.env.as_ref().expect("env vars should be set");

        let key_id_ref =
            get_env_secret_ref(env, "AWS_ACCESS_KEY_ID").expect("AWS_ACCESS_KEY_ID ref missing");
        assert_eq!(key_id_ref.name.as_deref(), Some("s3-credentials"));
        assert_eq!(key_id_ref.key, "AWS_ACCESS_KEY_ID");
        assert_eq!(key_id_ref.optional, Some(false));

        let secret_key_ref = get_env_secret_ref(env, "AWS_SECRET_ACCESS_KEY")
            .expect("AWS_SECRET_ACCESS_KEY ref missing");
        assert_eq!(secret_key_ref.name.as_deref(), Some("s3-credentials"));
        assert_eq!(secret_key_ref.key, "AWS_SECRET_ACCESS_KEY");
        assert_eq!(secret_key_ref.optional, Some(false));

        let session_token_ref =
            get_env_secret_ref(env, "AWS_SESSION_TOKEN").expect("AWS_SESSION_TOKEN ref missing");
        assert_eq!(session_token_ref.name.as_deref(), Some("s3-credentials"));
        assert_eq!(session_token_ref.optional, Some(true));
    }

    #[test]
    fn cronjob_endpoint_env_set_when_configured() {
        let cfg = BackupScheduleConfig {
            endpoint: Some("http://minio.svc:9000".to_string()),
            ..full_backup_config()
        };
        let node = make_validator_node("minio-test", Some(cfg.clone()));
        let cronjob = build_backup_cronjob(&node, &cfg);

        let container = &cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .containers[0];

        let env = container.env.as_ref().expect("env vars should be set");

        assert_eq!(
            get_env_value(env, "S3_ENDPOINT_URL"),
            Some("http://minio.svc:9000")
        );
    }

    #[test]
    fn cronjob_no_endpoint_env_when_not_configured() {
        let node = make_validator_node("aws-test", Some(full_backup_config()));
        let cfg = full_backup_config(); // endpoint is None
        let cronjob = build_backup_cronjob(&node, &cfg);

        let container = &cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .containers[0];

        let env = container.env.as_ref().expect("env vars should be set");

        assert!(
            !env.iter().any(|e| e.name == "S3_ENDPOINT_URL"),
            "S3_ENDPOINT_URL must not be set when no endpoint is configured"
        );
    }

    // -------------------------------------------------------------------------
    // Container – image and command
    // -------------------------------------------------------------------------

    #[test]
    fn cronjob_uses_default_image_when_not_specified() {
        let node = make_validator_node("img-default", Some(full_backup_config()));
        let cfg = full_backup_config(); // image is None
        let cronjob = build_backup_cronjob(&node, &cfg);

        let container = &cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .containers[0];

        assert_eq!(container.image.as_deref(), Some("amazon/aws-cli:latest"),);
    }

    #[test]
    fn cronjob_uses_custom_image_when_specified() {
        let cfg = BackupScheduleConfig {
            image: Some("bitnami/aws-cli:2.13".to_string()),
            ..full_backup_config()
        };
        let node = make_validator_node("img-custom", Some(cfg.clone()));
        let cronjob = build_backup_cronjob(&node, &cfg);

        let container = &cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .containers[0];

        assert_eq!(container.image.as_deref(), Some("bitnami/aws-cli:2.13"));
    }

    #[test]
    fn cronjob_container_invoked_via_sh_minus_c() {
        let node = make_validator_node("cmd-test", Some(full_backup_config()));
        let cfg = full_backup_config();
        let cronjob = build_backup_cronjob(&node, &cfg);

        let container = &cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .containers[0];

        let cmd = container.command.as_ref().expect("command should be set");
        assert_eq!(cmd, &["/bin/sh", "-c"]);

        let args = container.args.as_ref().expect("args should be set");
        assert!(!args.is_empty(), "backup script arg should be non-empty");
        // The script must contain the core upload command.
        assert!(
            args[0].contains("aws") && args[0].contains("s3 cp"),
            "backup script must contain 'aws s3 cp'"
        );
    }

    // -------------------------------------------------------------------------
    // Defaults – ledger path and prefix
    // -------------------------------------------------------------------------

    #[test]
    fn cronjob_defaults_ledger_path_to_data() {
        let cfg = BackupScheduleConfig {
            ledger_path: None, // should default to /data
            ..full_backup_config()
        };
        let node = make_validator_node("ledger-default", Some(cfg.clone()));
        let cronjob = build_backup_cronjob(&node, &cfg);

        let container = &cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .containers[0];

        let env = container.env.as_ref().expect("env vars should be set");
        assert_eq!(get_env_value(env, "LEDGER_PATH"), Some("/data"));

        let mounts = container
            .volume_mounts
            .as_ref()
            .expect("volume mounts should be set");
        let ledger_mount = mounts
            .iter()
            .find(|m| m.name == "ledger-data")
            .expect("ledger-data mount must exist");
        assert_eq!(ledger_mount.mount_path, "/data");
    }

    #[test]
    fn cronjob_defaults_prefix_to_snapshots() {
        let cfg = BackupScheduleConfig {
            prefix: None, // should default to "snapshots"
            ..full_backup_config()
        };
        let node = make_validator_node("prefix-default", Some(cfg.clone()));
        let cronjob = build_backup_cronjob(&node, &cfg);

        let container = &cronjob
            .spec
            .unwrap()
            .job_template
            .spec
            .unwrap()
            .template
            .spec
            .unwrap()
            .containers[0];

        let env = container.env.as_ref().expect("env vars should be set");
        assert_eq!(get_env_value(env, "S3_PREFIX"), Some("snapshots"));
    }

    // -------------------------------------------------------------------------
    // Validation – BackupScheduleConfig in StellarNodeSpec
    // -------------------------------------------------------------------------

    #[test]
    fn validation_passes_with_valid_backup_config() {
        let node = make_validator_node("valid-backup", Some(full_backup_config()));
        assert!(
            node.spec.validate().is_ok(),
            "spec with a valid backup config should pass validation"
        );
    }

    #[test]
    fn validation_passes_when_backup_is_absent() {
        let node = make_validator_node("no-backup", None);
        assert!(
            node.spec.validate().is_ok(),
            "spec without a backup config should pass validation"
        );
    }

    #[test]
    fn validation_passes_when_backup_is_disabled() {
        let cfg = BackupScheduleConfig {
            enabled: false,
            bucket: String::new(),
            region: String::new(),
            credentials_secret: String::new(),
            ..full_backup_config()
        };
        let node = make_validator_node("disabled-backup", Some(cfg));
        assert!(
            node.spec.validate().is_ok(),
            "spec with backup disabled should pass validation even with empty fields"
        );
    }

    #[test]
    fn validation_fails_when_enabled_with_empty_bucket() {
        let cfg = BackupScheduleConfig {
            bucket: String::new(),
            ..full_backup_config()
        };
        let node = make_validator_node("empty-bucket", Some(cfg));
        let errors = node.spec.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.field.contains("backupSchedule.bucket")),
            "should report an error for empty bucket"
        );
    }

    #[test]
    fn validation_fails_when_enabled_with_empty_region() {
        let cfg = BackupScheduleConfig {
            region: String::new(),
            ..full_backup_config()
        };
        let node = make_validator_node("empty-region", Some(cfg));
        let errors = node.spec.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.field.contains("backupSchedule.region")),
            "should report an error for empty region"
        );
    }

    #[test]
    fn validation_fails_when_enabled_with_empty_credentials_secret() {
        let cfg = BackupScheduleConfig {
            credentials_secret: String::new(),
            ..full_backup_config()
        };
        let node = make_validator_node("empty-creds", Some(cfg));
        let errors = node.spec.validate().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.field.contains("backupSchedule.credentialsSecret")),
            "should report an error for empty credentialsSecret"
        );
    }
}
