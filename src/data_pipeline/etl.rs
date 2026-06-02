//! ETL transformation layer for Stellar ledger data normalization and enrichment

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, warn};

use super::ingestion::LedgerRecord;

/// Result of applying ETL transformations to a ledger record
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EtlRecord {
    // Identity
    pub sequence: u64,
    pub hash: String,
    // Normalized fields
    pub base_fee_xlm: f64,
    pub base_reserve_xlm: f64,
    pub timestamp_epoch_ms: i64,
    pub date_partition: String, // YYYY-MM-DD
    pub hour_partition: u32,
    // Enriched fields
    pub tx_success_rate: f64,
    pub avg_ops_per_tx: f64,
    pub ledger_size_category: LedgerSizeCategory,
    // Metadata
    pub pipeline_version: String,
    pub enriched_at: DateTime<Utc>,
    pub tags: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LedgerSizeCategory {
    Empty,
    Small,
    Medium,
    Large,
    VeryLarge,
}

impl LedgerSizeCategory {
    fn from_tx_count(count: u32) -> Self {
        match count {
            0 => Self::Empty,
            1..=10 => Self::Small,
            11..=100 => Self::Medium,
            101..=500 => Self::Large,
            _ => Self::VeryLarge,
        }
    }
}

/// Result returned per record after transformation
#[derive(Debug)]
pub struct TransformResult {
    pub record: EtlRecord,
    pub applied_transforms: Vec<String>,
    pub warnings: Vec<String>,
}

const STROOPS_PER_XLM: f64 = 10_000_000.0;
const PIPELINE_VERSION: &str = "1.0.0";

/// ETL pipeline — applies ordered transforms to raw ledger records
pub struct EtlPipeline {
    pipeline_version: String,
    custom_tags: HashMap<String, String>,
}

impl EtlPipeline {
    pub fn new() -> Self {
        Self {
            pipeline_version: PIPELINE_VERSION.into(),
            custom_tags: HashMap::new(),
        }
    }

    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom_tags.insert(key.into(), value.into());
        self
    }

    /// Transform a single raw ledger record into an enriched EtlRecord
    pub fn transform(&self, raw: &LedgerRecord) -> TransformResult {
        let mut applied = Vec::new();
        let mut warnings = Vec::new();

        // ── Transform 1: normalize fees from stroops to XLM ──────────────────
        let base_fee_xlm = raw.base_fee as f64 / STROOPS_PER_XLM;
        let base_reserve_xlm = raw.base_reserve as f64 / STROOPS_PER_XLM;
        applied.push("normalize_fees".into());

        // ── Transform 2: derive time partitions ──────────────────────────────
        let timestamp_epoch_ms = raw.closed_at.timestamp_millis();
        let date_partition = raw.closed_at.format("%Y-%m-%d").to_string();
        let hour_partition = raw.closed_at.hour();
        applied.push("time_partitioning".into());

        // ── Transform 3: compute derived metrics ─────────────────────────────
        let tx_success_rate = if raw.transaction_count > 0 {
            raw.successful_transaction_count as f64 / raw.transaction_count as f64
        } else {
            1.0
        };

        let avg_ops_per_tx = if raw.transaction_count > 0 {
            raw.operation_count as f64 / raw.transaction_count as f64
        } else {
            0.0
        };

        if tx_success_rate < 0.5 {
            warnings.push(format!(
                "Low transaction success rate: {:.1}% at ledger {}",
                tx_success_rate * 100.0,
                raw.sequence
            ));
        }
        applied.push("derived_metrics".into());

        // ── Transform 4: ledger size categorisation ──────────────────────────
        let ledger_size_category = LedgerSizeCategory::from_tx_count(raw.transaction_count);
        applied.push("size_classification".into());

        // ── Transform 5: enrichment tags ─────────────────────────────────────
        let mut tags = self.custom_tags.clone();
        tags.insert("source_node".into(), raw.source_node.clone());
        tags.insert("network".into(), "mainnet".into());
        applied.push("tag_enrichment".into());

        debug!(
            sequence = raw.sequence,
            transforms = applied.len(),
            "ETL transforms applied"
        );

        TransformResult {
            record: EtlRecord {
                sequence: raw.sequence,
                hash: raw.hash.clone(),
                base_fee_xlm,
                base_reserve_xlm,
                timestamp_epoch_ms,
                date_partition,
                hour_partition,
                tx_success_rate,
                avg_ops_per_tx,
                ledger_size_category,
                pipeline_version: self.pipeline_version.clone(),
                enriched_at: Utc::now(),
                tags,
            },
            applied_transforms: applied,
            warnings,
        }
    }

    /// Transform a batch of raw records
    pub fn transform_batch(&self, records: &[LedgerRecord]) -> Vec<TransformResult> {
        records.iter().map(|r| self.transform(r)).collect()
    }
}

impl Default for EtlPipeline {
    fn default() -> Self {
        Self::new()
    }
}

// Bring DateTime methods into scope for hour_partition
trait HourExt {
    fn hour(&self) -> u32;
}
impl HourExt for DateTime<Utc> {
    fn hour(&self) -> u32 {
        use chrono::Timelike;
        chrono::Timelike::hour(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(seq: u64, tx_count: u32, success: u32) -> LedgerRecord {
        LedgerRecord {
            sequence: seq,
            hash: format!("hash_{seq}"),
            prev_hash: String::new(),
            timestamp: Utc::now(),
            base_fee: 100,
            base_reserve: 5_000_000,
            transaction_count: tx_count,
            operation_count: tx_count * 2,
            successful_transaction_count: success,
            failed_transaction_count: tx_count - success,
            tx_set_operation_count: tx_count * 2,
            closed_at: Utc::now(),
            raw_xdr: None,
            ingest_time: Utc::now(),
            source_node: "core-0".into(),
        }
    }

    #[test]
    fn test_fee_normalization() {
        let pipe = EtlPipeline::new();
        let raw = sample_record(1, 10, 9);
        let result = pipe.transform(&raw);
        assert!((result.record.base_fee_xlm - 0.00001).abs() < 1e-9);
        assert!((result.record.base_reserve_xlm - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_success_rate() {
        let pipe = EtlPipeline::new();
        let raw = sample_record(2, 100, 80);
        let result = pipe.transform(&raw);
        assert!((result.record.tx_success_rate - 0.8).abs() < 1e-9);
    }

    #[test]
    fn test_size_categorization() {
        let pipe = EtlPipeline::new();
        assert_eq!(
            pipe.transform(&sample_record(3, 0, 0)).record.ledger_size_category,
            LedgerSizeCategory::Empty
        );
        assert_eq!(
            pipe.transform(&sample_record(4, 5, 5)).record.ledger_size_category,
            LedgerSizeCategory::Small
        );
        assert_eq!(
            pipe.transform(&sample_record(5, 600, 600)).record.ledger_size_category,
            LedgerSizeCategory::VeryLarge
        );
    }

    #[test]
    fn test_all_transforms_applied() {
        let pipe = EtlPipeline::new();
        let result = pipe.transform(&sample_record(6, 10, 10));
        assert!(result.applied_transforms.contains(&"normalize_fees".to_string()));
        assert!(result.applied_transforms.contains(&"derived_metrics".to_string()));
        assert!(result.applied_transforms.contains(&"tag_enrichment".to_string()));
    }
}
