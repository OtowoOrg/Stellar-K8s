use stellar_k8s::backup::{
    BackupSource, BackupVerificationConfig, VerificationResources, VerificationStrategy,
};

#[tokio::test]
async fn test_backup_verification_config_default() {
    let config = BackupVerificationConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.schedule, "0 2 * * 0");
    assert_eq!(config.timeout_minutes, 60);
    assert!(!config.benchmark_enabled);
    assert_eq!(config.strategy, VerificationStrategy::Standard);
}

#[tokio::test]
async fn test_backup_source_s3() {
    let source = BackupSource::S3 {
        bucket: "test-bucket".to_string(),
        region: "us-east-1".to_string(),
        prefix: "backups/".to_string(),
        credentials_secret: "aws-creds".to_string(),
    };

    let json = serde_json::to_string(&source).unwrap();
    let deserialized: BackupSource = serde_json::from_str(&json).unwrap();

    assert_eq!(source, deserialized);
}

#[tokio::test]
async fn test_backup_source_volume_snapshot() {
    let source = BackupSource::VolumeSnapshot {
        snapshot_name: "my-snapshot".to_string(),
        storage_class: "fast-ssd".to_string(),
    };

    let json = serde_json::to_string(&source).unwrap();
    let deserialized: BackupSource = serde_json::from_str(&json).unwrap();

    assert_eq!(source, deserialized);
}

#[tokio::test]
async fn test_verification_strategy() {
    let quick = VerificationStrategy::Quick;
    let standard = VerificationStrategy::Standard;
    let full = VerificationStrategy::Full;

    assert_ne!(quick, standard);
    assert_ne!(standard, full);
    assert_ne!(quick, full);
}

#[tokio::test]
async fn test_verification_resources_default() {
    let resources = VerificationResources::default();

    assert_eq!(resources.cpu_limit, "2000m");
    assert_eq!(resources.memory_limit, "4Gi");
    assert_eq!(resources.storage_size, "100Gi");
}

#[tokio::test]
async fn test_backup_verification_config_serialization() {
    let config = BackupVerificationConfig {
        enabled: true,
        schedule: "0 2 * * 0".to_string(),
        backup_source: BackupSource::S3 {
            bucket: "test-bucket".to_string(),
            region: "us-west-2".to_string(),
            prefix: "backups/".to_string(),
            credentials_secret: "aws-credentials".to_string(),
        },
        strategy: VerificationStrategy::Full,
        timeout_minutes: 120,
        benchmark_enabled: true,
        notification_webhook: Some("https://webhook.example.com".to_string()),
        report_storage: None,
        resources: VerificationResources::default(),
    };

    let json = serde_json::to_string(&config).unwrap();
    let deserialized: BackupVerificationConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(config, deserialized);
}
