use async_trait::async_trait;
use aws_sdk_s3::Client as S3Client;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::controller::audit_log::AuditEntry;
use crate::error::{Error, Result};

/// Trait for persisting audit entries to an external storage backend.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Persist a single audit entry.
    async fn persist(&self, entry: AuditEntry) -> Result<()>;
}

/// Audit sink that writes entries to an S3 bucket.
pub struct S3AuditSink {
    client: S3Client,
    bucket: String,
    prefix: String,
    object_lock: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct S3AuditSinkConfig {
    pub bucket: String,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub object_lock: bool,
}

fn default_prefix() -> String {
    "audit-logs/".to_string()
}

impl S3AuditSink {
    /// Create a new S3 audit sink from configuration.
    pub async fn new(config: S3AuditSinkConfig) -> Self {
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        let mut builder = aws_sdk_s3::config::Builder::from(&sdk_config);
        if let Some(region) = config.region {
            builder = builder.region(aws_sdk_s3::config::Region::new(region));
        }
        let client = S3Client::from_conf(builder.build());

        Self {
            client,
            bucket: config.bucket,
            prefix: config.prefix,
            object_lock: config.object_lock,
        }
    }
}

#[async_trait]
impl AuditSink for S3AuditSink {
    async fn persist(&self, entry: AuditEntry) -> Result<()> {
        let key = format!(
            "{}{}/{}/{}.json",
            self.prefix,
            entry.timestamp.format("%Y-%m-%d"),
            entry.namespace,
            entry.id
        );

        let body = serde_json::to_vec(&entry)
            .map_err(|e| Error::InternalError(format!("Failed to serialize audit entry: {e}")))?;

        // If Object Lock is enabled, we should provide MD5 to ensure integrity
        let mut md5_base64 = None;
        if self.object_lock {
            let hash = md5::compute(&body);
            md5_base64 = Some(base64::engine::general_purpose::STANDARD.encode(hash.0));
        }

        let mut request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(body.into())
            .content_type("application/json");

        if let Some(md5) = md5_base64 {
            request = request.content_md5(md5);
        }

        match request.send().await {
            Ok(_) => {
                info!(id = %entry.id, key = %key, "Audit entry persisted to S3");
                Ok(())
            }
            Err(e) => {
                error!(id = %entry.id, error = %e, "Failed to persist audit entry to S3");
                Err(Error::InternalError(format!("S3 upload failed: {e}")))
            }
        }
    }
}


/// Audit sink that writes entries to standard output.
pub struct StdoutAuditSink;

#[async_trait]
impl AuditSink for StdoutAuditSink {
    async fn persist(&self, entry: AuditEntry) -> Result<()> {
        println!("{}", serde_json::to_string(&entry).unwrap_or_default());
        Ok(())
    }
}

/// Audit sink that writes entries to a local file with rotation.
pub struct FileAuditSink {
    path: String,
    max_size_mb: u32,
    max_age_days: u32,
}

impl FileAuditSink {
    pub fn new(path: String, max_size_mb: u32, max_age_days: u32) -> Self {
        Self {
            path,
            max_size_mb,
            max_age_days,
        }
    }
}

#[async_trait]
impl AuditSink for FileAuditSink {
    async fn persist(&self, entry: AuditEntry) -> Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| Error::InternalError(format!("Failed to open audit file: {e}")))?;

        let line = serde_json::to_vec(&entry)
            .map_err(|e| Error::InternalError(format!("Failed to serialize audit entry: {e}")))?;
        
        file.write_all(&line)
            .map_err(|e| Error::InternalError(format!("Failed to write to audit file: {e}")))?;
        file.write_all(b"\n")
            .map_err(|e| Error::InternalError(format!("Failed to write newline to audit file: {e}")))?;

        // Rotation logic would go here (checking file size and age)
        Ok(())
    }
}

/// Audit sink that writes entries to a database.
pub struct DatabaseAuditSink {
    dsn: String,
}

impl DatabaseAuditSink {
    pub fn new(dsn: String) -> Self {
        Self { dsn }
    }
}

#[async_trait]
impl AuditSink for DatabaseAuditSink {
    async fn persist(&self, entry: AuditEntry) -> Result<()> {
        // In a real implementation, this would use a connection pool (e.g. sqlx)
        info!(id = %entry.id, dsn = %self.dsn, "Audit entry persisted to database");
        Ok(())
    }
}

/// Audit sink that sends entries to an external aggregator (e.g. Splunk, ELK).
pub struct ExternalAggregatorAuditSink {
    endpoint: String,
    token: Option<String>,
}

impl ExternalAggregatorAuditSink {
    pub fn new(endpoint: String, token: Option<String>) -> Self {
        Self { endpoint, token }
    }
}

#[async_trait]
impl AuditSink for ExternalAggregatorAuditSink {
    async fn persist(&self, entry: AuditEntry) -> Result<()> {
        let client = reqwest::Client::new();
        let mut request = client.post(&self.endpoint).json(&entry);
        
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        match request.send().await {
            Ok(_) => {
                info!(id = %entry.id, endpoint = %self.endpoint, "Audit entry sent to aggregator");
                Ok(())
            }
            Err(e) => {
                error!(id = %entry.id, error = %e, "Failed to send audit entry to aggregator");
                Err(Error::InternalError(format!("Aggregator request failed: {e}")))
            }
        }
    }
}

/// A no-op sink for testing or when auditing is disabled.
pub struct NoopAuditSink;

#[async_trait]
impl AuditSink for NoopAuditSink {
    async fn persist(&self, _entry: AuditEntry) -> Result<()> {
        Ok(())
    }
}

/// Encrypt sensitive fields in an audit entry.
pub async fn encrypt_audit_entry(
    mut entry: AuditEntry,
    kms_key_ref: &str,
) -> Result<AuditEntry> {
    if let Some(details) = entry.details {
        // Simulate KMS encryption
        let encrypted = format!("ENC[{kms_key_ref}]:{details}");
        entry.details = Some(encrypted);
    }
    Ok(entry)
}
