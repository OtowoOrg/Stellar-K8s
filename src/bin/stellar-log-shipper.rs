//! stellar-log-shipper — durable log-to-S3 sidecar
//!
//! Tails `/var/log/stellar/` (or `$LOG_DIR`), batches lines into gzip-compressed
//! chunks, and uploads them to S3.  Rotation triggers on either:
//! - `BATCH_SIZE_LINES` lines accumulated (default 5000), or
//! - `FLUSH_INTERVAL_SECS` seconds elapsed (default 60).
//!
//! # S3 key layout
//! ```
//! <prefix>/<node-name>/<YYYY-MM-DD>/<HH-MM-SS>-<seq>.log.gz
//! ```
//!
//! # Environment variables
//! | Variable | Default | Description |
//! |---|---|---|
//! | `LOG_DIR` | `/var/log/stellar` | Directory to tail |
//! | `S3_BUCKET` | — | Required. Target bucket |
//! | `S3_PREFIX` | `stellar-logs` | Key prefix |
//! | `S3_REGION` | `us-east-1` | AWS region |
//! | `NODE_NAME` | `unknown` | Injected by Kubernetes downward API |
//! | `BATCH_SIZE_LINES` | `5000` | Lines per gzip batch |
//! | `FLUSH_INTERVAL_SECS` | `60` | Max seconds between flushes |
//! | `AWS_ACCESS_KEY_ID` | — | Optional; use IRSA in production |
//! | `AWS_SECRET_ACCESS_KEY` | — | Optional; use IRSA in production |

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::sleep;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

struct Config {
    log_dir: PathBuf,
    s3_bucket: String,
    s3_prefix: String,
    s3_region: String,
    node_name: String,
    batch_size_lines: usize,
    flush_interval: Duration,
}

impl Config {
    fn from_env() -> Result<Self> {
        Ok(Self {
            log_dir: PathBuf::from(
                env::var("LOG_DIR").unwrap_or_else(|_| "/var/log/stellar".to_string()),
            ),
            s3_bucket: env::var("S3_BUCKET").context("S3_BUCKET env var required")?,
            s3_prefix: env::var("S3_PREFIX").unwrap_or_else(|_| "stellar-logs".to_string()),
            s3_region: env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
            node_name: env::var("NODE_NAME").unwrap_or_else(|_| "unknown".to_string()),
            batch_size_lines: env::var("BATCH_SIZE_LINES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5000),
            flush_interval: Duration::from_secs(
                env::var("FLUSH_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60),
            ),
        })
    }
}

// ---------------------------------------------------------------------------
// S3 upload (via AWS SDK v2 presigned PUT or aws-sdk-s3)
// We use reqwest + AWS Signature V4 to avoid pulling the full SDK.
// ---------------------------------------------------------------------------

/// Upload `data` (already gzip-compressed) to `s3://<bucket>/<key>`.
async fn upload_to_s3(
    client: &reqwest::Client,
    region: &str,
    bucket: &str,
    key: &str,
    data: Vec<u8>,
) -> Result<()> {
    // Build the S3 endpoint URL (path-style for compatibility with MinIO etc.)
    let url = format!("https://s3.{region}.amazonaws.com/{bucket}/{key}");

    let access_key = env::var("AWS_ACCESS_KEY_ID").unwrap_or_default();
    let secret_key = env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_default();
    let session_token = env::var("AWS_SESSION_TOKEN").ok();

    let now = chrono::Utc::now();
    let date_str = now.format("%Y%m%d").to_string();
    let datetime_str = now.format("%Y%m%dT%H%M%SZ").to_string();

    // SHA-256 of the payload
    let payload_hash = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&data);
        hex::encode(h.finalize())
    };

    // Canonical headers
    let host = format!("s3.{region}.amazonaws.com");
    let content_type = "application/gzip";

    let mut canonical_headers = format!(
        "content-type:{content_type}\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{datetime_str}\n"
    );
    let mut signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date".to_string();

    if let Some(ref tok) = session_token {
        canonical_headers.push_str(&format!("x-amz-security-token:{tok}\n"));
        signed_headers.push_str(";x-amz-security-token");
    }

    let canonical_request =
        format!("PUT\n/{bucket}/{key}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

    // String to sign
    let scope = format!("{date_str}/{region}/s3/aws4_request");
    let string_to_sign = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(canonical_request.as_bytes());
        format!(
            "AWS4-HMAC-SHA256\n{datetime_str}\n{scope}\n{}",
            hex::encode(h.finalize())
        )
    };

    // Signing key
    let signing_key = {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;

        let k_date = {
            let mut mac =
                HmacSha256::new_from_slice(format!("AWS4{secret_key}").as_bytes()).unwrap();
            mac.update(date_str.as_bytes());
            mac.finalize().into_bytes()
        };
        let k_region = {
            let mut mac = HmacSha256::new_from_slice(&k_date).unwrap();
            mac.update(region.as_bytes());
            mac.finalize().into_bytes()
        };
        let k_service = {
            let mut mac = HmacSha256::new_from_slice(&k_region).unwrap();
            mac.update(b"s3");
            mac.finalize().into_bytes()
        };
        let mut mac = HmacSha256::new_from_slice(&k_service).unwrap();
        mac.update(b"aws4_request");
        mac.finalize().into_bytes()
    };

    let signature = {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(&signing_key).unwrap();
        mac.update(string_to_sign.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    };

    let auth_header = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );

    let mut req = client
        .put(&url)
        .header("Content-Type", content_type)
        .header("x-amz-date", &datetime_str)
        .header("x-amz-content-sha256", &payload_hash)
        .header("Authorization", auth_header)
        .body(data);

    if let Some(tok) = session_token {
        req = req.header("x-amz-security-token", tok);
    }

    let resp = req.send().await.context("S3 PUT request failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("S3 PUT failed: HTTP {status}: {body}");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Batch builder
// ---------------------------------------------------------------------------

struct Batch {
    lines: Vec<String>,
    started_at: Instant,
}

impl Batch {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            started_at: Instant::now(),
        }
    }

    fn push(&mut self, line: String) {
        self.lines.push(line);
    }

    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Compress the batch into a gzip byte vector.
    fn compress(&self) -> Result<Vec<u8>> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        for line in &self.lines {
            enc.write_all(line.as_bytes())?;
            enc.write_all(b"\n")?;
        }
        Ok(enc.finish()?)
    }
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer().json())
        .with(EnvFilter::from_default_env())
        .init();

    let cfg = Config::from_env()?;
    info!(
        bucket = %cfg.s3_bucket,
        prefix = %cfg.s3_prefix,
        node  = %cfg.node_name,
        "stellar-log-shipper starting"
    );

    // Ensure log directory exists (it may not exist yet if the main container
    // hasn't started writing).
    tokio::fs::create_dir_all(&cfg.log_dir).await.ok();

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let seq = Arc::new(AtomicU64::new(0));

    loop {
        // Find the most recently modified .log file in the log directory.
        let log_file = find_latest_log(&cfg.log_dir).await;

        match log_file {
            Some(path) => {
                if let Err(e) = tail_and_ship(&cfg, &http, &path, Arc::clone(&seq)).await {
                    warn!("Log shipping error: {e}. Retrying in 5s.");
                    sleep(Duration::from_secs(5)).await;
                }
            }
            None => {
                // No log file yet — wait and retry.
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Find the most recently modified `.log` file in `dir`.
async fn find_latest_log(dir: &PathBuf) -> Option<PathBuf> {
    let mut read_dir = tokio::fs::read_dir(dir).await.ok()?;
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        if let Ok(meta) = entry.metadata().await {
            if let Ok(modified) = meta.modified() {
                if best.as_ref().map(|(_, t)| modified > *t).unwrap_or(true) {
                    best = Some((path, modified));
                }
            }
        }
    }

    best.map(|(p, _)| p)
}

/// Tail `log_file`, accumulate lines into batches, and upload each batch.
async fn tail_and_ship(
    cfg: &Config,
    http: &reqwest::Client,
    log_file: &PathBuf,
    seq: Arc<AtomicU64>,
) -> Result<()> {
    info!(file = %log_file.display(), "Tailing log file");

    let file = File::open(log_file)
        .await
        .with_context(|| format!("Cannot open {}", log_file.display()))?;

    let mut reader = BufReader::new(file).lines();
    let mut batch = Batch::new();
    let mut flush_deadline = Instant::now() + cfg.flush_interval;

    loop {
        // Non-blocking read with a short timeout so we can check the flush timer.
        let line = tokio::time::timeout(Duration::from_millis(500), reader.next_line()).await;

        match line {
            Ok(Ok(Some(l))) => {
                batch.push(l);
            }
            Ok(Ok(None)) => {
                // EOF — file may have been rotated; flush and re-discover.
                if !batch.is_empty() {
                    flush_batch(cfg, http, &batch, &seq).await;
                }
                return Ok(());
            }
            Ok(Err(e)) => {
                warn!("Read error: {e}");
                break;
            }
            Err(_) => {
                // Timeout — check flush conditions below.
            }
        }

        let should_flush =
            batch.lines.len() >= cfg.batch_size_lines || Instant::now() >= flush_deadline;

        if should_flush && !batch.is_empty() {
            flush_batch(cfg, http, &batch, &seq).await;
            batch = Batch::new();
            flush_deadline = Instant::now() + cfg.flush_interval;
        }
    }

    Ok(())
}

/// Compress the batch and upload it to S3.  Errors are logged but not fatal.
async fn flush_batch(cfg: &Config, http: &reqwest::Client, batch: &Batch, seq: &Arc<AtomicU64>) {
    let n = seq.fetch_add(1, Ordering::SeqCst);
    let now = chrono::Utc::now();
    let key = format!(
        "{}/{}/{}/{}.log.gz",
        cfg.s3_prefix,
        cfg.node_name,
        now.format("%Y-%m-%d"),
        now.format(&format!("%H-%M-%S-{n:06}")),
    );

    match batch.compress() {
        Ok(gz) => {
            info!(
                key = %key,
                lines = batch.lines.len(),
                bytes = gz.len(),
                "Uploading log batch"
            );
            if let Err(e) = upload_to_s3(http, &cfg.s3_region, &cfg.s3_bucket, &key, gz).await {
                error!("S3 upload failed for {key}: {e}");
            }
        }
        Err(e) => {
            error!("Compression failed: {e}");
        }
    }
}
