//! Multi-sink connectors: PostgreSQL, Elasticsearch, and S3.

use crate::data_pipeline::{config::SinksConfig, etl::EtlRecord};
use async_trait::async_trait;
use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum SinkError {
    #[error("postgres sink error: {0}")]
    Postgres(String),
    #[error("elasticsearch sink error: {0}")]
    Elasticsearch(String),
    #[error("s3 sink error: {0}")]
    S3(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Common interface for all pipeline sinks.
#[async_trait]
pub trait Sink: Send + Sync {
    async fn write_batch(&self, records: &[EtlRecord]) -> Result<usize, SinkError>;
    fn name(&self) -> &str;
}

// ── PostgreSQL Sink ───────────────────────────────────────────────────────────

pub struct PostgresSink {
    pool: sqlx::PgPool,
    table: String,
    batch_size: usize,
}

impl PostgresSink {
    pub async fn new(
        database_url: &str,
        table: impl Into<String>,
        batch_size: usize,
    ) -> Result<Self, SinkError> {
        let pool = sqlx::PgPool::connect(database_url)
            .await
            .map_err(|e| SinkError::Postgres(e.to_string()))?;
        Ok(Self {
            pool,
            table: table.into(),
            batch_size,
        })
    }
}

#[async_trait]
impl Sink for PostgresSink {
    fn name(&self) -> &str {
        "postgres"
    }

    async fn write_batch(&self, records: &[EtlRecord]) -> Result<usize, SinkError> {
        let mut written = 0;
        for chunk in records.chunks(self.batch_size) {
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| SinkError::Postgres(e.to_string()))?;
            for rec in chunk {
                let payload = serde_json::to_string(&rec.payload)?;
                sqlx::query(&format!(
                    "INSERT INTO {} (record_id, source_topic, partition, offset_val, payload, pipeline_ts, ledger_seq) \
                     VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7) ON CONFLICT (record_id) DO NOTHING",
                    self.table
                ))
                .bind(&rec.id)
                .bind(&rec.source_topic)
                .bind(rec.partition)
                .bind(rec.offset)
                .bind(&payload)
                .bind(&rec.pipeline_ts)
                .bind(rec.ledger_seq.map(|s| s as i64))
                .execute(&mut *tx)
                .await
                .map_err(|e| SinkError::Postgres(e.to_string()))?;
                written += 1;
            }
            tx.commit()
                .await
                .map_err(|e| SinkError::Postgres(e.to_string()))?;
        }
        info!(sink = "postgres", written, "batch written");
        Ok(written)
    }
}

// ── Elasticsearch Sink ────────────────────────────────────────────────────────

pub struct ElasticsearchSink {
    client: reqwest::Client,
    url: String,
    index: String,
    batch_size: usize,
}

impl ElasticsearchSink {
    pub fn new(url: impl Into<String>, index: impl Into<String>, batch_size: usize) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url.into(),
            index: index.into(),
            batch_size,
        }
    }

    /// Build an Elasticsearch bulk request body.
    fn build_bulk_body(records: &[EtlRecord]) -> Result<String, SinkError> {
        let mut body = String::new();
        for rec in records {
            let meta = serde_json::json!({ "index": { "_id": rec.id } });
            body.push_str(&serde_json::to_string(&meta)?);
            body.push('\n');
            body.push_str(&serde_json::to_string(rec)?);
            body.push('\n');
        }
        Ok(body)
    }
}

#[async_trait]
impl Sink for ElasticsearchSink {
    fn name(&self) -> &str {
        "elasticsearch"
    }

    async fn write_batch(&self, records: &[EtlRecord]) -> Result<usize, SinkError> {
        let mut written = 0;
        for chunk in records.chunks(self.batch_size) {
            let body = Self::build_bulk_body(chunk)?;
            let resp = self
                .client
                .post(format!("{}/_bulk", self.url))
                .header("Content-Type", "application/x-ndjson")
                .body(body)
                .send()
                .await
                .map_err(|e| SinkError::Elasticsearch(e.to_string()))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(SinkError::Elasticsearch(format!("{status}: {text}")));
            }
            written += chunk.len();
        }
        info!(sink = "elasticsearch", written, "batch written");
        Ok(written)
    }
}

// ── S3 Sink ───────────────────────────────────────────────────────────────────

pub struct S3Sink {
    client: aws_sdk_s3::Client,
    bucket: String,
    prefix: String,
    batch_size: usize,
}

impl S3Sink {
    pub async fn new(
        bucket: impl Into<String>,
        prefix: impl Into<String>,
        region: &str,
        batch_size: usize,
    ) -> Self {
        let config = aws_config::from_env()
            .region(aws_config::meta::region::RegionProviderChain::first_try(
                aws_sdk_s3::config::Region::new(region.to_string()),
            ))
            .load()
            .await;
        Self {
            client: aws_sdk_s3::Client::new(&config),
            bucket: bucket.into(),
            prefix: prefix.into(),
            batch_size,
        }
    }
}

#[async_trait]
impl Sink for S3Sink {
    fn name(&self) -> &str {
        "s3"
    }

    async fn write_batch(&self, records: &[EtlRecord]) -> Result<usize, SinkError> {
        let mut written = 0;
        for chunk in records.chunks(self.batch_size) {
            // Serialize chunk as newline-delimited JSON
            let mut body = String::new();
            for rec in chunk {
                body.push_str(&serde_json::to_string(rec)?);
                body.push('\n');
            }
            // Key: prefix/YYYY-MM-DD/first_record_id.ndjson
            let date = chrono::Utc::now().format("%Y-%m-%d");
            let first_id = chunk.first().map(|r| r.id.as_str()).unwrap_or("batch");
            let key = format!("{}{}/{}.ndjson", self.prefix, date, first_id);
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(&key)
                .body(aws_sdk_s3::primitives::ByteStream::from(body.into_bytes()))
                .content_type("application/x-ndjson")
                .send()
                .await
                .map_err(|e| SinkError::S3(e.to_string()))?;
            written += chunk.len();
        }
        info!(sink = "s3", written, "batch written");
        Ok(written)
    }
}

// ── Sink factory ─────────────────────────────────────────────────────────────

/// Build the list of enabled sinks from configuration.
pub async fn build_sinks(cfg: &SinksConfig) -> Result<Vec<Box<dyn Sink>>, SinkError> {
    let mut sinks: Vec<Box<dyn Sink>> = Vec::new();

    if let Some(pg) = &cfg.postgres {
        let sink = PostgresSink::new(&pg.database_url, &pg.table, pg.batch_size).await?;
        sinks.push(Box::new(sink));
    }

    if let Some(es) = &cfg.elasticsearch {
        sinks.push(Box::new(ElasticsearchSink::new(
            &es.url,
            &es.index,
            es.batch_size,
        )));
    }

    if let Some(s3) = &cfg.s3 {
        sinks.push(Box::new(
            S3Sink::new(&s3.bucket, &s3.prefix, &s3.region, s3.batch_size).await,
        ));
    }

    if sinks.is_empty() {
        warn!("no sinks configured — records will be consumed but not stored");
    }

    Ok(sinks)
}
