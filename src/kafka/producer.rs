use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rdkafka::error::KafkaError;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use tracing::warn;

use crate::config::{KafkaConfig, PartitionMode};

const FNV_OFFSET_BASIS_64: u64 = 0xcbf29ce484222325;
const FNV_PRIME_64: u64 = 0x100000001b3;

pub fn fnv1a_64(data: &[u8], seed: u64) -> u64 {
    let mut hash = FNV_OFFSET_BASIS_64 ^ seed;
    for b in data {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(FNV_PRIME_64);
    }
    hash
}

#[derive(Clone)]
pub struct PartitionSelector {
    inner: Arc<PartitionSelectorInner>,
}

struct PartitionSelectorInner {
    partition_count: AtomicUsize,
    seed: u64,
}

impl PartitionSelector {
    pub fn new(initial_count: usize, seed: u64) -> Self {
        Self {
            inner: Arc::new(PartitionSelectorInner {
                partition_count: AtomicUsize::new(initial_count.max(1)),
                seed,
            }),
        }
    }

    pub fn num_partitions(&self) -> usize {
        self.inner.partition_count.load(Ordering::Relaxed)
    }

    pub fn set_num_partitions(&self, n: usize) {
        self.inner.partition_count.store(n.max(1), Ordering::Relaxed);
    }

    pub fn partition(&self, key: &[u8]) -> usize {
        let count = self.num_partitions();
        let hash = fnv1a_64(key, self.inner.seed);
        (hash % count as u64) as usize
    }
}

pub struct KafkaProducer {
    producer: FutureProducer,
    topic: String,
    mode: PartitionMode,
    partition_selector: Option<PartitionSelector>,
    refresh_interval: Duration,
    refresh_task: Option<tokio::task::JoinHandle<()>>,
}

impl KafkaProducer {
    pub async fn new(config: &KafkaConfig, seed: u64) -> Result<Self, KafkaError> {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", &config.brokers)
            .set("message.timeout.ms", "5000")
            .create()?;

        let partition_selector = if config.partitioning == PartitionMode::Dynamic {
            let count = Self::fetch_partition_count(&producer, &config.topic)?;
            Some(PartitionSelector::new(count, seed))
        } else {
            None
        };

        Ok(Self {
            producer,
            topic: config.topic.clone(),
            mode: config.partitioning.clone(),
            partition_selector,
            refresh_interval: Duration::from_secs(config.metadata_refresh_interval_secs),
            refresh_task: None,
        })
    }

    fn fetch_partition_count(producer: &FutureProducer, topic: &str) -> Result<usize, KafkaError> {
        let metadata = producer.fetch_metadata(Some(topic), Duration::from_secs(5))?;
        let topic_meta = metadata.topics().iter().find(|t| t.name() == topic);
        if let Some(t) = topic_meta {
            Ok(t.partitions().len())
        } else {
            Err(KafkaError::MetadataFetch(
                rdkafka::error::RDKafkaErrorCode::UnknownTopicOrPartition,
            ))
        }
    }

    pub fn start_partition_refresh(&mut self) {
        if self.refresh_task.is_some() {
            return;
        }
        if let Some(selector) = self.partition_selector.clone() {
            let producer = self.producer.clone();
            let topic = self.topic.clone();
            let interval = self.refresh_interval;
            let task = tokio::spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                loop {
                    ticker.tick().await;
                    match Self::fetch_partition_count(&producer, &topic) {
                        Ok(n) => selector.set_num_partitions(n),
                        Err(e) => warn!("Kafka partition count refresh failed: {e}"),
                    }
                }
            });
            self.refresh_task = Some(task);
        }
    }

    async fn refresh_partition_count(&self) {
        if let Some(selector) = &self.partition_selector {
            match Self::fetch_partition_count(&self.producer, &self.topic) {
                Ok(n) => selector.set_num_partitions(n),
                Err(e) => warn!("Kafka partition count refresh failed: {e}"),
            }
        }
    }

    pub async fn send(&self, key: &[u8], payload: &[u8]) -> Result<(), KafkaError> {
        let partition = self.compute_partition(key);
        if let Err(e) = self.send_to_partition(partition, key, payload).await {
            if self.mode == PartitionMode::Dynamic {
                self.refresh_partition_count().await;
                let new_partition = self.compute_partition(key);
                if new_partition != partition {
                    return self.send_to_partition(new_partition, key, payload).await;
                }
            }
            Err(e)
        } else {
            Ok(())
        }
    }

    fn compute_partition(&self, key: &[u8]) -> usize {
        match &self.partition_selector {
            Some(selector) => selector.partition(key),
            None => 0,
        }
    }

    async fn send_to_partition(
        &self,
        partition: usize,
        key: &[u8],
        payload: &[u8],
    ) -> Result<(), KafkaError> {
        let record = FutureRecord::to(&self.topic)
            .partition(partition as i32)
            .key(key)
            .payload(payload);
        match self.producer.send(record, Duration::from_secs(1)).await {
            Ok(_) => Ok(()),
            Err((e, _)) => Err(e),
        }
    }
}

impl Drop for KafkaProducer {
    fn drop(&mut self) {
        if let Some(task) = self.refresh_task.take() {
            task.abort();
        }
    }
}
