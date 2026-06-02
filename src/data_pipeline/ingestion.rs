//! Real-time Stellar ledger data ingestion from Stellar Core

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Configuration for the ledger stream
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamConfig {
    /// Stellar Core HTTP endpoint
    pub core_endpoint: String,
    /// Starting ledger sequence (0 = live)
    pub start_ledger: u64,
    /// Maximum records to buffer in memory
    pub buffer_size: usize,
    /// Polling interval for new ledgers
    pub poll_interval_ms: u64,
    /// Batch size for downstream processing
    pub batch_size: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            core_endpoint: "http://stellar-core:11626".into(),
            start_ledger: 0,
            buffer_size: 10_000,
            poll_interval_ms: 5_000,
            batch_size: 500,
        }
    }
}

/// Raw ledger record ingested from Stellar Core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerRecord {
    pub sequence: u64,
    pub hash: String,
    pub prev_hash: String,
    pub timestamp: DateTime<Utc>,
    pub base_fee: u64,
    pub base_reserve: u64,
    pub transaction_count: u32,
    pub operation_count: u32,
    pub successful_transaction_count: u32,
    pub failed_transaction_count: u32,
    pub tx_set_operation_count: u32,
    pub closed_at: DateTime<Utc>,
    /// Raw XDR bytes (base64-encoded)
    pub raw_xdr: Option<String>,
    pub ingest_time: DateTime<Utc>,
    pub source_node: String,
}

impl LedgerRecord {
    pub fn new(sequence: u64, hash: String, source_node: String) -> Self {
        let now = Utc::now();
        Self {
            sequence,
            hash,
            prev_hash: String::new(),
            timestamp: now,
            base_fee: 100,
            base_reserve: 5_000_000,
            transaction_count: 0,
            operation_count: 0,
            successful_transaction_count: 0,
            failed_transaction_count: 0,
            tx_set_operation_count: 0,
            closed_at: now,
            raw_xdr: None,
            ingest_time: now,
            source_node,
        }
    }
}

/// Ingestion metrics
#[derive(Clone, Debug, Default)]
pub struct IngestionMetrics {
    pub records_ingested: u64,
    pub records_dropped: u64,
    pub last_ledger_sequence: u64,
    pub ingestion_lag_ms: u64,
    pub buffer_utilization_pct: f64,
}

/// Real-time ledger ingestion engine
pub struct LedgerIngestion {
    config: StreamConfig,
    buffer: Arc<RwLock<VecDeque<LedgerRecord>>>,
    metrics: Arc<RwLock<IngestionMetrics>>,
    running: Arc<RwLock<bool>>,
}

impl LedgerIngestion {
    pub fn new(config: StreamConfig) -> Self {
        Self {
            buffer: Arc::new(RwLock::new(VecDeque::with_capacity(config.buffer_size))),
            config,
            metrics: Arc::new(RwLock::new(IngestionMetrics::default())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Simulate ingesting a ledger record (in production: call Stellar Core API)
    pub async fn ingest_record(&self, record: LedgerRecord) -> Result<(), String> {
        let mut buf = self.buffer.write().await;
        let mut metrics = self.metrics.write().await;

        if buf.len() >= self.config.buffer_size {
            buf.pop_front();
            metrics.records_dropped += 1;
            warn!(
                "Buffer full, dropping oldest record. Dropped total: {}",
                metrics.records_dropped
            );
        }

        metrics.last_ledger_sequence = record.sequence;
        metrics.records_ingested += 1;
        metrics.buffer_utilization_pct =
            buf.len() as f64 / self.config.buffer_size as f64 * 100.0;

        debug!(
            sequence = record.sequence,
            txs = record.transaction_count,
            "Ingested ledger"
        );
        buf.push_back(record);
        Ok(())
    }

    /// Drain a batch of records for downstream processing
    pub async fn drain_batch(&self) -> Vec<LedgerRecord> {
        let mut buf = self.buffer.write().await;
        let batch_size = self.config.batch_size.min(buf.len());
        buf.drain(..batch_size).collect()
    }

    /// Get current ingestion metrics
    pub async fn metrics(&self) -> IngestionMetrics {
        self.metrics.read().await.clone()
    }

    /// Start the ingestion loop (simulated — wires to Stellar Core in production)
    pub async fn start(&self) {
        *self.running.write().await = true;
        let interval = Duration::from_millis(self.config.poll_interval_ms);
        let mut seq = self.config.start_ledger;

        info!(
            endpoint = %self.config.core_endpoint,
            start_ledger = seq,
            "Starting ledger ingestion stream"
        );

        while *self.running.read().await {
            seq += 1;
            let record = LedgerRecord::new(
                seq,
                format!("ledger_hash_{seq:016x}"),
                self.config.core_endpoint.clone(),
            );
            if let Err(e) = self.ingest_record(record).await {
                warn!("Ingestion error at ledger {seq}: {e}");
            }
            tokio::time::sleep(interval).await;
        }
        info!("Ledger ingestion stream stopped");
    }

    pub async fn stop(&self) {
        *self.running.write().await = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ingest_and_drain() {
        let cfg = StreamConfig { buffer_size: 10, batch_size: 3, ..Default::default() };
        let ingest = LedgerIngestion::new(cfg);

        for i in 1u64..=5 {
            let r = LedgerRecord::new(i, format!("hash_{i}"), "test".into());
            ingest.ingest_record(r).await.unwrap();
        }

        let metrics = ingest.metrics().await;
        assert_eq!(metrics.records_ingested, 5);

        let batch = ingest.drain_batch().await;
        assert_eq!(batch.len(), 3);
        assert_eq!(batch[0].sequence, 1);
    }

    #[tokio::test]
    async fn test_buffer_overflow_drops_oldest() {
        let cfg = StreamConfig { buffer_size: 3, batch_size: 10, ..Default::default() };
        let ingest = LedgerIngestion::new(cfg);

        for i in 1u64..=5 {
            let r = LedgerRecord::new(i, format!("hash_{i}"), "test".into());
            ingest.ingest_record(r).await.unwrap();
        }

        let metrics = ingest.metrics().await;
        assert!(metrics.records_dropped >= 1);

        let batch = ingest.drain_batch().await;
        // Should contain only the most-recent 3 records
        assert_eq!(batch.len(), 3);
        assert!(batch[0].sequence >= 3);
    }
}
