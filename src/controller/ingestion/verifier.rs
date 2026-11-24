//! Merkle proof verification for ledger state ingestion.
//!
//! This module provides a controller-facing interface for validating state
//! inclusion proofs during fast-sync bootstrap.

use sha2::{Digest, Sha256};

/// A 32-byte SHA-256 hash.
pub type Hash = [u8; 32];

/// A single step in a Merkle proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProofStep {
    /// The sibling hash at this level.
    pub hash: Hash,
    /// When true, the sibling is the left child and the current node is the right child.
    /// When false, the sibling is the right child and the current node is the left child.
    pub sibling_is_left: bool,
}

/// A Merkle proof for a leaf node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleProof {
    /// The ordered list of proof steps from leaf to root.
    pub steps: Vec<ProofStep>,
}

/// Computes the SHA-256 hash of data.
pub fn sha256(data: &[u8]) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

/// Computes the hash of a leaf node from its raw serialized bytes.
pub fn hash_leaf(leaf_data: &[u8]) -> Hash {
    sha256(leaf_data)
}

/// Computes the hash of an internal node from its two child hashes.
pub fn hash_node(left: &Hash, right: &Hash) -> Hash {
    let mut data = [0u8; 64];
    data[..32].copy_from_slice(left);
    data[32..].copy_from_slice(right);
    sha256(&data)
}

/// Verifies a Merkle proof against the expected root hash.
pub fn verify_merkle_proof(proof: &MerkleProof, leaf_hash: &Hash, expected_root: &Hash) -> bool {
    if proof.steps.is_empty() {
        return leaf_hash == expected_root;
    }

    let mut current = *leaf_hash;
    for step in &proof.steps {
        current = if step.sibling_is_left {
            hash_node(&step.hash, &current)
        } else {
            hash_node(&current, &step.hash)
        };
    }

    current == *expected_root
}

/// Verifier for Stellar ledger state Merkle proofs.
#[derive(Debug, Clone, Copy, Default)]
pub struct StateVerifier;

impl StateVerifier {
    /// Verifies a Merkle proof for the given serialized leaf data.
    pub fn verify_state_proof(
        &self,
        proof: &MerkleProof,
        leaf_data: &[u8],
        expected_root: &Hash,
    ) -> bool {
        let leaf_hash = hash_leaf(leaf_data);
        verify_merkle_proof(proof, &leaf_hash, expected_root)
    }

    /// Verifies a Merkle proof using a precomputed leaf hash.
    pub fn verify_state_proof_with_hash(
        &self,
        proof: &MerkleProof,
        leaf_hash: &Hash,
        expected_root: &Hash,
    ) -> bool {
        verify_merkle_proof(proof, leaf_hash, expected_root)
    }
}
