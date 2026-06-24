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
#[derive(Clone, Serialize, Deserialize)]
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
                // Extract date from pipeline_ts (ISO-8601 format)
                record
                    .pipeline_ts
                    .split('T')
                    .next()
                    .unwrap_or("unknown")
                    .replace('-', "/")
            }
            Self::ByDateHour => {
                // Extract date and hour from pipeline_ts
                let parts: Vec<&str> = record.pipeline_ts.split('T').collect();
                let date = parts.get(0).unwrap_or(&"unknown").replace('-', "/");
                let hour = parts
                    .get(1)
                    .and_then(|t| t.split(':').next())
                    .and_then(|h| h.parse::<u8>().ok())
                    .unwrap_or(0);
                format!("{}/{:02}", date, hour)
            }
            Self::ByLedgerRange { bucket_size } => {
                let sequence = record.ledger_seq.unwrap_or(0);
                let bucket = sequence / bucket_size;
                let start = bucket * bucket_size;
                let end = start + bucket_size - 1;
                format!("{start:010}-{end:010}")
            }
            Self::BySizeCategory => {
                // Extract size category from metadata or payload
                let category = record
                    .metadata
                    .get("size_category")
                    .map(|s| s.as_str())
                    .or_else(|| {
                        record
                            .payload
                            .get("ledger_size_category")
                            .and_then(|v| v.as_str())
                    })
                    .unwrap_or("unknown");
                category.to_lowercase()
            }
        };

        PartitionKey {
            strategy: format!("{self:?}")
                .to_lowercase()
                .split('{')
                .next()
                .unwrap_or("partition")
                .trim()
                .into(),
            value,
        }
    }

    /// Group a slice of records by their partition keys
    pub fn group<'a>(&self, records: &'a [EtlRecord]) -> HashMap<PartitionKey, Vec<&'a EtlRecord>> {
        let mut map: HashMap<PartitionKey, Vec<&EtlRecord>> = HashMap::new();
        for record in records {
            map.entry(self.compute_key(record))
                .or_default()
                .push(record);
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
    use std::collections::HashMap;

    fn make_record(seq: u64, date: &str, hour: u32) -> EtlRecord {
        let mut metadata = HashMap::new();
        metadata.insert("date_partition".into(), date.into());
        EtlRecord {
            id: format!("test-{seq}"),
            source_topic: "ledger".into(),
            partition: 0,
            offset: seq as i64,
            payload: serde_json::json!({
                "hash": format!("h{seq}"),
                "ledger_size_category": "small",
            }),
            metadata,
            pipeline_ts: format!("{date}T{hour:02}:00:00Z"),
            ledger_seq: Some(seq),
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
