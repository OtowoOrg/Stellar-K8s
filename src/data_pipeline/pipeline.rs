//! Main pipeline orchestration: Kafka consumer → ETL → multi-sink fan-out with DLQ.

use crate::data_pipeline::{
    config::PipelineConfig,
    lineage::LineageTracker,
    metrics::PipelineMetrics,
    sinks::{build_sinks, Sink},
};
use std::sync::Arc;
use tokio::sync::watch;
use tracing::info;

/// Handle returned by [`DataPipeline::start`] for monitoring and shutdown.
pub struct PipelineHandle {
    pub metrics: PipelineMetrics,
    pub lineage: LineageTracker,
    shutdown_tx: watch::Sender<bool>,
}

impl PipelineHandle {
    /// Signal the pipeline to stop consuming.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

/// The data pipeline.
pub struct DataPipeline {
    config: PipelineConfig,
}

impl DataPipeline {
    pub fn new(config: PipelineConfig) -> Self {
        Self { config }
    }

    /// Start the pipeline.  Returns a [`PipelineHandle`] immediately; the
    /// pipeline runs in a background Tokio task.
    ///
    /// When the `kafka` feature is disabled the pipeline runs in a no-op mode
    /// that is still useful for testing the ETL and sink layers.
    pub async fn start(self) -> Result<PipelineHandle, crate::error::Error> {
        let metrics = PipelineMetrics::default();
        let lineage = LineageTracker::new(10_000);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = PipelineHandle {
            metrics: metrics.clone(),
            lineage: lineage.clone(),
            shutdown_tx,
        };

        let config = Arc::new(self.config);
        let sinks = build_sinks(&config.sinks)
            .await
            .map_err(|e| crate::error::Error::ConfigError(e.to_string()))?;
        let sinks: Arc<Vec<Box<dyn Sink>>> = Arc::new(sinks);

        tokio::spawn(run_pipeline(config, sinks, metrics, lineage, shutdown_rx));

        Ok(handle)
    }
}

async fn run_pipeline(
    _config: Arc<PipelineConfig>,
    _sinks: Arc<Vec<Box<dyn Sink>>>,
    _metrics: PipelineMetrics,
    _lineage: LineageTracker,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    #[cfg(feature = "kafka")]
    {
        run_kafka_pipeline(config, sinks, metrics, lineage, shutdown_rx).await;
    }
    #[cfg(not(feature = "kafka"))]
    {
        info!("data pipeline started in no-op mode (kafka feature not enabled)");
        let _ = shutdown_rx.changed().await;
        info!("data pipeline stopped");
    }
}

#[cfg(feature = "kafka")]
async fn run_kafka_pipeline(
    config: Arc<PipelineConfig>,
    sinks: Arc<Vec<Box<dyn Sink>>>,
    metrics: PipelineMetrics,
    lineage: LineageTracker,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    use rdkafka::consumer::{Consumer, StreamConsumer};
    use rdkafka::message::Message as KafkaMessage;
    use rdkafka::producer::{FutureProducer, FutureRecord};
    use rdkafka::ClientConfig;
    use std::time::Duration;

    let mut client_config = ClientConfig::new();
    client_config
        .set("bootstrap.servers", &config.kafka.brokers)
        .set("group.id", &config.consumer_group)
        .set("security.protocol", &config.kafka.security_protocol)
        .set("enable.auto.commit", "true")
        .set("auto.offset.reset", "earliest");

    if let (Some(mech), Some(user), Some(pass)) = (
        &config.kafka.sasl_mechanism,
        &config.kafka.sasl_username,
        &config.kafka.sasl_password,
    ) {
        client_config
            .set("sasl.mechanism", mech)
            .set("sasl.username", user)
            .set("sasl.password", pass);
    }

    let consumer: StreamConsumer = match client_config.create() {
        Ok(c) => c,
        Err(e) => {
            error!("failed to create Kafka consumer: {e}");
            return;
        }
    };

    let topics: Vec<&str> = config.source_topics.iter().map(String::as_str).collect();
    if let Err(e) = consumer.subscribe(&topics) {
        error!("failed to subscribe to topics: {e}");
        return;
    }

    // DLQ producer
    let dlq_producer: FutureProducer = match client_config.create() {
        Ok(p) => p,
        Err(e) => {
            error!("failed to create DLQ producer: {e}");
            return;
        }
    };

    let transformer = EtlTransformer::new(config.etl.add_pipeline_metadata);
    info!(topics = ?config.source_topics, "data pipeline consuming");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                info!("data pipeline shutting down");
                break;
            }
            msg = consumer.recv() => {
                match msg {
                    Err(e) => {
                        warn!("kafka receive error: {e}");
                    }
                    Ok(m) => {
                        let start = Instant::now();
                        metrics.record_received();

                        let topic = m.topic().to_string();
                        let partition = m.partition();
                        let offset = m.offset();
                        let record_id = format!("{topic}:{partition}:{offset}");

                        lineage.record(&record_id, "kafka_source", LineageStatus::Received, None).await;

                        let payload = match m.payload() {
                            Some(p) => p,
                            None => {
                                warn!(%record_id, "empty kafka message payload");
                                continue;
                            }
                        };

                        // ETL transform
                        let etl_record = match transformer.transform(payload, &topic, partition, offset) {
                            Ok(r) => {
                                metrics.record_transformed();
                                lineage.record(&record_id, "etl", LineageStatus::Transformed, None).await;
                                r
                            }
                            Err(e) => {
                                metrics.record_transform_error();
                                lineage.record(&record_id, "etl", LineageStatus::ValidationFailed, Some(e.to_string())).await;
                                if !config.etl.drop_invalid {
                                    send_to_dlq(&dlq_producer, &config.dlq_topic, payload, &record_id, &e.to_string()).await;
                                    metrics.record_dlq();
                                    lineage.record(&record_id, "dlq", LineageStatus::DeadLettered, None).await;
                                }
                                continue;
                            }
                        };

                        // Fan-out to all sinks
                        let batch = std::slice::from_ref(&etl_record);
                        for sink in sinks.iter() {
                            match sink.write_batch(batch).await {
                                Ok(_) => {
                                    metrics.record_sink_success();
                                    lineage.record(&record_id, sink.name(), LineageStatus::SinkSuccess, None).await;
                                }
                                Err(e) => {
                                    metrics.record_sink_error();
                                    lineage.record(&record_id, sink.name(), LineageStatus::SinkFailed, Some(e.to_string())).await;
                                    error!(sink = sink.name(), %record_id, error = %e, "sink write failed");
                                    // Send to DLQ on sink failure
                                    send_to_dlq(&dlq_producer, &config.dlq_topic, payload, &record_id, &e.to_string()).await;
                                    metrics.record_dlq();
                                }
                            }
                        }

                        metrics.record_latency(start);
                    }
                }
            }
        }
    }
}

#[cfg(feature = "kafka")]
async fn send_to_dlq(
    producer: &rdkafka::producer::FutureProducer,
    dlq_topic: &str,
    payload: &[u8],
    record_id: &str,
    reason: &str,
) {
    use rdkafka::producer::FutureRecord;
    use std::time::Duration;

    let record = FutureRecord::to(dlq_topic)
        .key(record_id)
        .payload(payload)
        .headers(
            rdkafka::message::OwnedHeaders::new().insert(rdkafka::message::Header {
                key: "dlq_reason",
                value: Some(reason),
            }),
        );
    if let Err((e, _)) = producer.send(record, Duration::from_secs(5)).await {
        error!(dlq_topic, record_id, error = %e, "failed to send to DLQ");
    }
}
