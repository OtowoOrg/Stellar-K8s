//! ETL transformations, schema validation, and data quality checks.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

/// A record flowing through the pipeline after ETL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtlRecord {
    /// Unique record ID (Kafka topic + partition + offset)
    pub id: String,
    /// Source Kafka topic
    pub source_topic: String,
    /// Kafka partition
    pub partition: i32,
    /// Kafka offset
    pub offset: i64,
    /// Transformed payload
    pub payload: Value,
    /// Pipeline metadata injected during ETL
    pub metadata: HashMap<String, String>,
    /// ISO-8601 timestamp when the record entered the pipeline
    pub pipeline_ts: String,
    /// Ledger sequence number extracted from payload (if present)
    pub ledger_seq: Option<u64>,
}

#[derive(Debug, Error)]
pub enum TransformError {
    #[error("schema validation failed: {0}")]
    SchemaValidation(String),
    #[error("data quality check failed: {field} — {reason}")]
    DataQuality { field: String, reason: String },
    #[error("transform error: {0}")]
    Transform(String),
}

/// Validates and transforms a raw Kafka message payload.
pub struct EtlTransformer {
    add_metadata: bool,
}

impl EtlTransformer {
    pub fn new(add_metadata: bool) -> Self {
        Self { add_metadata }
    }

    /// Transform raw bytes from Kafka into an [`EtlRecord`].
    pub fn transform(
        &self,
        raw: &[u8],
        topic: &str,
        partition: i32,
        offset: i64,
    ) -> Result<EtlRecord, TransformError> {
        // Parse JSON payload
        let mut payload: Value = serde_json::from_slice(raw)
            .map_err(|e| TransformError::SchemaValidation(e.to_string()))?;

        // Schema validation: require top-level object
        if !payload.is_object() {
            return Err(TransformError::SchemaValidation(
                "payload must be a JSON object".into(),
            ));
        }

        // Data quality checks
        Self::check_quality(&payload)?;

        // Normalize: ensure event_type field exists
        if payload.get("event_type").is_none() {
            payload["event_type"] = Value::String("unknown".into());
        }

        // Extract ledger sequence if present
        let ledger_seq = payload
            .get("ledger_sequence")
            .or_else(|| payload.get("ledger_seq"))
            .and_then(Value::as_u64);

        let mut metadata = HashMap::new();
        if self.add_metadata {
            metadata.insert("pipeline_version".into(), env!("CARGO_PKG_VERSION").into());
            metadata.insert("source_topic".into(), topic.into());
            metadata.insert("partition".into(), partition.to_string());
            metadata.insert("offset".into(), offset.to_string());
        }

        Ok(EtlRecord {
            id: format!("{topic}:{partition}:{offset}"),
            source_topic: topic.into(),
            partition,
            offset,
            payload,
            metadata,
            pipeline_ts: Utc::now().to_rfc3339(),
            ledger_seq,
        })
    }

    fn check_quality(payload: &Value) -> Result<(), TransformError> {
        let obj = payload.as_object().unwrap(); // already validated above

        // Ledger sequence must be a non-negative integer if present
        if let Some(seq) = obj.get("ledger_sequence").or_else(|| obj.get("ledger_seq")) {
            if seq.as_u64().is_none() {
                return Err(TransformError::DataQuality {
                    field: "ledger_sequence".into(),
                    reason: "must be a non-negative integer".into(),
                });
            }
        }

        // Timestamp field must be parseable if present
        if let Some(ts) = obj.get("timestamp").and_then(Value::as_str) {
            if chrono::DateTime::parse_from_rfc3339(ts).is_err() {
                return Err(TransformError::DataQuality {
                    field: "timestamp".into(),
                    reason: "must be RFC-3339 formatted".into(),
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_valid_record() {
        let t = EtlTransformer::new(true);
        let raw = br#"{"ledger_sequence": 42, "event_type": "transaction"}"#;
        let rec = t.transform(raw, "stellar.ledger.events", 0, 100).unwrap();
        assert_eq!(rec.ledger_seq, Some(42));
        assert_eq!(rec.offset, 100);
    }

    #[test]
    fn transform_invalid_json() {
        let t = EtlTransformer::new(false);
        assert!(t.transform(b"not json", "topic", 0, 0).is_err());
    }

    #[test]
    fn transform_bad_ledger_seq() {
        let t = EtlTransformer::new(false);
        let raw = br#"{"ledger_sequence": -1}"#;
        // -1 is not a valid u64, but serde_json parses it as i64 so as_u64() returns None
        assert!(t.transform(raw, "topic", 0, 0).is_err());
    }
}
