//! Core Merkle-proof verification primitives.
//!
//! Provides iterative (non-recursive) SHA-256 and Keccak-256 Merkle root
//! computation and proof verification that operates safely within WASM stack
//! limits.
//!
//! # Merkle tree conventions
//!
//! - Leaves are sorted or unsorted depending on the tree variant.
//! - Each internal node is `H(left ++ right)` where `H` is the chosen hash.
//! - For an odd number of nodes at a level the last node is duplicated
//!   (Bitcoin / Stellar convention).
//! - Proof path elements carry a `side` flag (`Left`|`Right`) indicating
//!   which side the sibling sits on.
//!
//! # Anti-second-preimage protection
//!
//! Leaf hashes are domain-separated from internal node hashes by prepending
//! `0x00` to leaf data and `0x01` to internal node concatenations before
//! hashing.  This prevents second-preimage attacks.

use sha2::{Digest, Sha256};
use tiny_keccak::{Hasher as _, Keccak};

// ── Public constants ──────────────────────────────────────────────────────────

/// Domain prefix for leaf nodes (prevents second-preimage attacks).
pub const LEAF_PREFIX: u8 = 0x00;
/// Domain prefix for internal nodes.
pub const NODE_PREFIX: u8 = 0x01;

// ── Hash algorithm selection ──────────────────────────────────────────────────

/// Which hash function to use for Merkle operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha256,
    Keccak256,
}

// ── Internal hash dispatcher ──────────────────────────────────────────────────

/// Hash `data` with the selected algorithm, returning a 32-byte digest.
fn hash_bytes(alg: HashAlgorithm, data: &[u8]) -> [u8; 32] {
    match alg {
        HashAlgorithm::Sha256 => {
            let digest = Sha256::digest(data);
            digest.into()
        }
        HashAlgorithm::Keccak256 => {
            let mut k = Keccak::v256();
            let mut out = [0u8; 32];
            k.update(data);
            k.finalize(&mut out);
            out
        }
    }
}

/// Compute a domain-separated leaf hash.
pub fn hash_leaf(alg: HashAlgorithm, data: &[u8]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(1 + data.len());
    buf.push(LEAF_PREFIX);
    buf.extend_from_slice(data);
    hash_bytes(alg, &buf)
}

/// Combine two child hashes into a parent hash (domain-separated).
pub fn hash_node(alg: HashAlgorithm, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 65]; // 1 prefix + 32 + 32
    buf[0] = NODE_PREFIX;
    buf[1..33].copy_from_slice(left);
    buf[33..65].copy_from_slice(right);
    hash_bytes(alg, &buf)
}

// ── Proof path ────────────────────────────────────────────────────────────────

/// The side on which a sibling sits in the Merkle tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofSide {
    /// The sibling is to the **left** of the current node.
    Left,
    /// The sibling is to the **right** of the current node.
    Right,
}

/// A single element in a Merkle proof path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofElement {
    /// The sibling hash at this level.
    pub sibling: [u8; 32],
    /// Whether the sibling is on the left or right.
    pub side: ProofSide,
}

// ── Single-leaf proof verification ───────────────────────────────────────────

/// Verify a single-element Merkle proof.
///
/// Starting from `leaf_data`, the function iteratively combines hashes
/// up the proof path.  Returns `true` iff the computed root matches
/// `expected_root`.
///
/// This implementation is iterative (no recursion) and therefore safe within
/// the WASM instruction-budget constraints.
///
/// # Arguments
///
/// * `alg`           – hash algorithm to use
/// * `leaf_data`     – raw bytes of the leaf being proved
/// * `proof`         – ordered list of sibling hashes from leaf to root
/// * `expected_root` – the on-chain Merkle root to verify against
pub fn verify_proof(
    alg: HashAlgorithm,
    leaf_data: &[u8],
    proof: &[ProofElement],
    expected_root: &[u8; 32],
) -> bool {
    let mut current = hash_leaf(alg, leaf_data);

    for elem in proof {
        current = match elem.side {
            ProofSide::Left  => hash_node(alg, &elem.sibling, &current),
            ProofSide::Right => hash_node(alg, &current, &elem.sibling),
        };
    }

    &current == expected_root
}

// ── Multi-proof verification ──────────────────────────────────────────────────

/// A single leaf/proof pair for use in multi-proof verification.
#[derive(Debug, Clone)]
pub struct LeafProof {
    pub leaf_data: Vec<u8>,
    pub proof: Vec<ProofElement>,
}

/// Verify multiple leaf proofs against the **same** root in a single call.
///
/// Each leaf is verified independently using [`verify_proof`].  Returns `true`
/// only if **all** proofs are valid.
pub fn verify_multiproof(
    alg: HashAlgorithm,
    leaves: &[LeafProof],
    expected_root: &[u8; 32],
) -> bool {
    if leaves.is_empty() {
        return false;
    }
    leaves
        .iter()
        .all(|lp| verify_proof(alg, &lp.leaf_data, &lp.proof, expected_root))
}

// ── Root computation ──────────────────────────────────────────────────────────

/// Compute the Merkle root for a slice of leaf-data items.
///
/// Uses the iterative bottom-up approach — builds the tree level by level
/// without recursion.  Empty input returns the all-zero hash.
///
/// Odd-length levels duplicate the last element (Bitcoin convention).
pub fn compute_root(alg: HashAlgorithm, leaves: &[Vec<u8>]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }

    // Hash all leaves
    let mut level: Vec<[u8; 32]> = leaves.iter().map(|d| hash_leaf(alg, d)).collect();

    // Walk up the tree iteratively
    while level.len() > 1 {
        let mut next = Vec::with_capacity((level.len() + 1) / 2);
        let mut i = 0;
        while i < level.len() {
            let left = level[i];
            // Duplicate last node if odd count
            let right = if i + 1 < level.len() {
                level[i + 1]
            } else {
                level[i]
            };
            next.push(hash_node(alg, &left, &right));
            i += 2;
        }
        level = next;
    }

    level[0]
}

/// Compute the Merkle root from pre-hashed leaf hashes.
///
/// Useful when leaf hashes are stored on-chain and leaves are provided
/// separately for bandwidth efficiency.
pub fn compute_root_from_hashes(alg: HashAlgorithm, leaf_hashes: &[[u8; 32]]) -> [u8; 32] {
    if leaf_hashes.is_empty() {
        return [0u8; 32];
    }

    let mut level: Vec<[u8; 32]> = leaf_hashes.to_vec();

    while level.len() > 1 {
        let mut next = Vec::with_capacity((level.len() + 1) / 2);
        let mut i = 0;
        while i < level.len() {
            let left = level[i];
            let right = if i + 1 < level.len() { level[i + 1] } else { level[i] };
            next.push(hash_node(alg, &left, &right));
            i += 2;
        }
        level = next;
    }

    level[0]
}

// ── Proof generation helper (for testing) ────────────────────────────────────

/// Generate a single-leaf Merkle proof for `leaf_index` in `leaves`.
///
/// This is a test-only helper; production provers run off-chain.
pub fn generate_proof(
    alg: HashAlgorithm,
    leaves: &[Vec<u8>],
    leaf_index: usize,
) -> Option<Vec<ProofElement>> {
    if leaf_index >= leaves.len() {
        return None;
    }

    let mut level: Vec<[u8; 32]> = leaves.iter().map(|d| hash_leaf(alg, d)).collect();
    let mut index = leaf_index;
    let mut proof = Vec::new();

    while level.len() > 1 {
        let sibling_index = if index % 2 == 0 {
            (index + 1).min(level.len() - 1)
        } else {
            index - 1
        };

        let side = if sibling_index < index {
            ProofSide::Left
        } else {
            ProofSide::Right
        };

        proof.push(ProofElement {
            sibling: level[sibling_index],
            side,
        });

        // Build next level
        let mut next = Vec::with_capacity((level.len() + 1) / 2);
        let mut i = 0;
        while i < level.len() {
            let left = level[i];
            let right = if i + 1 < level.len() { level[i + 1] } else { level[i] };
            next.push(hash_node(alg, &left, &right));
            i += 2;
        }

        index /= 2;
        level = next;
    }

    Some(proof)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: usize) -> Vec<Vec<u8>> {
        (0..n).map(|i| format!("leaf-{i}").into_bytes()).collect()
    }

    // ── hash_leaf / hash_node domain separation ───────────────────────────

    #[test]
    fn test_leaf_and_node_hashes_differ_for_same_input() {
        let data = b"test";
        let lh = hash_leaf(HashAlgorithm::Sha256, data);
        // Construct a node hash using lh as both children
        let nh = hash_node(HashAlgorithm::Sha256, &lh, &lh);
        assert_ne!(lh, nh, "leaf hash must differ from node hash");
    }

    #[test]
    fn test_sha256_and_keccak_differ() {
        let data = b"same input";
        let s = hash_leaf(HashAlgorithm::Sha256, data);
        let k = hash_leaf(HashAlgorithm::Keccak256, data);
        assert_ne!(s, k);
    }

    // ── compute_root ──────────────────────────────────────────────────────

    #[test]
    fn test_compute_root_single_leaf() {
        let ls = leaves(1);
        let root = compute_root(HashAlgorithm::Sha256, &ls);
        // Single leaf root = hash_leaf of that leaf
        assert_eq!(root, hash_leaf(HashAlgorithm::Sha256, &ls[0]));
    }

    #[test]
    fn test_compute_root_two_leaves() {
        let ls = leaves(2);
        let root = compute_root(HashAlgorithm::Sha256, &ls);
        let expected = hash_node(
            HashAlgorithm::Sha256,
            &hash_leaf(HashAlgorithm::Sha256, &ls[0]),
            &hash_leaf(HashAlgorithm::Sha256, &ls[1]),
        );
        assert_eq!(root, expected);
    }

    #[test]
    fn test_compute_root_empty_is_zero_hash() {
        let root = compute_root(HashAlgorithm::Sha256, &[]);
        assert_eq!(root, [0u8; 32]);
    }

    #[test]
    fn test_compute_root_four_leaves_deterministic() {
        let ls = leaves(4);
        let r1 = compute_root(HashAlgorithm::Sha256, &ls);
        let r2 = compute_root(HashAlgorithm::Sha256, &ls);
        assert_eq!(r1, r2);
    }

    #[test]
    fn test_compute_root_odd_leaves_duplicates_last() {
        // 3-leaf tree: level 0 = [H0,H1,H2]
        // level 1 = [H(H0,H1), H(H2,H2)]
        // level 2 = [root]
        let ls = leaves(3);
        let h0 = hash_leaf(HashAlgorithm::Sha256, &ls[0]);
        let h1 = hash_leaf(HashAlgorithm::Sha256, &ls[1]);
        let h2 = hash_leaf(HashAlgorithm::Sha256, &ls[2]);
        let n01 = hash_node(HashAlgorithm::Sha256, &h0, &h1);
        let n22 = hash_node(HashAlgorithm::Sha256, &h2, &h2);
        let expected = hash_node(HashAlgorithm::Sha256, &n01, &n22);
        let root = compute_root(HashAlgorithm::Sha256, &ls);
        assert_eq!(root, expected);
    }

    // ── generate_proof + verify_proof ─────────────────────────────────────

    fn round_trip(alg: HashAlgorithm, n: usize, idx: usize) {
        let ls = leaves(n);
        let root = compute_root(alg, &ls);
        let proof = generate_proof(alg, &ls, idx).expect("proof generation failed");
        assert!(
            verify_proof(alg, &ls[idx], &proof, &root),
            "proof failed for n={n} idx={idx}"
        );
    }

    #[test]
    fn test_proof_sha256_various_sizes() {
        for n in [1, 2, 3, 4, 5, 7, 8, 15, 16, 31, 32] {
            for idx in [0, n / 2, n - 1] {
                round_trip(HashAlgorithm::Sha256, n, idx);
            }
        }
    }

    #[test]
    fn test_proof_keccak256_various_sizes() {
        for n in [2, 4, 8, 16, 32] {
            for idx in [0, n / 2, n - 1] {
                round_trip(HashAlgorithm::Keccak256, n, idx);
            }
        }
    }

    // ── Tampered leaf fails verification ──────────────────────────────────

    #[test]
    fn test_tampered_leaf_fails_verification() {
        let ls = leaves(8);
        let root = compute_root(HashAlgorithm::Sha256, &ls);
        let proof = generate_proof(HashAlgorithm::Sha256, &ls, 3).unwrap();
        let tampered = b"evil-data";
        assert!(!verify_proof(HashAlgorithm::Sha256, tampered, &proof, &root));
    }

    // ── Reordered proof fails verification ───────────────────────────────

    #[test]
    fn test_reordered_proof_fails() {
        let ls = leaves(8);
        let root = compute_root(HashAlgorithm::Sha256, &ls);
        let mut proof = generate_proof(HashAlgorithm::Sha256, &ls, 2).unwrap();
        if proof.len() >= 2 {
            proof.swap(0, 1); // reorder siblings
        }
        assert!(!verify_proof(HashAlgorithm::Sha256, &ls[2], &proof, &root));
    }

    // ── Wrong root fails ──────────────────────────────────────────────────

    #[test]
    fn test_wrong_root_fails() {
        let ls = leaves(4);
        let proof = generate_proof(HashAlgorithm::Sha256, &ls, 0).unwrap();
        let wrong_root = [0xffu8; 32];
        assert!(!verify_proof(HashAlgorithm::Sha256, &ls[0], &proof, &wrong_root));
    }

    // ── verify_multiproof ─────────────────────────────────────────────────

    #[test]
    fn test_verify_multiproof_all_valid() {
        let alg = HashAlgorithm::Sha256;
        let ls = leaves(8);
        let root = compute_root(alg, &ls);
        let pairs: Vec<LeafProof> = vec![0, 3, 7]
            .into_iter()
            .map(|i| LeafProof {
                leaf_data: ls[i].clone(),
                proof: generate_proof(alg, &ls, i).unwrap(),
            })
            .collect();
        assert!(verify_multiproof(alg, &pairs, &root));
    }

    #[test]
    fn test_verify_multiproof_one_invalid_fails() {
        let alg = HashAlgorithm::Sha256;
        let ls = leaves(8);
        let root = compute_root(alg, &ls);
        let mut pairs: Vec<LeafProof> = vec![0, 3, 7]
            .into_iter()
            .map(|i| LeafProof {
                leaf_data: ls[i].clone(),
                proof: generate_proof(alg, &ls, i).unwrap(),
            })
            .collect();
        // Corrupt the middle leaf
        pairs[1].leaf_data = b"tampered".to_vec();
        assert!(!verify_multiproof(alg, &pairs, &root));
    }

    #[test]
    fn test_verify_multiproof_empty_returns_false() {
        let root = [0u8; 32];
        assert!(!verify_multiproof(HashAlgorithm::Sha256, &[], &root));
    }

    // ── Depth 4 to 32 correctness ─────────────────────────────────────────

    #[test]
    fn test_proof_correctness_across_tree_depths() {
        for depth in 2u32..=5u32 {
            // Tree with 2^depth leaves
            let n = 1usize << depth;
            let ls = leaves(n);
            let root = compute_root(HashAlgorithm::Sha256, &ls);
            for idx in [0, n / 4, n / 2, n - 1] {
                let proof = generate_proof(HashAlgorithm::Sha256, &ls, idx).unwrap();
                assert!(
                    verify_proof(HashAlgorithm::Sha256, &ls[idx], &proof, &root),
                    "depth={depth} idx={idx}"
                );
            }
        }
    }

    // ── compute_root_from_hashes ──────────────────────────────────────────

    #[test]
    fn test_compute_root_from_hashes_matches_compute_root() {
        let ls = leaves(8);
        let hashes: Vec<[u8; 32]> = ls
            .iter()
            .map(|d| hash_leaf(HashAlgorithm::Sha256, d))
            .collect();
        let root_data = compute_root(HashAlgorithm::Sha256, &ls);
        let root_hashes = compute_root_from_hashes(HashAlgorithm::Sha256, &hashes);
        assert_eq!(root_data, root_hashes);
    }
}
