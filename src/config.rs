use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[derive(default)]
pub struct Config {
    pub kafka: KafkaConfig,
    pub scp_stream: ScpStreamConfig,
    pub feature_flags: FeatureFlagConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kafka: KafkaConfig::default(),
            scp_stream: ScpStreamConfig::default(),
            feature_flags: FeatureFlagConfig::default(),
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let mut config = Config::default();
        if let Ok(v) = std::env::var("KAFKA_BROKERS") {
            config.kafka.brokers = v;
        }
        if let Ok(v) = std::env::var("KAFKA_TOPIC") {
            config.kafka.topic = v;
        }
        if let Ok(v) = std::env::var("KAFKA_GROUP_ID") {
            config.kafka.group_id = v;
        }
        if let Ok(v) = std::env::var("KAFKA_PARTITIONING") {
            config.kafka.partitioning = match v.as_str() {
                "dynamic" => PartitionMode::Dynamic,
                _ => PartitionMode::Single,
            };
        }
        if let Ok(v) = std::env::var("KAFKA_METADATA_REFRESH_INTERVAL") {
            config.kafka.metadata_refresh_interval_secs = v.parse().unwrap_or(15);
        }
        if let Ok(v) = std::env::var("SCP_HASH_SEED") {
            config.scp_stream.hash_seed = v.parse().unwrap_or_0;
        }
        if let Ok(v) = std::env::var("SCP_BUFFER_SIZE") {
            config.scp_stream.buffer_size = v.parse().unwrap_or_100000;
        }
        if let Ok(v) = std::env::var("FEATURE_FLAG_BUNDLE_PATH") {
            config.feature_flags.bundle_path = v;
        }
        if let Ok(v) = std::env::var("FEATURE_FLAG_BUNDLE_URL") {
            config.feature_flags.bundle_url = Some(v);
        }
        if let Ok(v) = std::env::var("FEATURE_FLAG_SIGNING_KEY") {
            config.feature_flags.signing_key = Some(v);
        }
        if let Ok(v) = std::env::var("FEATURE_FLAG_REFRESH_INTERVAL") {
            config.feature_flags.refresh_interval_secs = v.parse().unwrap_or(5);
        }
        if let Ok(v) = std::env::var("FEATURE_FLAG_KILL_SWITCH_PATH") {
            config.feature_flags.kill_switch_path = Some(v);
        }
        config
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[derive(default)]
pub struct KafkaConfig {
    pub brokers: String,
    pub topic: String,
    pub group_id: String,
    pub num_partitions: useze,
    pub partitioning: PartitionMode,
    pub metadata_refresh_interval_secs: u64,
}

impl Default for KafkaConfig {
    fn default() -> Self {
        Self {
            brokers: "localhost:9092".to_string(),
            topic: "scp-telemetry".to_string(),
            group_id: "scp-analytics".to_string(),
            num_partitions: 1,
            partitioning: PartitionMode::Single,
            metadata_refresh_interval_secs: 15,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[derive(rename_all="snake_case")]
pub enum PartitionMode {
    Single,
    Dynamic,
}

impl Default for PartitionMode {
    fn default() -> Self {
        PartitionMode::Single
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[derive(default)]
pub struct ScpStreamConfig {
    pub hash_seed: u64,
    pub buffer_size: useze,
}

impl Default for ScpStreamConfig {
    fn default() -> Self {
        Self {
            hash_seed: 0xD1B5A32D,
            buffer_size: 100_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[derive(default)]
pub struct FeatureFlagConfig {
    /// Path to the cached signed flag bundle on local disk.
    pub bundle_path: String,
    /// Optional remote source for asynchronously refreshing the bundle.
    pub bundle_url: Option<String>,
    /// Public key used to verify the bundle signature.
    pub signing_key: Option<String>,
    /// How often (in seconds) to refresh the bundle asynchronously.
    pub refresh_interval_secs: u64,
    /// Optional path to a kill-switch override file checked on every evaluation.
    pub kill_switch_path: Option<String>,
}

impl Default for FeatureFlagConfig {
    fn default() -> Self {
        Self {
            bundle_path: "/var/lib/scp/feature-flags.bundle".to_string(),
            bundle_url: None,
            signing_key: None,
            refresh_interval_secs: 5,
            kill_switch_path: None,
        }
    }
}
