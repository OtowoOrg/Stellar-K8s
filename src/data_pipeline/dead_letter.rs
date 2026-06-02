//! Dead-letter queue for failed pipeline records

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::warn;

/// A record that failed processing, stored for retry or inspection
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FailedRecord {
    pub ledger_sequence: u64,
    pub stage: String,
    pub error: String,
    pub retry_count: u32,
    pub max_retries: u32,
    pub failed_at: DateTime<Utc>,
    pub last_retry_at: Option<DateTime<Utc>>,
    pub payload: serde_json::Value,
}

impl FailedRecord {
    pub fn is_exhausted(&self) -> bool {
        self.retry_count >= self.max_retries
    }
}

/// Dead-letter queue with capacity cap and retry tracking
pub struct DeadLetterQueue {
    queue: Arc<RwLock<VecDeque<FailedRecord>>>,
    capacity: usize,
}

impl DeadLetterQueue {
    pub fn new(capacity: usize) -> Self {
        Self { queue: Arc::new(RwLock::new(VecDeque::new())), capacity }
    }

    /// Push a failed record into the DLQ
    pub async fn push(&self, record: FailedRecord) {
        let mut q = self.queue.write().await;
        if q.len() >= self.capacity {
            q.pop_front();
            warn!("DLQ capacity exceeded — dropping oldest failed record");
        }
        warn!(
            ledger = record.ledger_sequence,
            stage  = %record.stage,
            error  = %record.error,
            retry  = record.retry_count,
            "Record pushed to dead-letter queue"
        );
        q.push_back(record);
    }

    /// Drain records eligible for retry (not yet exhausted)
    pub async fn drain_for_retry(&self) -> Vec<FailedRecord> {
        let mut q = self.queue.write().await;
        let mut retryable = Vec::new();
        let mut retain = VecDeque::new();
        while let Some(record) = q.pop_front() {
            if record.is_exhausted() {
                retain.push_back(record);
            } else {
                retryable.push(record);
            }
        }
        *q = retain;
        retryable
    }

    /// Total records currently in DLQ
    pub async fn len(&self) -> usize {
        self.queue.read().await.len()
    }

    /// Records that have exhausted retries (permanently failed)
    pub async fn exhausted_count(&self) -> usize {
        self.queue.read().await.iter().filter(|r| r.is_exhausted()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_failed(seq: u64, retries: u32, max: u32) -> FailedRecord {
        FailedRecord {
            ledger_sequence: seq,
            stage: "etl".into(),
            error: "test error".into(),
            retry_count: retries,
            max_retries: max,
            failed_at: Utc::now(),
            last_retry_at: None,
            payload: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn test_push_and_len() {
        let dlq = DeadLetterQueue::new(10);
        dlq.push(make_failed(1, 0, 3)).await;
        dlq.push(make_failed(2, 0, 3)).await;
        assert_eq!(dlq.len().await, 2);
    }

    #[tokio::test]
    async fn test_drain_for_retry_excludes_exhausted() {
        let dlq = DeadLetterQueue::new(10);
        dlq.push(make_failed(1, 3, 3)).await; // exhausted
        dlq.push(make_failed(2, 1, 3)).await; // retryable
        let retryable = dlq.drain_for_retry().await;
        assert_eq!(retryable.len(), 1);
        assert_eq!(retryable[0].ledger_sequence, 2);
        assert_eq!(dlq.len().await, 1); // exhausted record remains
    }

    #[tokio::test]
    async fn test_capacity_cap() {
        let dlq = DeadLetterQueue::new(3);
        for i in 1..=5u64 {
            dlq.push(make_failed(i, 3, 3)).await;
        }
        assert_eq!(dlq.len().await, 3);
    }
}
