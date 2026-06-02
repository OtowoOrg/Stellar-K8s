//! Data warehouse integration adapter (Snowflake/BigQuery adapter pattern)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

use super::etl::EtlRecord;
use super::partitioning::PartitionKey;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WarehouseProvider {
    Snowflake,
    BigQuery,
    Redshift,
    DeltaLake,
    /// Dry-run / testing adapter
    NoOp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WarehouseConfig {
    pub provider: WarehouseProvider,
    pub project_or_account: String,
    pub dataset_or_schema: String,
    pub table_prefix: String,
    /// Extra provider-specific options (region, warehouse name, etc.)
    pub options: HashMap<String, String>,
}

impl WarehouseConfig {
    pub fn noop() -> Self {
        Self {
            provider: WarehouseProvider::NoOp,
            project_or_account: "local".into(),
            dataset_or_schema: "test".into(),
            table_prefix: "stellar_".into(),
            options: HashMap::new(),
        }
    }
}

/// Write result for a single partition batch
#[derive(Debug)]
pub struct WriteResult {
    pub partition: PartitionKey,
    pub rows_written: usize,
    pub bytes_estimate: usize,
}

/// Common interface for all warehouse adapters
#[async_trait]
pub trait WarehouseAdapter: Send + Sync {
    async fn write_batch(
        &self,
        partition: &PartitionKey,
        records: &[EtlRecord],
    ) -> Result<WriteResult, String>;

    async fn ensure_table(&self, table_name: &str) -> Result<(), String>;

    fn provider(&self) -> WarehouseProvider;
}

// ── NoOp adapter (testing / dry-run) ─────────────────────────────────────────

pub struct NoOpAdapter {
    config: WarehouseConfig,
}

impl NoOpAdapter {
    pub fn new(config: WarehouseConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl WarehouseAdapter for NoOpAdapter {
    async fn write_batch(
        &self,
        partition: &PartitionKey,
        records: &[EtlRecord],
    ) -> Result<WriteResult, String> {
        let bytes_estimate = records.len() * 512; // rough estimate
        debug!(
            provider = "noop",
            partition = %partition.storage_path(),
            rows = records.len(),
            "NoOp: would write batch to warehouse"
        );
        Ok(WriteResult {
            partition: partition.clone(),
            rows_written: records.len(),
            bytes_estimate,
        })
    }

    async fn ensure_table(&self, table_name: &str) -> Result<(), String> {
        debug!(table = %table_name, "NoOp: would ensure table exists");
        Ok(())
    }

    fn provider(&self) -> WarehouseProvider {
        WarehouseProvider::NoOp
    }
}

// ── Snowflake adapter stub ────────────────────────────────────────────────────

pub struct SnowflakeAdapter {
    config: WarehouseConfig,
}

impl SnowflakeAdapter {
    pub fn new(config: WarehouseConfig) -> Self {
        Self { config }
    }

    fn table_name(&self, partition: &PartitionKey) -> String {
        format!(
            "{}.{}.{}ledgers",
            self.config.project_or_account,
            self.config.dataset_or_schema,
            self.config.table_prefix,
        )
    }
}

#[async_trait]
impl WarehouseAdapter for SnowflakeAdapter {
    async fn write_batch(
        &self,
        partition: &PartitionKey,
        records: &[EtlRecord],
    ) -> Result<WriteResult, String> {
        let table = self.table_name(partition);
        info!(
            table = %table,
            partition = %partition.storage_path(),
            rows = records.len(),
            "Snowflake: writing batch (stub — wire to snowflake-connector-rs)"
        );
        // Production: use snowflake-connector-rs or COPY INTO via stage
        Ok(WriteResult {
            partition: partition.clone(),
            rows_written: records.len(),
            bytes_estimate: records.len() * 512,
        })
    }

    async fn ensure_table(&self, table_name: &str) -> Result<(), String> {
        info!(table = %table_name, "Snowflake: ensuring table exists (stub)");
        Ok(())
    }

    fn provider(&self) -> WarehouseProvider {
        WarehouseProvider::Snowflake
    }
}

// ── BigQuery adapter stub ─────────────────────────────────────────────────────

pub struct BigQueryAdapter {
    config: WarehouseConfig,
}

impl BigQueryAdapter {
    pub fn new(config: WarehouseConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl WarehouseAdapter for BigQueryAdapter {
    async fn write_batch(
        &self,
        partition: &PartitionKey,
        records: &[EtlRecord],
    ) -> Result<WriteResult, String> {
        let table = format!(
            "{}.{}.{}ledgers${}",
            self.config.project_or_account,
            self.config.dataset_or_schema,
            self.config.table_prefix,
            partition.value.replace('/', ""),
        );
        info!(
            table = %table,
            rows = records.len(),
            "BigQuery: writing batch (stub — wire to gcp-bigquery-rs)"
        );
        Ok(WriteResult {
            partition: partition.clone(),
            rows_written: records.len(),
            bytes_estimate: records.len() * 512,
        })
    }

    async fn ensure_table(&self, table_name: &str) -> Result<(), String> {
        info!(table = %table_name, "BigQuery: ensuring table / partition exists (stub)");
        Ok(())
    }

    fn provider(&self) -> WarehouseProvider {
        WarehouseProvider::BigQuery
    }
}

/// Factory: create the right adapter from config
pub fn create_adapter(config: WarehouseConfig) -> Box<dyn WarehouseAdapter> {
    match config.provider {
        WarehouseProvider::Snowflake => Box::new(SnowflakeAdapter::new(config)),
        WarehouseProvider::BigQuery => Box::new(BigQueryAdapter::new(config)),
        WarehouseProvider::NoOp | _ => Box::new(NoOpAdapter::new(config)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_pipeline::etl::LedgerSizeCategory;
    use std::collections::HashMap as HM;

    fn sample_etl_record(seq: u64) -> EtlRecord {
        EtlRecord {
            sequence: seq,
            hash: format!("h{seq}"),
            base_fee_xlm: 0.00001,
            base_reserve_xlm: 0.5,
            timestamp_epoch_ms: 0,
            date_partition: "2024-01-15".into(),
            hour_partition: 0,
            tx_success_rate: 1.0,
            avg_ops_per_tx: 2.0,
            ledger_size_category: LedgerSizeCategory::Small,
            pipeline_version: "1.0.0".into(),
            enriched_at: chrono::Utc::now(),
            tags: HM::new(),
        }
    }

    #[tokio::test]
    async fn test_noop_adapter_write_batch() {
        let adapter = NoOpAdapter::new(WarehouseConfig::noop());
        let records: Vec<EtlRecord> = (1..=5).map(sample_etl_record).collect();
        let key = PartitionKey { strategy: "bydate".into(), value: "2024/01/15".into() };
        let result = adapter.write_batch(&key, &records).await.unwrap();
        assert_eq!(result.rows_written, 5);
    }

    #[tokio::test]
    async fn test_factory_creates_noop() {
        let adapter = create_adapter(WarehouseConfig::noop());
        assert_eq!(adapter.provider(), WarehouseProvider::NoOp);
    }
}
