//! ZK Merkle proof verifier for Stellar ledger state.
///
/// This crate provides a deterministic, Wasm-compatible Merkle proof
/// verification routine using SHA-256 as the hash function, matching
/// Stellar Core's hashing standard.

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
///
/// Stellar Core hashes leaf (bucket) entries by taking the SHA-256 of the
/// serialized entry. This function mirrors that behavior.
pub fn hash_leaf(leaf_data: &[u8]) -> Hash {
    sha256(leaf_data)
}

/// Computes the hash of an internal node from its two child hashes.
///
/// The canonical ordering is left then right followed by a SHA-256 digest.
pub fn hash_node(left: &Hash, right: &Hash) -> Hash {
    let mut data = [0u8; 64];
    data[..32].copy_from_slice(left);
    data[32.].copy_from_slice(right);
    sha256(&data)
}

/// Verifies a Merkle proof against the expected root hash.
///
/// Returns true if the proof is valid and the leaf hashes to the root.
pub fn verify_merkle_proof(proof: &MerkleProof, leaf_hash: &Hash, expected_root: &Hash) -> bool {
    // An empty proof is valid only if the leaf itself is the root.
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

/// Convenience wrapper that hashes leaf_data before verification.
pub fn verify_leaf(proof: &MerkleProof, leaf_data: &[u8], expected_root: &Hash) -> bool {
    let leaf_hash = hash_leaf(leaf_data);
    verify_merkle_proof(proof, &leaf_hash, expected_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_proof_matches_root() {
        let data = b"leaf";
        let root = hash_leaf(data);
        assert!(verify_leaf(&MerkleProof { steps: vec![] }, data, &root));
    }

    #[test]
    fn simple_two_leaf_proof() {
        let a = hash_leaf(b"left");
        let b = hash_leaf(b"right");
        let root = hash_node(&a, &b);

        // Proof for a: sibling b is on the right => sibling_is_left = false
        let proof = MerkleProof {
            steps: vec![ProofStep { hash: b, sibling_is_left: false }],
        };
        assert!(verify_merkle_proof(&proof, &a, &root));

        // Proof for b: sibling a is on the left => sibling_is_left = true
        let proof = MerkleProof {
            steps: vec![ProofStep { hash: a, sibling_is_left: true }],
        };
        assert!(verify_merkle_proof(&proof, &b, &root));
    }
}