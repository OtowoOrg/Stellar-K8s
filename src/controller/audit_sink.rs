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

/// A no-op sink for testing or when auditing is disabled.
pub struct NoopAuditSink;

#[async_trait]
impl AuditSink for NoopAuditSink {
    async fn persist(&self, _entry: AuditEntry) -> Result<()> {
        Ok(())
    }
}
