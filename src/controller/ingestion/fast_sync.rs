//! Fast-sync bootstrap logic for non-validator RPC nodes.
//!
//! The verifier allows a fresh node to cryptographically validate a small
//! state proof from a peer or archive instead of downloading the full
//! historical ledger. This module coordinates that process.

use crate::controller::ingestion::verifier::{Hash, MerkleProof, StateVerifier};

/// Errors that can occur during fast-sync.
#[derive(Debug, thiserror::Error)]
pub enum FastSyncError {
    #[error("Merkle proof verification failed for state root {0:?}")]
    InvalidProof(Hash),
    #[error("state entry already exists for key")]
    AlreadyExists,
    #[error("failed to persist verified state entry: {0}")]
    Persistence(String),
}

/// Result of a fast-sync bootstrap attempt.
pub type FastSyncResult<T> = Result<T, FastSyncError>;

/// Coordinates fast-sync bootstrapping using Merkle proof verification.
#[derive(Debug, Clone)]
pub struct FastSync {
    verifier: StateVerifier,
}

impl FastSync {
    /// Creates a new fast-sync coordinator.
    pub fn new() -> Self {
        Self {
            verifier: StateVerifier,
        }
    }

    /// Attempts to verify and ingest a single state entry.
    ///
    /// * state_root - the trusted ledger state root from a peer/checkpoint
    /// * leaf_data - serialized state entry to verify
    /// * proof - Merkle proof of inclusion for the entry
    pub fn bootstrap(
        &self,
        state_root: &Hash,
        leaf_data: &[u8],
        proof: &MerkleProof,
    ) -> FastSyncResult<()> {
        if !self
            .verifier
            .verify_state_proof(proof, leaf_data, state_root)
        {
            return Err(FastSyncError::InvalidProof(*state_root));
        }

        // In a full implementation this would write the verified entry into the
        // local key-value store. The current controller contract stops at proof
        // verification so callers can decide where to persist accepted state.
        Ok(())
    }

    /// Verifies a proof against a precomputed leaf hash.
    pub fn verify_with_hash(
        &self,
        state_root: &Hash,
        leaf_hash: &Hash,
        proof: &MerkleProof,
    ) -> FastSyncResult<()> {
        if !self
            .verifier
            .verify_state_proof_with_hash(proof, leaf_hash, state_root)
        {
            return Err(FastSyncError::InvalidProof(*state_root));
        }
        Ok(())
    }
}

impl Default for FastSync {
    fn default() -> Self {
        Self::new()
    }
}
