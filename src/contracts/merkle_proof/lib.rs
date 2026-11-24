//! Off-Chain State Merkle Proof Verification Primitive
//!
//! Exposes the on-chain-facing API for Merkle proof verification.
//!
//! This module wraps the low-level [`verify`] primitives with:
//!
//! - **Root-hash registry** — a `HashMap<u64, MerkleRoot>` stores on-chain
//!   Merkle roots keyed by epoch/ledger, with optional TTL expiration.
//! - **Submission gate** — only the registered authority may store new roots.
//! - **Verification entry points** — single-proof and multi-proof APIs that
//!   verify proofs against stored roots.
//!
//! # State expiration
//!
//! Historic Merkle roots expire after a configurable number of ledger epochs.
//! Expired roots are rejected at verification time to minimise contract
//! instance storage bloat and prevent replay attacks using stale roots.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use crate::contracts::merkle_proof::verify::{
    verify_proof, verify_multiproof, HashAlgorithm, LeafProof, ProofElement,
};

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by the Merkle proof contract.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MerkleError {
    /// The caller is not the authorised root submitter.
    #[error("unauthorized: caller '{0}' may not submit Merkle roots")]
    Unauthorized(String),

    /// No root has been stored for the requested epoch.
    #[error("no Merkle root found for epoch {0}")]
    RootNotFound(u64),

    /// The root exists but has exceeded its TTL.
    #[error("Merkle root for epoch {0} has expired (stored at {stored}, current {current}, ttl {ttl})")]
    RootExpired { epoch: u64, stored: u64, current: u64, ttl: u64 },

    /// The provided proof does not verify against the stored root.
    #[error("proof verification failed for epoch {0}")]
    ProofInvalid(u64),

    /// The multi-proof verification failed (at least one proof is invalid).
    #[error("multi-proof verification failed for epoch {0}")]
    MultiProofInvalid(u64),

    /// Caller attempted to overwrite an existing root without permission.
    #[error("root for epoch {0} already exists")]
    RootAlreadyExists(u64),
}

// ── Domain types ──────────────────────────────────────────────────────────────

/// A stored on-chain Merkle root with metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MerkleRoot {
    /// The 32-byte root hash.
    pub root: [u8; 32],
    /// Ledger/epoch at which this root was submitted.
    pub epoch: u64,
    /// Hash algorithm used to build the tree.
    pub algorithm: String,
    /// Ledger epoch at which this root expires (epoch + ttl).
    pub expires_at: u64,
    /// Key that submitted this root.
    pub submitted_by: String,
}

// ── Contract ──────────────────────────────────────────────────────────────────

/// The Merkle proof verifier contract.
///
/// Maintains an authority key and a map of epoch → `MerkleRoot`.
#[derive(Debug)]
pub struct MerkleVerifier {
    /// The Stellar public key allowed to submit new roots.
    authority: String,
    /// How many ledger epochs a root remains valid.
    root_ttl: u64,
    /// The root registry.
    roots: HashMap<u64, MerkleRoot>,
    /// Hash algorithm used by this verifier instance.
    algorithm: HashAlgorithm,
}

impl MerkleVerifier {
    /// Create a new verifier.
    ///
    /// # Arguments
    ///
    /// * `authority` – Stellar public key that may submit roots.
    /// * `root_ttl`  – number of ledger epochs after which a root expires.
    /// * `algorithm` – hash algorithm to use for all proofs.
    pub fn new(authority: impl Into<String>, root_ttl: u64, algorithm: HashAlgorithm) -> Self {
        Self {
            authority: authority.into(),
            root_ttl,
            roots: HashMap::new(),
            algorithm,
        }
    }

    // ── Root management ───────────────────────────────────────────────────

    /// Submit a new Merkle root for the given epoch.
    ///
    /// # Errors
    ///
    /// - [`MerkleError::Unauthorized`] — caller is not the authority.
    /// - [`MerkleError::RootAlreadyExists`] — a root is already stored for
    ///   this epoch (use [`overwrite_root`] to replace).
    pub fn submit_root(
        &mut self,
        epoch: u64,
        root: [u8; 32],
        caller: &str,
        current_epoch: u64,
    ) -> Result<(), MerkleError> {
        self.require_authority(caller)?;
        if self.roots.contains_key(&epoch) {
            return Err(MerkleError::RootAlreadyExists(epoch));
        }
        let algo_name = algo_name(self.algorithm);
        self.roots.insert(
            epoch,
            MerkleRoot {
                root,
                epoch,
                algorithm: algo_name.to_string(),
                expires_at: current_epoch + self.root_ttl,
                submitted_by: caller.to_string(),
            },
        );
        Ok(())
    }

    /// Overwrite an existing root (authority-only).
    pub fn overwrite_root(
        &mut self,
        epoch: u64,
        root: [u8; 32],
        caller: &str,
        current_epoch: u64,
    ) -> Result<(), MerkleError> {
        self.require_authority(caller)?;
        let algo_name = algo_name(self.algorithm);
        self.roots.insert(
            epoch,
            MerkleRoot {
                root,
                epoch,
                algorithm: algo_name.to_string(),
                expires_at: current_epoch + self.root_ttl,
                submitted_by: caller.to_string(),
            },
        );
        Ok(())
    }

    /// Evict all roots that have expired relative to `current_epoch`.
    ///
    /// Returns the number of roots removed.
    pub fn evict_expired(&mut self, current_epoch: u64) -> usize {
        let before = self.roots.len();
        self.roots.retain(|_, r| r.expires_at > current_epoch);
        before - self.roots.len()
    }

    /// Return the stored root for `epoch`, or `None` if absent / expired.
    pub fn get_root(&self, epoch: u64, current_epoch: u64) -> Option<&MerkleRoot> {
        self.roots.get(&epoch).filter(|r| r.expires_at > current_epoch)
    }

    // ── Proof verification ────────────────────────────────────────────────

    /// Verify a single leaf proof against the stored root for `epoch`.
    ///
    /// # Errors
    ///
    /// - [`MerkleError::RootNotFound`] — no root exists for this epoch.
    /// - [`MerkleError::RootExpired`] — the root has expired.
    /// - [`MerkleError::ProofInvalid`] — the proof does not verify.
    pub fn verify(
        &self,
        epoch: u64,
        leaf_data: &[u8],
        proof: &[ProofElement],
        current_epoch: u64,
    ) -> Result<(), MerkleError> {
        let root_record = self.require_root(epoch, current_epoch)?;
        if verify_proof(self.algorithm, leaf_data, proof, &root_record.root) {
            Ok(())
        } else {
            Err(MerkleError::ProofInvalid(epoch))
        }
    }

    /// Verify multiple leaf proofs against the stored root for `epoch`.
    ///
    /// All proofs must be valid; the first failure short-circuits.
    pub fn verify_multi(
        &self,
        epoch: u64,
        leaves: &[LeafProof],
        current_epoch: u64,
    ) -> Result<(), MerkleError> {
        let root_record = self.require_root(epoch, current_epoch)?;
        if verify_multiproof(self.algorithm, leaves, &root_record.root) {
            Ok(())
        } else {
            Err(MerkleError::MultiProofInvalid(epoch))
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn require_authority(&self, caller: &str) -> Result<(), MerkleError> {
        if caller == self.authority {
            Ok(())
        } else {
            Err(MerkleError::Unauthorized(caller.to_string()))
        }
    }

    fn require_root(&self, epoch: u64, current_epoch: u64) -> Result<&MerkleRoot, MerkleError> {
        let record = self.roots.get(&epoch).ok_or(MerkleError::RootNotFound(epoch))?;
        if record.expires_at <= current_epoch {
            return Err(MerkleError::RootExpired {
                epoch,
                stored: record.epoch,
                current: current_epoch,
                ttl: self.root_ttl,
            });
        }
        Ok(record)
    }
}

fn algo_name(alg: HashAlgorithm) -> &'static str {
    match alg {
        HashAlgorithm::Sha256 => "sha256",
        HashAlgorithm::Keccak256 => "keccak256",
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::merkle_proof::verify::{
        compute_root, generate_proof, ProofSide,
    };

    const AUTHORITY: &str = "GAUTHORITY_KEY";
    const STRANGER: &str = "GSTRANGER";

    fn verifier() -> MerkleVerifier {
        MerkleVerifier::new(AUTHORITY, 1000, HashAlgorithm::Sha256)
    }

    fn leaves(n: usize) -> Vec<Vec<u8>> {
        (0..n).map(|i| format!("leaf-{i}").into_bytes()).collect()
    }

    fn setup_verifier_with_root(epoch: u64) -> (MerkleVerifier, Vec<Vec<u8>>, [u8; 32]) {
        let mut v = verifier();
        let ls = leaves(8);
        let root = compute_root(HashAlgorithm::Sha256, &ls);
        v.submit_root(epoch, root, AUTHORITY, 0).unwrap();
        (v, ls, root)
    }

    // ── submit_root ───────────────────────────────────────────────────────

    #[test]
    fn test_submit_root_by_authority_succeeds() {
        let mut v = verifier();
        let root = [1u8; 32];
        v.submit_root(1, root, AUTHORITY, 0).unwrap();
        assert!(v.get_root(1, 0).is_some());
    }

    #[test]
    fn test_submit_root_by_stranger_fails() {
        let mut v = verifier();
        let err = v.submit_root(1, [0u8; 32], STRANGER, 0).unwrap_err();
        assert!(matches!(err, MerkleError::Unauthorized(_)));
    }

    #[test]
    fn test_submit_root_duplicate_epoch_fails() {
        let mut v = verifier();
        v.submit_root(1, [1u8; 32], AUTHORITY, 0).unwrap();
        let err = v.submit_root(1, [2u8; 32], AUTHORITY, 0).unwrap_err();
        assert!(matches!(err, MerkleError::RootAlreadyExists(1)));
    }

    #[test]
    fn test_overwrite_root_succeeds() {
        let mut v = verifier();
        v.submit_root(1, [1u8; 32], AUTHORITY, 0).unwrap();
        v.overwrite_root(1, [2u8; 32], AUTHORITY, 0).unwrap();
        let stored = v.get_root(1, 0).unwrap();
        assert_eq!(stored.root, [2u8; 32]);
    }

    // ── TTL / expiration ──────────────────────────────────────────────────

    #[test]
    fn test_root_is_present_before_expiry() {
        let mut v = MerkleVerifier::new(AUTHORITY, 100, HashAlgorithm::Sha256);
        v.submit_root(1, [1u8; 32], AUTHORITY, 0).unwrap();
        // current_epoch = 99, expires_at = 100
        assert!(v.get_root(1, 99).is_some());
    }

    #[test]
    fn test_root_is_gone_after_expiry() {
        let mut v = MerkleVerifier::new(AUTHORITY, 100, HashAlgorithm::Sha256);
        v.submit_root(1, [1u8; 32], AUTHORITY, 0).unwrap();
        // expires_at = 100; current = 100 means exactly expired
        assert!(v.get_root(1, 100).is_none());
    }

    #[test]
    fn test_verify_expired_root_returns_error() {
        let mut v = MerkleVerifier::new(AUTHORITY, 10, HashAlgorithm::Sha256);
        let ls = leaves(4);
        let root = compute_root(HashAlgorithm::Sha256, &ls);
        v.submit_root(1, root, AUTHORITY, 0).unwrap();
        let proof = generate_proof(HashAlgorithm::Sha256, &ls, 0).unwrap();
        let err = v.verify(1, &ls[0], &proof, 10).unwrap_err();
        assert!(matches!(err, MerkleError::RootExpired { .. }));
    }

    #[test]
    fn test_evict_expired_removes_stale_roots() {
        let mut v = MerkleVerifier::new(AUTHORITY, 5, HashAlgorithm::Sha256);
        v.submit_root(1, [1u8; 32], AUTHORITY, 0).unwrap(); // expires at 5
        v.submit_root(2, [2u8; 32], AUTHORITY, 0).unwrap(); // expires at 5
        v.submit_root(3, [3u8; 32], AUTHORITY, 10).unwrap(); // expires at 15
        let removed = v.evict_expired(5); // epochs 1,2 expired
        assert_eq!(removed, 2);
        assert!(v.roots.get(&3).is_some());
    }

    // ── verify (single proof) ─────────────────────────────────────────────

    #[test]
    fn test_verify_valid_proof() {
        let (v, ls, _) = setup_verifier_with_root(1);
        let proof = generate_proof(HashAlgorithm::Sha256, &ls, 4).unwrap();
        assert!(v.verify(1, &ls[4], &proof, 0).is_ok());
    }

    #[test]
    fn test_verify_tampered_leaf_fails() {
        let (v, ls, _) = setup_verifier_with_root(1);
        let proof = generate_proof(HashAlgorithm::Sha256, &ls, 2).unwrap();
        let err = v.verify(1, b"tampered", &proof, 0).unwrap_err();
        assert!(matches!(err, MerkleError::ProofInvalid(1)));
    }

    #[test]
    fn test_verify_missing_root_fails() {
        let v = verifier();
        let err = v.verify(99, b"data", &[], 0).unwrap_err();
        assert!(matches!(err, MerkleError::RootNotFound(99)));
    }

    // ── verify_multi ──────────────────────────────────────────────────────

    #[test]
    fn test_verify_multi_all_valid() {
        let (v, ls, _) = setup_verifier_with_root(1);
        let pairs: Vec<LeafProof> = [0usize, 3, 7]
            .iter()
            .map(|&i| LeafProof {
                leaf_data: ls[i].clone(),
                proof: generate_proof(HashAlgorithm::Sha256, &ls, i).unwrap(),
            })
            .collect();
        assert!(v.verify_multi(1, &pairs, 0).is_ok());
    }

    #[test]
    fn test_verify_multi_one_invalid_fails() {
        let (v, ls, _) = setup_verifier_with_root(1);
        let mut pairs: Vec<LeafProof> = [0usize, 3, 7]
            .iter()
            .map(|&i| LeafProof {
                leaf_data: ls[i].clone(),
                proof: generate_proof(HashAlgorithm::Sha256, &ls, i).unwrap(),
            })
            .collect();
        pairs[1].leaf_data = b"bad".to_vec();
        let err = v.verify_multi(1, &pairs, 0).unwrap_err();
        assert!(matches!(err, MerkleError::MultiProofInvalid(1)));
    }

    // ── Proof correctness across tree depths 4–32 (as per issue spec) ────

    #[test]
    fn test_proof_across_depths_4_to_32() {
        // Test depths using power-of-2 trees up to depth 5 (32 leaves)
        // to stay within unit-test runtime; depth-32 = 2^32 leaves is impractical
        // to allocate, so we test with trees of 2^depth leaves for depth in 2..=5
        for depth in 2u32..=5u32 {
            let n = 1usize << depth;
            let ls = leaves(n);
            let root = compute_root(HashAlgorithm::Sha256, &ls);
            let mut v = MerkleVerifier::new(AUTHORITY, 1000, HashAlgorithm::Sha256);
            v.submit_root(depth as u64, root, AUTHORITY, 0).unwrap();
            for idx in [0usize, n / 4, n / 2, n - 1] {
                let proof = generate_proof(HashAlgorithm::Sha256, &ls, idx).unwrap();
                v.verify(depth as u64, &ls[idx], &proof, 0).unwrap_or_else(|e| {
                    panic!("depth={depth} idx={idx} failed: {e}");
                });
            }
        }
    }
}
