//! SHA-256 integrity verification for Stellar snapshot archives.
//!
//! Before a snapshot is extracted and used to bootstrap a secondary cluster node,
//! this module verifies that the downloaded bytes match the expected checksum.
//! Verification is done in a streaming fashion over the data already written to
//! disk — no second copy is kept in RAM.
//!
//! # Verification Flow
//!
//! ```text
//!  Cloud Storage
//!       │
//!       │  (1) download archive → streaming write to disk
//!       ▼
//!  /tmp/snapshot-<id>.tar.gz  ←── also fed into SHA-256 context during download
//!       │
//!       │  (2) verify_file(path, expected_hex) compares computed vs expected
//!       ▼
//!  VerificationResult { matched: true, … }
//!       │
//!       │  (3) reconciler proceeds with extraction only on success
//!       ▼
//!  Node data directory bootstrapped
//! ```
//!
//! # Checksum Sources
//!
//! Checksums can be supplied in three ways:
//! 1. Inline `expected_sha256` field in [`SnapshotRef`]
//! 2. A sidecar `.sha256` file stored next to the archive in cloud storage
//! 3. Embedded in the cloud storage object's metadata header `x-amz-checksum-sha256`
//!
//! If none are provided the download is rejected as unverifiable.

use std::fmt;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};
use tracing::{debug, instrument, warn};

use crate::error::{Error, Result};

// ──────────────────────────────────────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────────────────────────────────────

/// Outcome of a single integrity-verification pass over an archive file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    /// Path to the file that was verified.
    pub path: String,
    /// True when the computed digest matches the expected digest.
    pub matched: bool,
    /// Hex-encoded SHA-256 digest computed from the file bytes.
    pub computed_hex: String,
    /// Hex-encoded SHA-256 digest that was expected (provided by the caller).
    pub expected_hex: String,
    /// File size in bytes.
    pub file_size_bytes: u64,
    /// Wall-clock duration of the verification pass (milliseconds).
    pub elapsed_ms: u64,
    /// ISO-8601 timestamp of when verification completed.
    pub verified_at: String,
}

impl fmt::Display for VerificationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "VerificationResult {{ path={}, matched={}, size={}B, elapsed={}ms }}",
            self.path, self.matched, self.file_size_bytes, self.elapsed_ms
        )
    }
}

impl VerificationResult {
    /// Return an [`Error::ValidationError`] if the digest did not match.
    pub fn into_result(self) -> Result<Self> {
        if self.matched {
            Ok(self)
        } else {
            Err(Error::ValidationError(format!(
                "Snapshot integrity check FAILED for {}: expected={} computed={}",
                self.path, self.expected_hex, self.computed_hex
            )))
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Async verifier
// ──────────────────────────────────────────────────────────────────────────────

/// Verify the SHA-256 digest of the file at `path` against `expected_hex`.
///
/// Reads the file in 64 KiB chunks to keep memory usage constant regardless
/// of the archive size (supports 20 GB+ archives on nodes with limited RAM).
///
/// # Errors
///
/// Returns `Error::IoError` if the file cannot be opened or read.
/// Returns `Error::ValidationError` if the digest does not match.
#[instrument(skip_all, fields(path = %path.as_ref().display()))]
pub async fn verify_file(path: impl AsRef<Path>, expected_hex: &str) -> Result<VerificationResult> {
    let path_str = path.as_ref().display().to_string();
    let expected_hex = expected_hex.to_lowercase();

    debug!(path = %path_str, expected = %expected_hex, "starting SHA-256 verification");

    let file = File::open(path.as_ref()).await?;
    let metadata = file.metadata().await?;
    let file_size_bytes = metadata.len();

    let start = Instant::now();
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total_read: u64 = 0;

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total_read += n as u64;

        if total_read % (100 * 1024 * 1024) == 0 {
            debug!(bytes_read = total_read, "verification progress");
        }
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let computed = hasher.finalize();
    let computed_hex = hex::encode(computed);
    let matched = computed_hex == expected_hex;

    if !matched {
        warn!(
            path = %path_str,
            expected = %expected_hex,
            computed = %computed_hex,
            "SHA-256 mismatch — rejecting snapshot"
        );
    }

    let result = VerificationResult {
        path: path_str,
        matched,
        computed_hex,
        expected_hex,
        file_size_bytes,
        elapsed_ms,
        verified_at: Utc::now().to_rfc3339(),
    };

    result.into_result()
}

// ──────────────────────────────────────────────────────────────────────────────
// Synchronous helper (for use in non-async contexts / tests)
// ──────────────────────────────────────────────────────────────────────────────

/// Compute the SHA-256 hex digest of a file synchronously (blocking I/O).
///
/// Reads in 64 KiB chunks. Suitable for use in tests and CLI tooling.
pub fn compute_sha256_sync(path: impl AsRef<Path>) -> Result<String> {
    let mut file = std::fs::File::open(path.as_ref())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Parse a `.sha256` sidecar file into a `(filename, hex_digest)` pair.
///
/// Standard format: `<hex>  <filename>\n`
pub fn parse_sha256_sidecar(contents: &str) -> Option<(String, String)> {
    let line = contents.lines().next()?.trim();
    let mut parts = line.splitn(2, "  ");
    let hex = parts.next()?.trim().to_lowercase();
    let filename = parts.next()?.trim().to_string();
    if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Some((filename, hex))
    } else {
        None
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp(content: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content).unwrap();
        f.flush().unwrap();
        f
    }

    fn sha256_of(data: &[u8]) -> String {
        hex::encode(Sha256::digest(data))
    }

    #[test]
    fn compute_sha256_sync_matches_reference() {
        let data = b"hello stellar-k8s snapshot";
        let file = write_temp(data);
        let computed = compute_sha256_sync(file.path()).unwrap();
        assert_eq!(computed, sha256_of(data));
    }

    #[tokio::test]
    async fn verify_file_passes_when_digest_matches() {
        let data = b"snapshot archive bytes";
        let file = write_temp(data);
        let expected = sha256_of(data);
        let result = verify_file(file.path(), &expected).await;
        assert!(result.is_ok(), "should pass: {:?}", result);
        let r = result.unwrap();
        assert!(r.matched);
        assert_eq!(r.file_size_bytes, data.len() as u64);
    }

    #[tokio::test]
    async fn verify_file_fails_when_digest_mismatches() {
        let data = b"corrupted snapshot data";
        let file = write_temp(data);
        let wrong_hex = "a".repeat(64); // invalid digest
        let result = verify_file(file.path(), &wrong_hex).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("integrity check FAILED"));
    }

    #[tokio::test]
    async fn verify_file_works_on_large_synthetic_file() {
        // 4 MB synthetic archive (tests chunked reading)
        let data: Vec<u8> = (0u8..=255).cycle().take(4 * 1024 * 1024).collect();
        let file = write_temp(&data);
        let expected = sha256_of(&data);
        let result = verify_file(file.path(), &expected).await;
        assert!(result.is_ok());
    }

    #[test]
    fn parse_sha256_sidecar_parses_standard_format() {
        let line = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  archive.tar.gz\n";
        let (name, hex) = parse_sha256_sidecar(line).unwrap();
        assert_eq!(name, "archive.tar.gz");
        assert_eq!(hex, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn parse_sha256_sidecar_rejects_malformed_input() {
        assert!(parse_sha256_sidecar("not a checksum file").is_none());
        assert!(parse_sha256_sidecar("").is_none());
        // too-short hex
        assert!(parse_sha256_sidecar("deadbeef  file.tar.gz").is_none());
    }

    #[test]
    fn verification_result_display_includes_key_fields() {
        let r = VerificationResult {
            path: "/tmp/snap.tar.gz".to_string(),
            matched: true,
            computed_hex: "abc".to_string(),
            expected_hex: "abc".to_string(),
            file_size_bytes: 1024,
            elapsed_ms: 5,
            verified_at: "2026-08-30T00:00:00Z".to_string(),
        };
        let s = r.to_string();
        assert!(s.contains("matched=true"));
        assert!(s.contains("1024B"));
    }
}
