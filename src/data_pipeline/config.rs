//! Pipeline configuration types.

use serde::{Deserialize, Serialize};

/// Top-level pipeline configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    pub kafka: KafkaConfig,
    pub sinks: SinksConfig,
    pub etl: EtlConfig,
    pub dlq_topic: String,
    pub consumer_group: String,
    pub source_topics: Vec<String>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            kafka: KafkaConfig::default(),
            sinks: SinksConfig::default(),
            etl: EtlConfig::default(),
            dlq_topic: "stellar.dlq".into(),
            consumer_group: "stellar-pipeline".into(),
            source_topics: vec!["stellar.ledger.events".into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KafkaConfig {
    pub brokers: String,
    pub security_protocol: String,
    pub sasl_mechanism: Option<String>,
    pub sasl_username: Option<String>,
    pub sasl_password: Option<String>,
    /// Max messages to buffer before back-pressure
    pub fetch_max_bytes: usize,
}

impl Default for KafkaConfig {
    fn default() -> Self {
        Self {
            brokers: "localhost:9092".into(),
            security_protocol: "PLAINTEXT".into(),
            sasl_mechanism: None,
            sasl_username: None,
            sasl_password: None,
            fetch_max_bytes: 52_428_800,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SinksConfig {
    pub postgres: Option<PostgresSinkConfig>,
    pub elasticsearch: Option<ElasticsearchSinkConfig>,
    pub s3: Option<S3SinkConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresSinkConfig {
    pub database_url: String,
    pub table: String,
    pub batch_size: usize,
}

impl Default for PostgresSinkConfig {
    fn default() -> Self {
        Self {
            database_url: "postgres://localhost/stellar".into(),
            table: "ledger_events".into(),
            batch_size: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElasticsearchSinkConfig {
    pub url: String,
    pub index: String,
    pub batch_size: usize,
}

impl Default for ElasticsearchSinkConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:9200".into(),
            index: "stellar-ledger".into(),
            batch_size: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3SinkConfig {
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    /// Flush to S3 after this many records
    pub batch_size: usize,
}

impl Default for S3SinkConfig {
    fn default() -> Self {
        Self {
            bucket: "stellar-pipeline".into(),
            prefix: "ledger-events/".into(),
            region: "us-east-1".into(),
            batch_size: 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtlConfig {
    /// Drop records that fail schema validation instead of sending to DLQ
    pub drop_invalid: bool,
    /// Enrich records with pipeline metadata fields
    pub add_pipeline_metadata: bool,
}

impl Default for EtlConfig {
    fn default() -> Self {
        Self {
            drop_invalid: false,
            add_pipeline_metadata: true,
        }
    }
}
