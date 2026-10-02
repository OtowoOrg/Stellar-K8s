use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub kafka: KafkaConfig,
    pub scp_stream: ScpStreamConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kafka: KafkaConfig::default(),
            scp_stream: ScpStreamConfig::default(),
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
            config.scp_stream.hash_seed = v.parse().unwrap_or(0);
        }
        if let Ok(v) = std::env::var("SCP_BUFFER_SIZE") {
            config.scp_stream.buffer_size = v.parse().unwrap_or(100_000);
        }
        if let Ok(v) = std::env::var("SCP_OVERFLOW_STRATEGY") {
            config.scp_stream.overflow_strategy = match v.as_str() {
                "backpressure" => OverflowStrategy::Backpressure,
                _ => OverflowStrategy::DropOldest,
            };
        }
        if let Ok(v) = std::env::var("SCP_DRAIN_BATCH_SIZE") {
            config.scp_stream.drain_batch_size = v.parse().unwrap_or(256);
        }
        config
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KafkaConfig {
    pub brokers: String,
    pub topic: String,
    pub group_id: String,
    pub num_partitions: usize,
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
#[serde(rename_all = "snake_case")]
pub enum PartitionMode {
    Single,
    Dynamic,
}

impl Default for PartitionMode {
    fn default() -> Self {
        PartitionMode::Single
    }
}

/// Buffer overflow strategy when the ring-buffer is full.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverflowStrategy {
    /// Silently drop the oldest unread message and insert the new one.
    DropOldest,
    /// Signal backpressure to the caller (write returns `false`).
    Backpressure,
}

impl Default for OverflowStrategy {
    fn default() -> Self {
        OverflowStrategy::DropOldest
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScpStreamConfig {
    pub hash_seed: u64,
    /// Capacity of the lock-free ring-buffer (number of messages).
    pub buffer_size: usize,
    /// What to do when the buffer is full.
    pub overflow_strategy: OverflowStrategy,
    /// Number of messages drained per micro-batch iteration.
    pub drain_batch_size: usize,
}

impl Default for ScpStreamConfig {
    fn default() -> Self {
        Self {
            hash_seed: 0xD1B5A32D,
            buffer_size: 100_000,
            overflow_strategy: OverflowStrategy::DropOldest,
            drain_batch_size: 256,
        }
    }
}
