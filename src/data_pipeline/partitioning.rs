//! Data partitioning and indexing strategies for the Stellar data pipeline

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::etl::EtlRecord;

/// Partition key computed for a record
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PartitionKey {
    pub strategy: String,
    pub value: String,
}

impl PartitionKey {
    pub fn storage_path(&self) -> String {
        format!("{}/{}", self.strategy, self.value.replace(':', "/"))
    }
}

/// Partitioning strategy
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PartitionStrategy {
    /// Partition by date: YYYY/MM/DD
    ByDate,
    /// Partition by date + hour: YYYY/MM/DD/HH
    ByDateHour,
    /// Partition by ledger range bucket (N ledgers per bucket)
    ByLedgerRange { bucket_size: u64 },
    /// Partition by size category
    BySizeCategory,
}

impl PartitionStrategy {
    pub fn compute_key(&self, record: &EtlRecord) -> PartitionKey {
        let value = match self {
            Self::ByDate => {
                record.date_partition.replace('-', "/")
            }
            Self::ByDateHour => {
                format!("{}/{:02}", record.date_partition.replace('-', "/"), record.hour_partition)
            }
            Self::ByLedgerRange { bucket_size } => {
                let bucket = record.sequence / bucket_size;
                let start = bucket * bucket_size;
                let end = start + bucket_size - 1;
                format!("{start:010}-{end:010}")
            }
            Self::BySizeCategory => {
                format!("{:?}", record.ledger_size_category).to_lowercase()
            }
        };

        PartitionKey {
            strategy: format!("{self:?}").to_lowercase().split('{').next().unwrap_or("partition").trim().into(),
            value,
        }
    }

    /// Group a slice of records by their partition keys
    pub fn group<'a>(&self, records: &'a [EtlRecord]) -> HashMap<PartitionKey, Vec<&'a EtlRecord>> {
        let mut map: HashMap<PartitionKey, Vec<&EtlRecord>> = HashMap::new();
        for record in records {
            map.entry(self.compute_key(record)).or_default().push(record);
        }
        map
    }
}

impl std::fmt::Debug for PartitionStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ByDate => write!(f, "ByDate"),
            Self::ByDateHour => write!(f, "ByDateHour"),
            Self::ByLedgerRange { bucket_size } => {
                write!(f, "ByLedgerRange {{ bucket_size: {bucket_size} }}")
            }
            Self::BySizeCategory => write!(f, "BySizeCategory"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_pipeline::etl::LedgerSizeCategory;
    use std::collections::HashMap;

    fn make_record(seq: u64, date: &str, hour: u32) -> EtlRecord {
        EtlRecord {
            sequence: seq,
            hash: format!("h{seq}"),
            base_fee_xlm: 0.00001,
            base_reserve_xlm: 0.5,
            timestamp_epoch_ms: 0,
            date_partition: date.into(),
            hour_partition: hour,
            tx_success_rate: 1.0,
            avg_ops_per_tx: 2.0,
            ledger_size_category: LedgerSizeCategory::Small,
            pipeline_version: "1.0.0".into(),
            enriched_at: chrono::Utc::now(),
            tags: HashMap::new(),
        }
    }

    #[test]
    fn test_by_date_key() {
        let strategy = PartitionStrategy::ByDate;
        let record = make_record(1, "2024-06-15", 10);
        let key = strategy.compute_key(&record);
        assert_eq!(key.value, "2024/06/15");
    }

    #[test]
    fn test_by_date_hour_key() {
        let strategy = PartitionStrategy::ByDateHour;
        let record = make_record(1, "2024-06-15", 9);
        let key = strategy.compute_key(&record);
        assert_eq!(key.value, "2024/06/15/09");
    }

    #[test]
    fn test_ledger_range_bucket() {
        let strategy = PartitionStrategy::ByLedgerRange { bucket_size: 1000 };
        let record = make_record(1500, "2024-01-01", 0);
        let key = strategy.compute_key(&record);
        assert_eq!(key.value, "0000001000-0000001999");
    }

    #[test]
    fn test_group_by_date() {
        let strategy = PartitionStrategy::ByDate;
        let records = vec![
            make_record(1, "2024-01-01", 0),
            make_record(2, "2024-01-01", 12),
            make_record(3, "2024-01-02", 0),
        ];
        let groups = strategy.group(&records);
        assert_eq!(groups.len(), 2);
    }
}
