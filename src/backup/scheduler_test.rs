//! Integration tests for the backup scheduler (first half – issue #175).
//!
//! Covers: cron schedule parsing & next-execution timing, invalid schedule
//! rejection, BackupScheduler construction, DecentralizedBackupConfig
//! serialisation round-trips for every provider variant, serde default
//! values, RetentionPolicy serialisation, UploadMetadata construction,
//! and gzip compression via `compress_data`.

#[cfg(test)]
mod tests {
    use crate::backup::providers::{StorageProviderTrait, UploadMetadata};
    use crate::backup::scheduler::{compress_data, BackupScheduler};
    use crate::backup::*;

    use anyhow::Result;
    use async_trait::async_trait;
    use chrono::Utc;
    use cron::Schedule;
    use std::str::FromStr;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // ---------------------------------------------------------------------
    // Mock provider
    // ---------------------------------------------------------------------

    type UploadRecord = Vec<(Vec<u8>, UploadMetadata)>;

    struct MockProvider {
        uploads: Arc<RwLock<UploadRecord>>,
    }

    impl MockProvider {
        fn new() -> Self {
            Self {
                uploads: Arc::new(RwLock::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl StorageProviderTrait for MockProvider {
        async fn upload(&self, data: Vec<u8>, metadata: UploadMetadata) -> Result<String> {
            self.uploads.write().await.push((data, metadata));
            Ok("mock-cid-12345".to_string())
        }

        async fn exists(&self, _content_hash: &str) -> Result<bool> {
            Ok(false)
        }

        async fn verify(&self, _cid: &str, _expected_hash: &str) -> Result<bool> {
            Ok(true)
        }
    }

    // ---------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------

    /// The `cron` 0.15 crate expects 6- or 7-field expressions
    /// (sec min hour dom month dow [year]), not 5-field Unix cron.
    const EVERY_6H_CRON: &str = "0 0 */6 * * *";
    const DAILY_MIDNIGHT_CRON: &str = "0 0 0 * * *";
    const EVERY_2D_CRON: &str = "0 0 0 */2 * *";

    fn arweave_config() -> DecentralizedBackupConfig {
        DecentralizedBackupConfig {
            enabled: true,
            provider: StorageProvider::Arweave {
                wallet_secret: "arweave-secret".to_string(),
                gateway: "https://arweave.net".to_string(),
                tags: vec![("App".to_string(), "StellarK8s".to_string())],
            },
            schedule: EVERY_6H_CRON.to_string(),
            max_concurrent_uploads: 3,
            compression_enabled: true,
            retention: Some(RetentionPolicy {
                days: 30,
                min_backups: 5,
            }),
        }
    }

    fn ipfs_config() -> DecentralizedBackupConfig {
        DecentralizedBackupConfig {
            enabled: true,
            provider: StorageProvider::IPFS {
                api_url: "http://localhost:5001".to_string(),
                pinning_service: Some(PinningService {
                    service_type: PinningServiceType::Pinata,
                    api_key_secret: "pinata-key".to_string(),
                }),
            },
            schedule: DAILY_MIDNIGHT_CRON.to_string(),
            max_concurrent_uploads: 5,
            compression_enabled: false,
            retention: None,
        }
    }

    fn filecoin_config() -> DecentralizedBackupConfig {
        DecentralizedBackupConfig {
            enabled: false,
            provider: StorageProvider::Filecoin {
                lotus_api: "http://lotus:1234/rpc/v0".to_string(),
                wallet_address: "f1abc123".to_string(),
                deal_params: FilecoinDealParams {
                    price_per_epoch: "500000000".to_string(),
                    duration: 518400,
                    verified: true,
                },
            },
            schedule: EVERY_2D_CRON.to_string(),
            max_concurrent_uploads: 1,
            compression_enabled: true,
            retention: Some(RetentionPolicy {
                days: 90,
                min_backups: 10,
            }),
        }
    }

    // ---------------------------------------------------------------------
    // 1. Cron schedule parsing – valid schedules
    // ---------------------------------------------------------------------

    #[test]
    fn test_every_6h_schedule_parses_correctly() {
        let schedule = Schedule::from_str(EVERY_6H_CRON);
        assert!(schedule.is_ok(), "6-hour cron expression must parse");
    }

    #[test]
    fn test_every_6h_schedule_next_executions_six_hours_apart() {
        let schedule = Schedule::from_str(EVERY_6H_CRON).unwrap();
        let upcoming: Vec<_> = schedule.upcoming(Utc).take(3).collect();

        assert_eq!(
            upcoming.len(),
            3,
            "Should produce at least 3 upcoming times"
        );

        let gap_1 = upcoming[1] - upcoming[0];
        let gap_2 = upcoming[2] - upcoming[1];

        assert!(
            gap_1.num_hours() <= 6,
            "Gap between consecutive firings should be at most 6 hours, got {}h",
            gap_1.num_hours(),
        );
        assert!(
            gap_2.num_hours() <= 6,
            "Second gap should also be at most 6 hours, got {}h",
            gap_2.num_hours(),
        );
    }

    #[test]
    fn test_daily_schedule_parses() {
        let schedule = Schedule::from_str(DAILY_MIDNIGHT_CRON);
        assert!(
            schedule.is_ok(),
            "Daily midnight cron expression must parse"
        );
    }

    #[test]
    fn test_five_field_unix_cron_rejected_by_crate() {
        let result = Schedule::from_str("0 */6 * * *");
        assert!(
            result.is_err(),
            "cron 0.15 requires 6-7 fields; 5-field Unix cron should be rejected"
        );
    }

    // ---------------------------------------------------------------------
    // 2. Invalid cron schedule detection
    // ---------------------------------------------------------------------

    #[test]
    fn test_invalid_cron_is_rejected() {
        let result = Schedule::from_str("not a cron");
        assert!(result.is_err(), "Garbage string must fail cron parsing");
    }

    #[test]
    fn test_empty_cron_is_rejected() {
        let result = Schedule::from_str("");
        assert!(result.is_err(), "Empty string must fail cron parsing");
    }

    // ---------------------------------------------------------------------
    // 3. BackupScheduler construction
    // ---------------------------------------------------------------------

    #[test]
    fn test_backup_scheduler_new_with_arweave() {
        let config = arweave_config();
        assert!(config.enabled);
        assert_eq!(config.schedule, EVERY_6H_CRON);
        assert_eq!(config.max_concurrent_uploads, 3);
        assert!(config.compression_enabled);

        let provider: Arc<dyn StorageProviderTrait> = Arc::new(MockProvider::new());
        let _scheduler = BackupScheduler::new(config, provider);
    }

    #[test]
    fn test_backup_scheduler_new_with_ipfs() {
        let config = ipfs_config();
        assert!(config.enabled);
        assert_eq!(config.schedule, DAILY_MIDNIGHT_CRON);
        assert_eq!(config.max_concurrent_uploads, 5);
        assert!(!config.compression_enabled);

        let provider: Arc<dyn StorageProviderTrait> = Arc::new(MockProvider::new());
        let _scheduler = BackupScheduler::new(config, provider);
    }

    // ---------------------------------------------------------------------
    // 4. DecentralizedBackupConfig serialization round-trip
    // ---------------------------------------------------------------------

    #[test]
    fn test_arweave_config_roundtrip() {
        let original = arweave_config();
        let json = serde_json::to_string_pretty(&original).expect("serialize");
        let restored: DecentralizedBackupConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn test_ipfs_config_roundtrip() {
        let original = ipfs_config();
        let json = serde_json::to_string_pretty(&original).expect("serialize");
        let restored: DecentralizedBackupConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn test_filecoin_config_roundtrip() {
        let original = filecoin_config();
        let json = serde_json::to_string_pretty(&original).expect("serialize");
        let restored: DecentralizedBackupConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn test_arweave_json_contains_expected_keys() {
        let config = arweave_config();
        let v: serde_json::Value = serde_json::to_value(&config).unwrap();

        assert_eq!(v["enabled"], true);
        assert_eq!(v["provider"]["type"], "arweave");
        assert_eq!(v["provider"]["wallet_secret"], "arweave-secret");
        assert_eq!(v["provider"]["gateway"], "https://arweave.net");
        assert_eq!(v["schedule"], EVERY_6H_CRON);
        assert_eq!(v["maxConcurrentUploads"], 3);
        assert_eq!(v["compressionEnabled"], true);
    }

    #[test]
    fn test_ipfs_json_contains_expected_keys() {
        let config = ipfs_config();
        let v: serde_json::Value = serde_json::to_value(&config).unwrap();

        assert_eq!(v["provider"]["type"], "ipfs");
        assert_eq!(v["provider"]["api_url"], "http://localhost:5001");
        assert_eq!(v["provider"]["pinning_service"]["serviceType"], "pinata");
        assert_eq!(
            v["provider"]["pinning_service"]["apiKeySecret"],
            "pinata-key"
        );
    }

    #[test]
    fn test_filecoin_json_contains_expected_keys() {
        let config = filecoin_config();
        let v: serde_json::Value = serde_json::to_value(&config).unwrap();

        assert_eq!(v["provider"]["type"], "filecoin");
        assert_eq!(v["provider"]["lotus_api"], "http://lotus:1234/rpc/v0");
        assert_eq!(v["provider"]["wallet_address"], "f1abc123");
        assert_eq!(v["provider"]["deal_params"]["pricePerEpoch"], "500000000");
        assert_eq!(v["provider"]["deal_params"]["duration"], 518400);
        assert_eq!(v["provider"]["deal_params"]["verified"], true);
    }

    // ---------------------------------------------------------------------
    // 5. Serde default values
    // ---------------------------------------------------------------------

    #[test]
    fn test_defaults_applied_when_fields_omitted() {
        let json = r#"{
            "enabled": true,
            "provider": {
                "type": "ipfs",
                "api_url": "http://localhost:5001",
                "pinning_service": null
            }
        }"#;

        let config: DecentralizedBackupConfig =
            serde_json::from_str(json).expect("should deserialize with defaults");

        assert_eq!(config.schedule, "0 */6 * * *");
        assert_eq!(config.max_concurrent_uploads, 3);
        assert!(config.compression_enabled);
        assert_eq!(config.retention, None);
    }

    #[test]
    fn test_defaults_can_be_overridden() {
        let json = r#"{
            "enabled": true,
            "provider": {
                "type": "ipfs",
                "api_url": "http://localhost:5001",
                "pinning_service": null
            },
            "schedule": "0 0 0 * * *",
            "maxConcurrentUploads": 10,
            "compressionEnabled": false
        }"#;

        let config: DecentralizedBackupConfig =
            serde_json::from_str(json).expect("should deserialize with overrides");

        assert_eq!(config.schedule, "0 0 0 * * *");
        assert_eq!(config.max_concurrent_uploads, 10);
        assert!(!config.compression_enabled);
    }

    #[test]
    fn test_arweave_gateway_default() {
        let json = r#"{
            "enabled": true,
            "provider": {
                "type": "arweave",
                "wallet_secret": "secret"
            }
        }"#;

        let config: DecentralizedBackupConfig =
            serde_json::from_str(json).expect("should deserialize arweave with defaults");

        match &config.provider {
            StorageProvider::Arweave { gateway, tags, .. } => {
                assert_eq!(gateway, "https://arweave.net");
                assert!(tags.is_empty());
            }
            other => panic!("Expected Arweave, got {other:?}"),
        }
    }

    #[test]
    fn test_filecoin_verified_default_false() {
        let json = r#"{
            "enabled": true,
            "provider": {
                "type": "filecoin",
                "lotus_api": "http://lotus:1234/rpc/v0",
                "wallet_address": "f1xyz",
                "deal_params": {
                    "pricePerEpoch": "100",
                    "duration": 100000
                }
            }
        }"#;

        let config: DecentralizedBackupConfig =
            serde_json::from_str(json).expect("should deserialize filecoin with defaults");

        match &config.provider {
            StorageProvider::Filecoin { deal_params, .. } => {
                assert!(!deal_params.verified);
            }
            other => panic!("Expected Filecoin, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------------
    // 6. RetentionPolicy serialization
    // ---------------------------------------------------------------------

    #[test]
    fn test_retention_policy_roundtrip() {
        let policy = RetentionPolicy {
            days: 60,
            min_backups: 7,
        };
        let json = serde_json::to_string(&policy).unwrap();
        let restored: RetentionPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, restored);
    }

    #[test]
    fn test_retention_policy_json_keys() {
        let policy = RetentionPolicy {
            days: 30,
            min_backups: 5,
        };
        let v: serde_json::Value = serde_json::to_value(&policy).unwrap();

        assert_eq!(v["days"], 30);
        assert_eq!(v["minBackups"], 5);
    }

    #[test]
    fn test_config_with_retention_none_roundtrip() {
        let mut config = ipfs_config();
        config.retention = None;

        let json = serde_json::to_string(&config).unwrap();
        let restored: DecentralizedBackupConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.retention, None);
    }

    // ---------------------------------------------------------------------
    // 7. UploadMetadata construction
    // ---------------------------------------------------------------------

    #[test]
    fn test_upload_metadata_construction() {
        let meta = UploadMetadata {
            filename: "history-0abcdef0.xdr.gz".to_string(),
            content_type: "application/octet-stream".to_string(),
            size: 4096,
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            tags: vec![
                ("Ledger".to_string(), "123456".to_string()),
                ("Type".to_string(), "history".to_string()),
            ],
        };

        assert_eq!(meta.filename, "history-0abcdef0.xdr.gz");
        assert_eq!(meta.content_type, "application/octet-stream");
        assert_eq!(meta.size, 4096);
        assert_eq!(meta.sha256.len(), 64);
        assert_eq!(meta.tags.len(), 2);
        assert_eq!(meta.tags[0].0, "Ledger");
        assert_eq!(meta.tags[0].1, "123456");
        assert_eq!(meta.tags[1].0, "Type");
        assert_eq!(meta.tags[1].1, "history");
    }

    #[test]
    fn test_upload_metadata_empty_tags() {
        let meta = UploadMetadata {
            filename: "test.bin".to_string(),
            content_type: "application/octet-stream".to_string(),
            size: 0,
            sha256: "".to_string(),
            tags: vec![],
        };

        assert!(meta.tags.is_empty());
        assert_eq!(meta.size, 0);
    }

    #[test]
    fn test_upload_metadata_clone() {
        let meta = UploadMetadata {
            filename: "segment.xdr".to_string(),
            content_type: "application/octet-stream".to_string(),
            size: 1024,
            sha256: "abc123".to_string(),
            tags: vec![("Key".to_string(), "Value".to_string())],
        };

        let cloned = meta.clone();
        assert_eq!(meta.filename, cloned.filename);
        assert_eq!(meta.size, cloned.size);
        assert_eq!(meta.sha256, cloned.sha256);
        assert_eq!(meta.tags, cloned.tags);
    }

    // ---------------------------------------------------------------------
    // 8. Compression – compress_data round-trip
    // ---------------------------------------------------------------------

    #[test]
    fn test_compress_data_roundtrip() {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let original = b"the quick brown fox jumps over the lazy dog";
        let compressed = compress_data(original).expect("compression must succeed");

        assert_ne!(
            compressed,
            original.to_vec(),
            "compressed output differs from input"
        );

        let mut decoder = GzDecoder::new(&compressed[..]);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .expect("decompression must succeed");

        assert_eq!(decompressed, original.to_vec());
    }

    #[test]
    fn test_compress_data_gzip_magic_bytes() {
        let compressed = compress_data(b"hello world").expect("compression must succeed");
        assert!(
            compressed.len() >= 2,
            "gzip output must have at least 2 bytes"
        );
        assert_eq!(compressed[0], 0x1f, "first magic byte");
        assert_eq!(compressed[1], 0x8b, "second magic byte");
    }

    #[test]
    fn test_compress_data_empty_input() {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let compressed = compress_data(b"").expect("compressing empty input must succeed");
        assert_eq!(compressed[0], 0x1f);
        assert_eq!(compressed[1], 0x8b);

        let mut decoder = GzDecoder::new(&compressed[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap();
        assert!(decompressed.is_empty());
    }

    #[test]
    fn test_compress_data_large_input() {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let original: Vec<u8> = (0..10_000).map(|i| (i % 256) as u8).collect();
        let compressed = compress_data(&original).expect("compression must succeed");

        let mut decoder = GzDecoder::new(&compressed[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap();

        assert_eq!(decompressed, original);
    }
}
