//! Zero-Knowledge Archive Verifier
//!
//! Verifies the integrity and completeness of encrypted Stellar history archives
//! without requiring access to decryption keys. Uses a signed manifest and
//! hash-chain scheme: the operator only reads the manifest JSON, never the
//! ciphertext, and gains no knowledge of ledger contents.
//!
//! # How it works
//!
//! The archive publisher generates an `archive-manifest.json` alongside the
//! encrypted checkpoints. The manifest lists every checkpoint's sequence number
//! and its SHA-256 file hash, links them into a forward hash-chain, and signs
//! the whole body with an Ed25519 key. The verifier:
//!
//! 1. Downloads only the manifest (not the encrypted data).
//! 2. Verifies the Ed25519 signature.
//! 3. Walks the checkpoint list in order, detecting gaps (missing ledger ranges).
//! 4. Recomputes the hash chain and flags any broken links.
//!
//! A result is `verified = true` only when all three checks pass.

use crate::error::{Error, Result};
use ed25519_dalek::{Signature, VerifyingKey};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tracing::{debug, info, warn};

/// One entry in the archive manifest, covering a single Stellar checkpoint.
///
/// Checkpoints occur every 64 ledgers. `prev_hash` forms a forward chain:
/// `entry[i].prev_hash = SHA-256(entry[i-1].sequence || ":" || entry[i-1].file_hash)`.
/// The first entry uses `prev_hash = "0"` (genesis sentinel).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointEntry {
    /// Checkpoint ledger sequence number (always a multiple of 64 minus 1, e.g. 63, 127, …)
    pub sequence: u64,
    /// Hex-encoded SHA-256 of the encrypted checkpoint file
    pub file_hash: String,
    /// Hex-encoded SHA-256 linking this entry to the previous one in the chain
    pub prev_hash: String,
}

/// The signed body of an archive manifest.
///
/// This is the exact byte representation that the Ed25519 signature covers.
/// Stored separately so that signature verification is unambiguous.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestBody {
    /// Manifest format version (currently 1)
    pub version: u32,
    /// URL of the archive this manifest describes
    pub archive_url: String,
    /// The latest ledger included in the archive at the time of signing
    pub current_ledger: u64,
    /// Ordered list of all checkpoints from oldest to newest
    pub checkpoints: Vec<CheckpointEntry>,
}

/// Full `archive-manifest.json` as stored in the archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveManifest {
    #[serde(flatten)]
    pub body: ManifestBody,
    /// Hex-encoded Ed25519 public key used to sign this manifest
    pub signer_key: String,
    /// Hex-encoded Ed25519 signature over the canonical JSON of `body`
    pub signature: String,
}

/// Outcome of a ZK verification run for one archive URL.
#[derive(Debug, Clone)]
pub struct ZkVerificationResult {
    /// The archive URL that was verified
    pub url: String,
    /// `true` only when signature, chain, and gap checks all pass
    pub verified: bool,
    /// Whether the Ed25519 signature was valid
    pub signature_valid: bool,
    /// Whether the SHA-256 hash chain is unbroken
    pub chain_intact: bool,
    /// Ledger ranges that are missing: `(first_missing_seq, last_missing_seq)`
    pub gaps: Vec<(u64, u64)>,
    /// Number of checkpoint entries that were examined
    pub checkpoints_verified: u64,
    /// Human-readable summary of the result
    pub message: String,
}

impl ZkVerificationResult {
    fn ok(url: &str, count: u64) -> Self {
        ZkVerificationResult {
            url: url.to_string(),
            verified: true,
            signature_valid: true,
            chain_intact: true,
            gaps: vec![],
            checkpoints_verified: count,
            message: format!(
                "Verification passed: {count} checkpoints, chain intact, signature valid"
            ),
        }
    }

    fn no_manifest(url: &str) -> Self {
        ZkVerificationResult {
            url: url.to_string(),
            verified: true,
            signature_valid: true,
            chain_intact: true,
            gaps: vec![],
            checkpoints_verified: 0,
            message: "No manifest present; archive is not encrypted — skipping ZK check"
                .to_string(),
        }
    }

    fn invalid_signature(url: &str, detail: &str) -> Self {
        ZkVerificationResult {
            url: url.to_string(),
            verified: false,
            signature_valid: false,
            chain_intact: false,
            gaps: vec![],
            checkpoints_verified: 0,
            message: format!("Signature verification failed: {detail}"),
        }
    }

    fn chain_broken(url: &str, at_seq: u64, count: u64) -> Self {
        ZkVerificationResult {
            url: url.to_string(),
            verified: false,
            signature_valid: true,
            chain_intact: false,
            gaps: vec![],
            checkpoints_verified: count,
            message: format!("Hash chain broken at checkpoint sequence {at_seq}"),
        }
    }

    fn gaps_found(url: &str, gaps: Vec<(u64, u64)>, count: u64) -> Self {
        let desc = gaps
            .iter()
            .map(|(s, e)| format!("{s}–{e}"))
            .collect::<Vec<_>>()
            .join(", ");
        ZkVerificationResult {
            url: url.to_string(),
            verified: false,
            signature_valid: true,
            chain_intact: true,
            gaps,
            checkpoints_verified: count,
            message: format!("Checkpoint gaps detected in ledger ranges: {desc}"),
        }
    }
}

/// Verify an encrypted archive's integrity without decrypting any content.
///
/// Fetches `{url}/.well-known/archive-manifest.json`. If the manifest is absent
/// the archive is assumed to be unencrypted and the check is skipped (returns
/// `verified = true`). When a manifest is found, three checks run:
///
/// 1. **Signature** — the Ed25519 signature over the manifest body is verified.
///    If `expected_signer_key` is `Some`, the key embedded in the manifest must
///    also match it exactly.
/// 2. **Gap detection** — the checkpoint list must be contiguous at 64-ledger
///    intervals. Any break is recorded in `ZkVerificationResult::gaps`.
/// 3. **Hash chain** — each entry's `prev_hash` must equal the SHA-256 of the
///    previous entry's `sequence:file_hash` string, proving the chain was not
///    tampered after signing.
pub async fn verify_encrypted_archive(
    url: &str,
    expected_signer_key: Option<&str>,
    timeout: Duration,
) -> Result<ZkVerificationResult> {
    let base_url = url.trim_end_matches('/');
    let manifest_url = format!("{base_url}/.well-known/archive-manifest.json");

    let client = Client::builder()
        .timeout(timeout)
        .user_agent("stellar-k8s-operator/0.1.0")
        .build()
        .map_err(Error::HttpError)?;

    debug!("Fetching archive manifest from {manifest_url}");

    let resp = client
        .get(&manifest_url)
        .send()
        .await
        .map_err(Error::HttpError)?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        info!("No archive-manifest.json at {manifest_url}; archive is not encrypted");
        return Ok(ZkVerificationResult::no_manifest(url));
    }

    if !resp.status().is_success() {
        return Err(Error::ArchiveHealthCheckError(format!(
            "Failed to fetch manifest from {manifest_url}: HTTP {}",
            resp.status()
        )));
    }

    let manifest: ArchiveManifest = resp.json().await.map_err(|e| {
        Error::ArchiveHealthCheckError(format!("Malformed archive-manifest.json: {e}"))
    })?;

    // ── 1. Signature verification ──────────────────────────────────────────────

    if let Some(expected) = expected_signer_key {
        if manifest.signer_key != expected {
            warn!(
                "Manifest signer key mismatch at {url}: expected {expected}, got {}",
                manifest.signer_key
            );
            return Ok(ZkVerificationResult::invalid_signature(
                url,
                &format!(
                    "signer key mismatch (expected {expected}, got {})",
                    manifest.signer_key
                ),
            ));
        }
    }

    match verify_manifest_signature(&manifest) {
        Ok(()) => debug!("Manifest signature valid for {url}"),
        Err(detail) => {
            warn!("Invalid manifest signature at {url}: {detail}");
            return Ok(ZkVerificationResult::invalid_signature(url, &detail));
        }
    }

    let checkpoints = &manifest.body.checkpoints;
    let count = checkpoints.len() as u64;

    if count == 0 {
        info!("Manifest at {url} has 0 checkpoints; archive is empty");
        return Ok(ZkVerificationResult::ok(url, 0));
    }

    // ── 2. Gap detection ────────────────────────────────────────────────────────
    // Stellar checkpoints are at ledger 63, 127, 191, … (step = 64).

    let mut sorted = checkpoints.clone();
    sorted.sort_by_key(|e| e.sequence);

    let mut gaps: Vec<(u64, u64)> = vec![];
    for window in sorted.windows(2) {
        let expected_next = window[0].sequence + 64;
        if window[1].sequence != expected_next {
            gaps.push((expected_next, window[1].sequence - 1));
        }
    }

    if !gaps.is_empty() {
        warn!(
            "Archive at {url} has {} gap(s) in checkpoint chain",
            gaps.len()
        );
        return Ok(ZkVerificationResult::gaps_found(url, gaps, count));
    }

    // ── 3. Hash chain verification ──────────────────────────────────────────────
    // entry[0].prev_hash must be "0" (genesis sentinel).
    // entry[i].prev_hash must equal SHA-256("sequence:file_hash" of entry[i-1]).

    if sorted[0].prev_hash != "0" {
        return Ok(ZkVerificationResult::chain_broken(
            url,
            sorted[0].sequence,
            0,
        ));
    }

    for i in 1..sorted.len() {
        let prev = &sorted[i - 1];
        let current = &sorted[i];
        let expected_prev_hash = checkpoint_chain_hash(prev);
        if current.prev_hash != expected_prev_hash {
            warn!(
                "Hash chain broken at sequence {} in archive {url}",
                current.sequence
            );
            return Ok(ZkVerificationResult::chain_broken(
                url,
                current.sequence,
                i as u64,
            ));
        }
    }

    info!("ZK archive verification passed for {url}: {count} checkpoints verified");
    Ok(ZkVerificationResult::ok(url, count))
}

/// Run ZK verification for a list of archive URLs.
///
/// Returns one `ZkVerificationResult` per URL. A missing manifest is treated
/// as a pass so that unencrypted archives are not penalised.
pub async fn run_zk_verification(
    urls: &[String],
    expected_signer_key: Option<&str>,
    timeout: Duration,
) -> Vec<ZkVerificationResult> {
    let mut results = Vec::with_capacity(urls.len());
    for url in urls {
        match verify_encrypted_archive(url, expected_signer_key, timeout).await {
            Ok(r) => results.push(r),
            Err(e) => {
                warn!("ZK verification error for {url}: {e}");
                results.push(ZkVerificationResult {
                    url: url.clone(),
                    verified: false,
                    signature_valid: false,
                    chain_intact: false,
                    gaps: vec![],
                    checkpoints_verified: 0,
                    message: format!("Verification error: {e}"),
                });
            }
        }
    }
    results
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Compute the chain link hash for a checkpoint entry.
///
/// The hash input is `"{sequence}:{file_hash}"` — a deterministic,
/// human-readable string that does not require canonicalising JSON.
fn checkpoint_chain_hash(entry: &CheckpointEntry) -> String {
    let input = format!("{}:{}", entry.sequence, entry.file_hash);
    let digest = Sha256::digest(input.as_bytes());
    bytes_to_hex(&digest)
}

/// Verify the Ed25519 signature embedded in a manifest.
///
/// The signature covers the stable JSON encoding of `ManifestBody` (all fields
/// except `signer_key` and `signature`).
fn verify_manifest_signature(manifest: &ArchiveManifest) -> std::result::Result<(), String> {
    // Decode the public key (32 bytes)
    let key_bytes = hex_to_bytes_fixed::<32>(&manifest.signer_key)
        .map_err(|e| format!("Invalid signer_key: {e}"))?;
    let verifying_key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|e| format!("Cannot construct verifying key: {e}"))?;

    // Decode the signature (64 bytes)
    let sig_bytes = hex_to_bytes_fixed::<64>(&manifest.signature)
        .map_err(|e| format!("Invalid signature encoding: {e}"))?;
    let signature = Signature::from_bytes(&sig_bytes);

    // Canonical message: JSON of the body (deterministic field order via serde)
    let body_json = serde_json::to_string(&manifest.body)
        .map_err(|e| format!("Failed to serialise manifest body: {e}"))?;

    use ed25519_dalek::Verifier;
    verifying_key
        .verify(body_json.as_bytes(), &signature)
        .map_err(|e| format!("Signature mismatch: {e}"))
}

/// Decode a hex string into exactly `N` bytes.
fn hex_to_bytes_fixed<const N: usize>(hex: &str) -> std::result::Result<[u8; N], String> {
    if hex.len() != N * 2 {
        return Err(format!("Expected {} hex chars, got {}", N * 2, hex.len()));
    }
    let bytes: std::result::Result<Vec<u8>, _> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect();
    let bytes = bytes.map_err(|e| format!("Invalid hex character: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| format!("Could not fit {} bytes into [{N}]", hex.len() / 2))
}

/// Encode a byte slice as a lowercase hex string.
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(sequence: u64, file_hash: &str, prev_hash: &str) -> CheckpointEntry {
        CheckpointEntry {
            sequence,
            file_hash: file_hash.to_string(),
            prev_hash: prev_hash.to_string(),
        }
    }

    fn build_valid_chain(sequences: &[u64]) -> Vec<CheckpointEntry> {
        let mut entries = Vec::new();
        let mut prev_hash = "0".to_string();
        for &seq in sequences {
            let file_hash = bytes_to_hex(&Sha256::digest(seq.to_string().as_bytes()));
            let entry = make_entry(seq, &file_hash, &prev_hash);
            prev_hash = checkpoint_chain_hash(&entry);
            entries.push(entry);
        }
        entries
    }

    #[test]
    fn test_bytes_to_hex_roundtrip() {
        let original = b"hello world";
        let hex = bytes_to_hex(original);
        assert_eq!(hex.len(), original.len() * 2);
        let decoded = hex_to_bytes_fixed::<11>(&hex).unwrap();
        assert_eq!(&decoded, original);
    }

    #[test]
    fn test_hex_to_bytes_wrong_length() {
        let err = hex_to_bytes_fixed::<4>("abcdef").unwrap_err();
        assert!(err.contains("Expected 8 hex chars"));
    }

    #[test]
    fn test_gap_detection_contiguous() {
        // Sequences 63, 127, 191 are contiguous (step = 64)
        let entries = build_valid_chain(&[63, 127, 191]);
        let mut sorted = entries.clone();
        sorted.sort_by_key(|e| e.sequence);

        let mut gaps: Vec<(u64, u64)> = vec![];
        for window in sorted.windows(2) {
            let expected_next = window[0].sequence + 64;
            if window[1].sequence != expected_next {
                gaps.push((expected_next, window[1].sequence - 1));
            }
        }
        assert!(gaps.is_empty(), "No gaps expected in contiguous chain");
    }

    #[test]
    fn test_gap_detection_with_gap() {
        // Sequences 63, 127, then 319 — missing 191, 255
        let entries = build_valid_chain(&[63, 127, 319]);
        let mut sorted = entries.clone();
        sorted.sort_by_key(|e| e.sequence);

        let mut gaps: Vec<(u64, u64)> = vec![];
        for window in sorted.windows(2) {
            let expected_next = window[0].sequence + 64;
            if window[1].sequence != expected_next {
                gaps.push((expected_next, window[1].sequence - 1));
            }
        }
        assert_eq!(gaps.len(), 1, "Exactly one gap range expected");
        assert_eq!(gaps[0], (191, 318));
    }

    #[test]
    fn test_hash_chain_valid() {
        let entries = build_valid_chain(&[63, 127, 191, 255]);
        assert_eq!(entries[0].prev_hash, "0");
        for i in 1..entries.len() {
            let expected = checkpoint_chain_hash(&entries[i - 1]);
            assert_eq!(
                entries[i].prev_hash, expected,
                "Chain link broken at index {i}"
            );
        }
    }

    #[test]
    fn test_hash_chain_tampered() {
        let mut entries = build_valid_chain(&[63, 127, 191]);
        // Tamper: change file_hash of the second entry without updating chain link
        entries[1].file_hash = "deadbeef".repeat(8);

        // The third entry's prev_hash was computed from the original entry[1].
        // After tampering entry[1].file_hash, recomputing the chain link gives a
        // different value — so the chain is broken between entries[1] and entries[2].
        let recomputed = checkpoint_chain_hash(&entries[1]);
        assert_ne!(
            entries[2].prev_hash, recomputed,
            "Chain should be broken after tampering"
        );
    }

    #[test]
    fn test_zk_result_constructors() {
        let ok = ZkVerificationResult::ok("http://example.com", 5);
        assert!(ok.verified);
        assert!(ok.gaps.is_empty());

        let no_manifest = ZkVerificationResult::no_manifest("http://example.com");
        assert!(no_manifest.verified);

        let bad_sig = ZkVerificationResult::invalid_signature("http://example.com", "mismatch");
        assert!(!bad_sig.verified);
        assert!(!bad_sig.signature_valid);

        let broken = ZkVerificationResult::chain_broken("http://example.com", 127, 2);
        assert!(!broken.verified);
        assert!(!broken.chain_intact);

        let gapped = ZkVerificationResult::gaps_found("http://example.com", vec![(191, 254)], 4);
        assert!(!gapped.verified);
        assert_eq!(gapped.gaps.len(), 1);
    }
}
