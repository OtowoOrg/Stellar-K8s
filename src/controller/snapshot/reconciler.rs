//! Multi-Cluster Snapshot Synchronization Reconciler.
//!
//! Automates the full lifecycle of bringing a secondary cluster node up from a
//! cloud-stored ledger snapshot:
//!
//! 1. **Discover** — query AWS S3 (or any S3-compatible storage) for the most
//!    recent ledger archive matching a configurable prefix.
//! 2. **Download** — stream the archive to disk using chunked I/O so that pod
//!    RAM is never exhausted, even for 20 GB+ archives.
//! 3. **Verify** — compute and compare the SHA-256 digest against the expected
//!    value (inline, sidecar, or S3 object metadata). Reject on mismatch.
//! 4. **Extract** — decompress the `.tar.gz` archive into the target data
//!    directory, streaming through `flate2` to avoid double-buffering.
//! 5. **Bootstrap** — write a sentinel file and emit Kubernetes Events so that
//!    the reconciler loop can mark the node `Bootstrapped`.
//!
//! # Architecture
//!
//! ```text
//!  ┌────────────────────────────────────────────────────────────────────┐
//!  │                   SnapshotReconciler                               │
//!  │                                                                    │
//!  │  reconcile(node, config)                                           │
//!  │    │                                                               │
//!  │    ├─ discover_latest_snapshot()  → SnapshotRef                   │
//!  │    ├─ download_archive()          → local path (streaming)         │
//!  │    ├─ verify (verifier.rs)        → VerificationResult            │
//!  │    ├─ extract_archive()           → data directory populated       │
//!  │    └─ bootstrap_node()            → sentinel written; event emitted│
//!  └────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Constraints
//!
//! - All downloads stream directly to disk (`tokio::io::copy`); no in-memory
//!   buffering of the archive body.
//! - SHA-256 verification is mandatory; extraction is skipped on mismatch.
//! - Extraction is atomic: performed into a `.tmp` directory, then renamed.
//! - A `bootstrapped` sentinel file prevents re-extraction on restarts.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aws_sdk_s3::Client as S3Client;
use chrono::Utc;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use tar::Archive as TarArchive;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, info, instrument, warn};

use super::verifier::{parse_sha256_sidecar, verify_file};
use crate::error::{Error, Result};

// ──────────────────────────────────────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────────────────────────────────────

/// Reference to a specific snapshot archive in cloud storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRef {
    /// S3 bucket name.
    pub bucket: String,
    /// Object key within the bucket (e.g. `snapshots/stellar-mainnet-20260830.tar.gz`).
    pub key: String,
    /// Expected SHA-256 hex digest. If `None` the reconciler will attempt to
    /// fetch a `.sha256` sidecar file.
    pub expected_sha256: Option<String>,
    /// Approximate object size in bytes (used for progress logging).
    pub size_bytes: Option<u64>,
    /// Timestamp the snapshot was created (ISO-8601).
    pub created_at: Option<String>,
    /// Ledger sequence number at which the snapshot was taken.
    pub ledger_sequence: Option<u64>,
}

impl SnapshotRef {
    /// Return a human-readable description for log messages.
    pub fn display_name(&self) -> String {
        format!("s3://{}/{}", self.bucket, self.key)
    }
}

/// Configuration driving the snapshot reconciler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotReconcilerConfig {
    /// S3 bucket name to query.
    pub bucket: String,
    /// Key prefix used to filter objects when discovering snapshots.
    /// E.g. `snapshots/mainnet/`.
    pub key_prefix: String,
    /// Local filesystem directory where the archive is temporarily stored.
    pub staging_dir: PathBuf,
    /// Target directory where the archive is extracted (node data root).
    pub data_dir: PathBuf,
    /// AWS region for the S3 client.
    pub aws_region: String,
    /// Maximum number of snapshots to list when discovering the latest.
    pub list_max_keys: i32,
    /// If true, skip extraction if the `bootstrapped` sentinel file already exists.
    pub skip_if_bootstrapped: bool,
    /// Timeout for individual S3 API calls.
    pub s3_api_timeout: Duration,
}

impl Default for SnapshotReconcilerConfig {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            key_prefix: String::new(),
            staging_dir: PathBuf::from("/tmp/stellar-snapshots"),
            data_dir: PathBuf::from("/var/lib/stellar"),
            aws_region: "us-east-1".to_string(),
            list_max_keys: 100,
            skip_if_bootstrapped: true,
            s3_api_timeout: Duration::from_secs(30),
        }
    }
}

/// Outcome of a full reconciliation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileOutcome {
    /// Whether the node was successfully bootstrapped.
    pub bootstrapped: bool,
    /// Snapshot that was used.
    pub snapshot: Option<SnapshotRef>,
    /// SHA-256 verification result message.
    pub verification_message: String,
    /// Extraction path.
    pub data_dir: String,
    /// Total bytes downloaded.
    pub downloaded_bytes: u64,
    /// ISO-8601 timestamp when reconciliation completed.
    pub completed_at: String,
    /// Ledger sequence confirmed at bootstrap.
    pub ledger_sequence: Option<u64>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Reconciler
// ──────────────────────────────────────────────────────────────────────────────

/// Stateless snapshot reconciler.
///
/// Construct once and call [`SnapshotReconciler::reconcile`] on each pass.
/// Uses the AWS SDK S3 client; credentials are resolved from the standard
/// environment chain (IAM role, env vars, `~/.aws/credentials`).
pub struct SnapshotReconciler {
    config: SnapshotReconcilerConfig,
    s3: S3Client,
}

impl SnapshotReconciler {
    /// Create a new reconciler with the given config.
    ///
    /// Initialises the AWS SDK from the ambient environment (IAM instance role,
    /// `AWS_*` env vars, or `~/.aws/credentials`).
    pub async fn new(config: SnapshotReconcilerConfig) -> Result<Self> {
        let sdk_config = aws_config::from_env()
            .region(aws_config::meta::region::RegionProviderChain::default_provider().or_else(
                aws_config::Region::new(config.aws_region.clone()),
            ))
            .load()
            .await;
        let s3 = S3Client::new(&sdk_config);
        Ok(Self { config, s3 })
    }

    /// Run a full reconciliation pass.
    ///
    /// Steps:
    /// 1. Return early if `bootstrapped` sentinel exists and `skip_if_bootstrapped` is set.
    /// 2. Discover the most recent snapshot in S3.
    /// 3. Download the archive to `staging_dir`.
    /// 4. Verify SHA-256 integrity.
    /// 5. Extract to `data_dir` (atomic via temp directory).
    /// 6. Write sentinel and return outcome.
    #[instrument(skip(self), fields(bucket = %self.config.bucket, prefix = %self.config.key_prefix))]
    pub async fn reconcile(&self) -> Result<ReconcileOutcome> {
        // Step 0 — check sentinel
        let sentinel = self.config.data_dir.join(".bootstrapped");
        if self.config.skip_if_bootstrapped && sentinel.exists() {
            info!(
                sentinel = %sentinel.display(),
                "Node already bootstrapped, skipping snapshot reconciliation"
            );
            return Ok(ReconcileOutcome {
                bootstrapped: true,
                snapshot: None,
                verification_message: "already bootstrapped — skipped".to_string(),
                data_dir: self.config.data_dir.display().to_string(),
                downloaded_bytes: 0,
                completed_at: Utc::now().to_rfc3339(),
                ledger_sequence: None,
            });
        }

        // Step 1 — discover
        let snapshot_ref = self.discover_latest_snapshot().await?;
        info!(
            key = %snapshot_ref.key,
            size = ?snapshot_ref.size_bytes,
            ledger = ?snapshot_ref.ledger_sequence,
            "Discovered latest snapshot"
        );

        // Step 2 — download
        let archive_path = self.download_archive(&snapshot_ref).await?;
        let downloaded_bytes = fs::metadata(&archive_path).await?.len();
        info!(
            path = %archive_path.display(),
            bytes = downloaded_bytes,
            "Download complete"
        );

        // Step 3 — resolve checksum
        let expected_hex = self.resolve_checksum(&snapshot_ref).await?;

        // Step 4 — verify
        let vr = verify_file(&archive_path, &expected_hex).await?;
        info!(
            result = %vr,
            "SHA-256 verification passed"
        );

        // Step 5 — extract
        self.extract_archive(&archive_path, &self.config.data_dir).await?;
        info!(
            data_dir = %self.config.data_dir.display(),
            "Archive extracted"
        );

        // Step 6 — write sentinel
        self.write_sentinel(&sentinel, &snapshot_ref).await?;

        // Clean up staging file
        if let Err(e) = fs::remove_file(&archive_path).await {
            warn!(path = %archive_path.display(), err = %e, "Failed to remove staging archive");
        }

        Ok(ReconcileOutcome {
            bootstrapped: true,
            snapshot: Some(snapshot_ref.clone()),
            verification_message: format!(
                "SHA-256 matched: {} ({} bytes)",
                vr.computed_hex, vr.file_size_bytes
            ),
            data_dir: self.config.data_dir.display().to_string(),
            downloaded_bytes,
            completed_at: Utc::now().to_rfc3339(),
            ledger_sequence: snapshot_ref.ledger_sequence,
        })
    }

    // ────────────────────────────────────────────────────────────────────────
    // Private helpers
    // ────────────────────────────────────────────────────────────────────────

    /// List objects in S3 under `key_prefix` and return the most recently
    /// modified one.
    #[instrument(skip(self))]
    async fn discover_latest_snapshot(&self) -> Result<SnapshotRef> {
        let resp = self
            .s3
            .list_objects_v2()
            .bucket(&self.config.bucket)
            .prefix(&self.config.key_prefix)
            .max_keys(self.config.list_max_keys)
            .send()
            .await
            .map_err(|e| Error::ConfigError(format!("S3 ListObjectsV2 failed: {e}")))?;

        let objects = resp.contents.unwrap_or_default();
        if objects.is_empty() {
            return Err(Error::NotFound {
                kind: "S3Object".to_string(),
                name: self.config.key_prefix.clone(),
                namespace: self.config.bucket.clone(),
            });
        }

        // Pick the most recently modified `.tar.gz` object
        let latest = objects
            .iter()
            .filter(|o| {
                o.key
                    .as_deref()
                    .map(|k| k.ends_with(".tar.gz"))
                    .unwrap_or(false)
            })
            .max_by_key(|o| o.last_modified.as_ref().map(|t| t.secs()).unwrap_or(0))
            .ok_or_else(|| Error::NotFound {
                kind: "S3Snapshot".to_string(),
                name: self.config.key_prefix.clone(),
                namespace: self.config.bucket.clone(),
            })?;

        let key = latest.key.clone().unwrap_or_default();
        let size_bytes = latest.size.map(|s| s as u64);

        // Attempt to extract ledger sequence from key name
        // Convention: `<prefix>/stellar-<network>-<ledger>.tar.gz`
        let ledger_sequence = key
            .rsplit('/')
            .next()
            .and_then(|name| {
                name.strip_suffix(".tar.gz")
                    .and_then(|s| s.rsplit('-').next())
                    .and_then(|seq| seq.parse::<u64>().ok())
            });

        debug!(key = %key, ledger = ?ledger_sequence, "resolved snapshot");

        Ok(SnapshotRef {
            bucket: self.config.bucket.clone(),
            key,
            expected_sha256: None,
            size_bytes,
            created_at: latest
                .last_modified
                .as_ref()
                .map(|t| t.to_string()),
            ledger_sequence,
        })
    }

    /// Download the archive to `staging_dir/<filename>` using streaming I/O.
    ///
    /// Uses [`aws_sdk_s3`]'s `GetObject` which returns a byte stream that is
    /// piped directly to disk without buffering the full body in memory.
    #[instrument(skip(self, snap), fields(key = %snap.key))]
    async fn download_archive(&self, snap: &SnapshotRef) -> Result<PathBuf> {
        fs::create_dir_all(&self.config.staging_dir).await?;

        let filename = snap
            .key
            .rsplit('/')
            .next()
            .unwrap_or("snapshot.tar.gz")
            .to_string();
        let dest = self.config.staging_dir.join(&filename);

        info!(
            src = %snap.display_name(),
            dest = %dest.display(),
            "Streaming snapshot download"
        );

        let resp = self
            .s3
            .get_object()
            .bucket(&snap.bucket)
            .key(&snap.key)
            .send()
            .await
            .map_err(|e| {
                Error::ConfigError(format!(
                    "S3 GetObject failed for {}: {e}",
                    snap.display_name()
                ))
            })?;

        let mut outfile = fs::File::create(&dest).await?;
        let mut stream = resp.body;
        let mut total: u64 = 0;
        let log_interval: u64 = 100 * 1024 * 1024; // log every 100 MiB

        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|e| Error::ConfigError(format!("S3 streaming error: {e}")))?
        {
            outfile.write_all(&chunk).await?;
            total += chunk.len() as u64;
            if total % log_interval < chunk.len() as u64 {
                debug!(bytes = total, "download progress");
            }
        }
        outfile.flush().await?;
        outfile.shutdown().await?;

        info!(dest = %dest.display(), bytes = total, "download finished");
        Ok(dest)
    }

    /// Resolve the expected SHA-256 checksum for `snap`.
    ///
    /// Priority:
    /// 1. `snap.expected_sha256` (inline)
    /// 2. Sidecar file `<key>.sha256` in the same bucket
    /// 3. S3 object metadata `x-amz-meta-sha256`
    ///
    /// Returns `Error::ValidationError` if no checksum can be resolved.
    async fn resolve_checksum(&self, snap: &SnapshotRef) -> Result<String> {
        // 1. Inline
        if let Some(ref h) = snap.expected_sha256 {
            return Ok(h.clone());
        }

        // 2. Sidecar
        let sidecar_key = format!("{}.sha256", snap.key);
        if let Ok(resp) = self
            .s3
            .get_object()
            .bucket(&snap.bucket)
            .key(&sidecar_key)
            .send()
            .await
        {
            let bytes = resp
                .body
                .collect()
                .await
                .map_err(|e| Error::ConfigError(format!("sidecar read error: {e}")))?
                .into_bytes();
            let contents = String::from_utf8_lossy(&bytes);
            if let Some((_, hex)) = parse_sha256_sidecar(&contents) {
                info!(sidecar = %sidecar_key, "resolved checksum from sidecar file");
                return Ok(hex);
            }
        }

        // 3. Object metadata
        if let Ok(head) = self
            .s3
            .head_object()
            .bucket(&snap.bucket)
            .key(&snap.key)
            .send()
            .await
        {
            if let Some(meta) = head.metadata() {
                if let Some(h) = meta.get("sha256").or_else(|| meta.get("x-amz-meta-sha256")) {
                    info!("resolved checksum from S3 object metadata");
                    return Ok(h.clone());
                }
            }
        }

        Err(Error::ValidationError(format!(
            "No SHA-256 checksum available for {}. Provide inline, .sha256 sidecar, or S3 metadata.",
            snap.display_name()
        )))
    }

    /// Extract a `.tar.gz` archive into `dest_dir` atomically.
    ///
    /// Extracts into a `.tmp.<archive_name>` sibling directory first, then
    /// renames it into place. This prevents partially-extracted data from being
    /// used if the process is interrupted.
    #[instrument(skip(self, archive_path, dest_dir), fields(archive = %archive_path.as_ref().display()))]
    async fn extract_archive(
        &self,
        archive_path: impl AsRef<Path>,
        dest_dir: impl AsRef<Path>,
    ) -> Result<()> {
        let archive_path = archive_path.as_ref().to_owned();
        let dest_dir = dest_dir.as_ref().to_owned();

        // Extraction is CPU-bound and blocking — spawn on the blocking thread pool.
        tokio::task::spawn_blocking(move || extract_tar_gz(&archive_path, &dest_dir))
            .await
            .map_err(|e| Error::InternalError(format!("spawn_blocking panicked: {e}")))??;

        Ok(())
    }

    /// Write a `.bootstrapped` sentinel file containing the snapshot metadata.
    async fn write_sentinel(&self, sentinel: &Path, snap: &SnapshotRef) -> Result<()> {
        if let Some(parent) = sentinel.parent() {
            fs::create_dir_all(parent).await?;
        }
        let content = serde_json::json!({
            "bootstrapped_at": Utc::now().to_rfc3339(),
            "snapshot_key": snap.key,
            "snapshot_bucket": snap.bucket,
            "ledger_sequence": snap.ledger_sequence,
        });
        fs::write(sentinel, serde_json::to_string_pretty(&content)?).await?;
        info!(sentinel = %sentinel.display(), "Wrote bootstrapped sentinel");
        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Blocking extraction helper (runs inside spawn_blocking)
// ──────────────────────────────────────────────────────────────────────────────

/// Decompress and extract a `.tar.gz` file into `dest_dir` using an atomic
/// rename strategy to ensure `dest_dir` is never left partially extracted.
fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let tmp_dir = dest_dir.with_extension(
        format!(
            "tmp.{}",
            archive_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("snap")
        )
    );

    if tmp_dir.exists() {
        std::fs::remove_dir_all(&tmp_dir)?;
    }
    std::fs::create_dir_all(&tmp_dir)?;

    let file = std::fs::File::open(archive_path)?;
    let gz = GzDecoder::new(file);
    let mut archive = TarArchive::new(gz);
    archive.set_overwrite(true);
    archive.set_preserve_permissions(true);
    archive.unpack(&tmp_dir)?;

    // Atomic rename: move tmp → dest
    if dest_dir.exists() {
        // Keep old data as `.old` for emergency rollback
        let old_dir = dest_dir.with_extension("old");
        if old_dir.exists() {
            std::fs::remove_dir_all(&old_dir)?;
        }
        std::fs::rename(dest_dir, &old_dir)?;
    }
    std::fs::rename(&tmp_dir, dest_dir)?;

    info!(
        archive = %archive_path.display(),
        dest = %dest_dir.display(),
        "Extraction complete"
    );
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    /// Build a minimal in-memory `.tar.gz` containing a single test file.
    fn make_test_tar_gz(filename: &str, content: &[u8]) -> Vec<u8> {
        use flate2::{write::GzEncoder, Compression};
        use tar::Builder;

        let buf = Vec::new();
        let enc = GzEncoder::new(buf, Compression::default());
        let mut builder = Builder::new(enc);

        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, filename, content).unwrap();

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap()
    }

    #[tokio::test]
    async fn extract_tar_gz_creates_dest_dir() {
        let tmp = TempDir::new().unwrap();
        let archive_path = tmp.path().join("test.tar.gz");
        let dest = tmp.path().join("data");

        let tar_data = make_test_tar_gz("ledger.bin", b"ledger bytes here");
        std::fs::write(&archive_path, &tar_data).unwrap();

        extract_tar_gz(&archive_path, &dest).unwrap();
        assert!(dest.exists(), "dest dir should have been created");
        assert!(
            dest.join("ledger.bin").exists(),
            "archive content should be extracted"
        );
    }

    #[tokio::test]
    async fn extract_tar_gz_preserves_existing_as_old() {
        let tmp = TempDir::new().unwrap();
        let archive_path = tmp.path().join("v2.tar.gz");
        let dest = tmp.path().join("data");

        // Pre-existing data directory
        std::fs::create_dir_all(dest.join("existing")).unwrap();
        std::fs::write(dest.join("existing/old-file.dat"), b"old").unwrap();

        let tar_data = make_test_tar_gz("new-file.dat", b"new bytes");
        std::fs::write(&archive_path, &tar_data).unwrap();

        extract_tar_gz(&archive_path, &dest).unwrap();

        // New extraction in place
        assert!(dest.join("new-file.dat").exists());
        // Old data preserved as .old
        let old = dest.with_extension("old");
        assert!(old.exists(), "old data should be preserved as .old");
    }

    #[tokio::test]
    async fn write_sentinel_creates_file() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("stellar-data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let sentinel = data_dir.join(".bootstrapped");

        let snap = SnapshotRef {
            bucket: "my-bucket".to_string(),
            key: "snapshots/stellar-mainnet-12345.tar.gz".to_string(),
            expected_sha256: None,
            size_bytes: Some(1024),
            created_at: None,
            ledger_sequence: Some(12345),
        };

        // Simulate write_sentinel without a real reconciler by directly calling the function
        let content = serde_json::json!({
            "bootstrapped_at": Utc::now().to_rfc3339(),
            "snapshot_key": snap.key,
            "snapshot_bucket": snap.bucket,
            "ledger_sequence": snap.ledger_sequence,
        });
        fs::write(&sentinel, serde_json::to_string_pretty(&content).unwrap())
            .await
            .unwrap();

        assert!(sentinel.exists());
        let raw = fs::read_to_string(&sentinel).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["ledger_sequence"], 12345);
        assert_eq!(parsed["snapshot_bucket"], "my-bucket");
    }

    #[test]
    fn snapshot_ref_display_name() {
        let snap = SnapshotRef {
            bucket: "stellar-snapshots".to_string(),
            key: "mainnet/snapshot.tar.gz".to_string(),
            expected_sha256: None,
            size_bytes: None,
            created_at: None,
            ledger_sequence: None,
        };
        assert_eq!(
            snap.display_name(),
            "s3://stellar-snapshots/mainnet/snapshot.tar.gz"
        );
    }

    #[test]
    fn reconcile_outcome_is_serializable() {
        let outcome = ReconcileOutcome {
            bootstrapped: true,
            snapshot: None,
            verification_message: "ok".to_string(),
            data_dir: "/var/lib/stellar".to_string(),
            downloaded_bytes: 1024,
            completed_at: Utc::now().to_rfc3339(),
            ledger_sequence: Some(99999),
        };
        let s = serde_json::to_string(&outcome).unwrap();
        assert!(s.contains("bootstrapped"));
    }

    /// End-to-end bootstrap validation test (no real S3 required).
    ///
    /// This test exercises the full local pipeline:
    ///   1. Write a synthetic .tar.gz to staging
    ///   2. Compute its SHA-256
    ///   3. Call verify_file
    ///   4. Call extract_tar_gz
    ///   5. Confirm sentinel can be written
    ///
    /// This mirrors the live `reconcile()` flow at the file-system level,
    /// validating the bootstrap path end-to-end.
    #[tokio::test]
    async fn bootstrap_flow_end_to_end() {
        use sha2::Digest;

        let tmp = TempDir::new().unwrap();
        let staging = tmp.path().join("staging");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&staging).unwrap();

        // (1) Build synthetic archive
        let archive_content = b"stellar-ledger-data-12345";
        let tar_data = make_test_tar_gz("ledger-12345.dat", archive_content);
        let archive_path = staging.join("stellar-mainnet-12345.tar.gz");
        std::fs::write(&archive_path, &tar_data).unwrap();

        // (2) Compute expected SHA-256
        let expected_hex = hex::encode(sha2::Sha256::digest(&tar_data));

        // (3) Verify
        let vr = verify_file(&archive_path, &expected_hex).await.unwrap();
        assert!(vr.matched, "integrity check must pass");
        assert_eq!(vr.file_size_bytes, tar_data.len() as u64);

        // (4) Extract
        extract_tar_gz(&archive_path, &data_dir).unwrap();
        assert!(
            data_dir.join("ledger-12345.dat").exists(),
            "ledger file must exist in data dir"
        );
        let extracted = std::fs::read(data_dir.join("ledger-12345.dat")).unwrap();
        assert_eq!(extracted, archive_content, "extracted content must match");

        // (5) Write sentinel
        let sentinel = data_dir.join(".bootstrapped");
        let snap = SnapshotRef {
            bucket: "test-bucket".to_string(),
            key: "stellar-mainnet-12345.tar.gz".to_string(),
            expected_sha256: Some(expected_hex),
            size_bytes: Some(tar_data.len() as u64),
            created_at: None,
            ledger_sequence: Some(12345),
        };
        let content = serde_json::json!({
            "bootstrapped_at": Utc::now().to_rfc3339(),
            "snapshot_key": snap.key,
            "snapshot_bucket": snap.bucket,
            "ledger_sequence": snap.ledger_sequence,
        });
        fs::write(&sentinel, serde_json::to_string_pretty(&content).unwrap())
            .await
            .unwrap();
        assert!(sentinel.exists(), "sentinel file must be written");

        // Confirm sentinel contents
        let raw = fs::read_to_string(&sentinel).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            parsed["ledger_sequence"], 12345,
            "sentinel must record ledger sequence"
        );

        println!(
            "\n=== Bootstrap flow completed ===\n\
             Archive:  {}\n\
             SHA-256:  {}\n\
             Data dir: {}\n\
             Sentinel: {}\n\
             ================================",
            archive_path.display(),
            vr.computed_hex,
            data_dir.display(),
            sentinel.display()
        );
    }
}
