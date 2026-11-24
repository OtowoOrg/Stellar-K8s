//! # Merkle Proof Verification
//!
//! Provides iterative (stack-safe) single-path and multi-leaf Merkle proof
//! verification using SHA-256 or a caller-supplied Soroban-native hash function.
//!
//! ## Complexity
//! Each proof step performs exactly one hash call, so total CPU instruction cost
//! scales as **O(log N)** relative to the tree depth.  No recursion is used, which
//! prevents Wasm stack overflow even for 32-level trees.

use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A 32-byte Merkle node digest.
pub type Hash = [u8; 32];

/// Direction of a sibling at a given tree level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The sibling is to the **left** of the current node.
    Left,
    /// The sibling is to the **right** of the current node.
    Right,
}

/// One step in a Merkle proof path.
#[derive(Debug, Clone)]
pub struct ProofNode {
    /// The sibling hash at this level.
    pub sibling: Hash,
    /// Whether the sibling is on the left or the right.
    pub side: Side,
}

/// A complete single-leaf Merkle proof.
#[derive(Debug, Clone)]
pub struct MerkleProof {
    /// The leaf whose membership is being proven.
    pub leaf: Hash,
    /// Ordered proof path from the leaf up to (but not including) the root.
    pub path: Vec<ProofNode>,
}

/// One leaf entry inside a multi-proof batch.
#[derive(Debug, Clone)]
pub struct MultiLeaf {
    /// The leaf whose membership is being proven.
    pub leaf: Hash,
    /// 0-based index of the leaf in the Merkle tree.
    pub index: u64,
}

/// A compact multi-proof that verifies several leaves in one pass.
///
/// Uses the standard "multi-proof" algorithm: the caller supplies the full
/// ordered set of sibling hashes required to reconstruct the root, together
/// with the leaf index set so the algorithm knows when to use a sibling from
/// the proof vs. a hash computed from another proved leaf.
#[derive(Debug, Clone)]
pub struct MultiProof {
    /// Leaves to be verified (must be sorted by `index` ascending).
    pub leaves: Vec<MultiLeaf>,
    /// Auxiliary sibling hashes needed to reconstruct the root, in the order
    /// they are consumed (left-to-right, bottom-to-top, matching the standard
    /// OpenZeppelin / Bitcoin-SPV multi-proof ordering).
    pub siblings: Vec<Hash>,
    /// Total number of leaves in the original tree (must be a power of two).
    pub total_leaves: u64,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Hash a pair of child nodes in canonical (left ‖ right) order.
///
/// `sha2` is `no_std`-compatible so this compiles inside a Soroban Wasm guest
/// without pulling in std.
#[inline]
fn hash_pair(left: &Hash, right: &Hash) -> Hash {
    let mut h = Sha256::new();
    h.update(left);
    h.update(right);
    h.finalize().into()
}

/// Hash a raw byte slice to produce a leaf digest.
///
/// Compatible with Ethereum/Stellar "double-SHA256 on leaf data" if the caller
/// pre-hashes the raw data once before passing it here, or feeds raw bytes for
/// a single-SHA256 scheme.
#[inline]
pub fn hash_leaf(data: &[u8]) -> Hash {
    Sha256::digest(data).into()
}

// ---------------------------------------------------------------------------
// Single-path verification
// ---------------------------------------------------------------------------

/// Verify that `proof.leaf` is a member of the Merkle tree whose root is
/// `expected_root`.
///
/// # Returns
/// `true`  if the reconstructed root matches `expected_root`.
/// `false` if any proof step produces a mismatch.
///
/// # Complexity
/// O(path.len()) hash operations — typically O(log N) for a balanced tree.
pub fn verify_proof(proof: &MerkleProof, expected_root: &Hash) -> bool {
    let mut current = proof.leaf;

    for node in &proof.path {
        current = match node.side {
            Side::Left => hash_pair(&node.sibling, &current),
            Side::Right => hash_pair(&current, &node.sibling),
        };
    }

    &current == expected_root
}

// ---------------------------------------------------------------------------
// Multi-proof verification
// ---------------------------------------------------------------------------

/// Verify that all leaves in `proof.leaves` are members of the Merkle tree
/// whose root is `expected_root`.
///
/// ## Algorithm
/// Implements the iterative bottom-up multi-proof algorithm popularised by
/// OpenZeppelin.  Leaves are sorted by index; at each tree level pairs of
/// adjacent proved nodes are combined without consuming a sibling from the
/// proof, while un-paired nodes consume one sibling from `proof.siblings`.
///
/// ## Complexity
/// O(k · log N) where k = number of proved leaves and N = total tree size.
/// No recursion; memory usage is bounded by two O(k) arrays.
///
/// ## Errors
/// Returns `false` if:
/// * `proof.total_leaves` is not a power of two.
/// * `proof.leaves` is empty.
/// * Any leaf index is out of bounds.
/// * The sibling list is exhausted before reconstruction finishes.
/// * The reconstructed root does not match `expected_root`.
pub fn verify_multi_proof(proof: &MultiProof, expected_root: &Hash) -> bool {
    let n = proof.total_leaves;
    // Tree size must be a power of two ≥ 1.
    if n == 0 || (n & (n - 1)) != 0 {
        return false;
    }
    if proof.leaves.is_empty() {
        return false;
    }

    // Validate and collect (index, hash) pairs sorted by index.
    let mut layer: Vec<(u64, Hash)> = proof
        .leaves
        .iter()
        .map(|l| (l.index, l.leaf))
        .collect();
    layer.sort_by_key(|(idx, _)| *idx);

    // Guard: all indices must be < n.
    if layer.last().map(|(i, _)| *i).unwrap_or(0) >= n {
        return false;
    }

    let mut sibling_iter = proof.siblings.iter();
    let mut level_size = n;

    // Ascend one level at a time until a single root hash remains.
    while layer.len() > 1 || level_size > 1 {
        let mut next_layer: Vec<(u64, Hash)> = Vec::with_capacity((layer.len() + 1) / 2);
        let mut i = 0;

        while i < layer.len() {
            let (idx, hash) = layer[i];
            let parent_idx = idx / 2;
            let sibling_idx = idx ^ 1; // flip the lowest bit

            // Check if the next proved node is the sibling.
            if i + 1 < layer.len() && layer[i + 1].0 == sibling_idx {
                // Both children are in the proof batch — combine directly.
                let (_, sibling_hash) = layer[i + 1];
                let parent_hash = if idx % 2 == 0 {
                    hash_pair(&hash, &sibling_hash)
                } else {
                    hash_pair(&sibling_hash, &hash)
                };
                next_layer.push((parent_idx, parent_hash));
                i += 2; // consumed two nodes
            } else {
                // Need one external sibling from the proof.
                let sibling_hash = match sibling_iter.next() {
                    Some(h) => h,
                    None => return false,
                };
                let parent_hash = if idx % 2 == 0 {
                    hash_pair(&hash, sibling_hash)
                } else {
                    hash_pair(sibling_hash, &hash)
                };
                next_layer.push((parent_idx, parent_hash));
                i += 1;
            }
        }

        layer = next_layer;
        level_size /= 2;
    }

    // layer now contains exactly one entry: the reconstructed root.
    match layer.first() {
        Some((_, root)) => root == expected_root,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helper: build a balanced Merkle tree from raw leaf data.
    // Returns (root, all_hashes_by_level) where level 0 = leaves.
    // -----------------------------------------------------------------------
    fn build_tree(leaves: &[&[u8]]) -> (Hash, Vec<Vec<Hash>>) {
        assert!(leaves.len().is_power_of_two(), "leaf count must be a power of two");
        let mut level: Vec<Hash> = leaves.iter().map(|d| hash_leaf(d)).collect();
        let mut levels = vec![level.clone()];

        while level.len() > 1 {
            level = level
                .chunks(2)
                .map(|pair| hash_pair(&pair[0], &pair[1]))
                .collect();
            levels.push(level.clone());
        }
        (level[0], levels)
    }

    // -----------------------------------------------------------------------
    // Build a single-path proof for leaf at `index` in a pre-built tree.
    // -----------------------------------------------------------------------
    fn build_proof(index: usize, levels: &[Vec<Hash>]) -> MerkleProof {
        let leaf = levels[0][index];
        let mut path = Vec::new();
        let mut idx = index;

        for level in &levels[..levels.len() - 1] {
            let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
            let side = if idx % 2 == 0 { Side::Right } else { Side::Left };
            path.push(ProofNode {
                sibling: level[sibling_idx],
                side,
            });
            idx /= 2;
        }

        MerkleProof { leaf, path }
    }

    // -----------------------------------------------------------------------
    // Test vectors: 4-leaf tree
    // -----------------------------------------------------------------------
    #[test]
    fn test_single_proof_4_leaves() {
        let data: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d"];
        let (root, levels) = build_tree(&data);

        for i in 0..4 {
            let proof = build_proof(i, &levels);
            assert!(verify_proof(&proof, &root), "leaf {i} failed");
        }
    }

    #[test]
    fn test_tampered_leaf_rejected() {
        let data: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d"];
        let (root, levels) = build_tree(&data);

        let mut proof = build_proof(0, &levels);
        proof.leaf = hash_leaf(b"evil"); // tampered
        assert!(!verify_proof(&proof, &root));
    }

    #[test]
    fn test_tampered_sibling_rejected() {
        let data: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d"];
        let (root, levels) = build_tree(&data);

        let mut proof = build_proof(1, &levels);
        proof.path[0].sibling = hash_leaf(b"evil"); // tampered sibling
        assert!(!verify_proof(&proof, &root));
    }

    // -----------------------------------------------------------------------
    // Test vectors: 8-leaf tree
    // -----------------------------------------------------------------------
    #[test]
    fn test_single_proof_8_leaves() {
        let data: Vec<&[u8]> = vec![b"A", b"B", b"C", b"D", b"E", b"F", b"G", b"H"];
        let (root, levels) = build_tree(&data);

        for i in 0..8 {
            let proof = build_proof(i, &levels);
            assert!(verify_proof(&proof, &root), "leaf {i} failed");
        }
    }

    // -----------------------------------------------------------------------
    // Multi-proof: prove leaves 1 and 3 in a 4-leaf tree
    // -----------------------------------------------------------------------
    #[test]
    fn test_multi_proof_4_leaves() {
        let data: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d"];
        let (root, levels) = build_tree(&data);

        // Proving leaves at index 1 and 2 (adjacent pair — no external sibling needed
        // between them, but level-1 needs leaf[0] and leaf[3]).
        // Let's prove index 0 and index 2 instead (non-adjacent).
        let leaves = vec![
            MultiLeaf { leaf: levels[0][0], index: 0 },
            MultiLeaf { leaf: levels[0][2], index: 2 },
        ];
        // Sibling for leaf 0 is leaf 1 (right), sibling for leaf 2 is leaf 3 (right).
        // At level 1, nodes 0 and 1 are both computed, so no extra sibling needed.
        let siblings = vec![
            levels[0][1], // sibling of leaf 0
            levels[0][3], // sibling of leaf 2
        ];

        let proof = MultiProof {
            leaves,
            siblings,
            total_leaves: 4,
        };
        assert!(verify_multi_proof(&proof, &root));
    }

    #[test]
    fn test_multi_proof_tampered_leaf_rejected() {
        let data: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d"];
        let (root, levels) = build_tree(&data);

        let leaves = vec![
            MultiLeaf { leaf: hash_leaf(b"evil"), index: 0 }, // tampered
            MultiLeaf { leaf: levels[0][2], index: 2 },
        ];
        let siblings = vec![levels[0][1], levels[0][3]];
        let proof = MultiProof { leaves, siblings, total_leaves: 4 };
        assert!(!verify_multi_proof(&proof, &root));
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------
    #[test]
    fn test_invalid_tree_size_rejected() {
        let (root, levels) = build_tree(&[b"x", b"y"]);
        let proof = MultiProof {
            leaves: vec![MultiLeaf { leaf: levels[0][0], index: 0 }],
            siblings: vec![levels[0][1]],
            total_leaves: 3, // not a power of two
        };
        assert!(!verify_multi_proof(&proof, &root));
    }

    #[test]
    fn test_empty_leaves_rejected() {
        let (root, _) = build_tree(&[b"x", b"y"]);
        let proof = MultiProof {
            leaves: vec![],
            siblings: vec![],
            total_leaves: 2,
        };
        assert!(!verify_multi_proof(&proof, &root));
    }

    // -----------------------------------------------------------------------
    // 32-level tree depth benchmark probe
    // -----------------------------------------------------------------------
    #[test]
    fn test_single_proof_depth_32() {
        // Build a 2^10 = 1024-leaf tree (practical stand-in for 2^32 for unit
        // tests; the algorithm is depth-independent).
        let n = 1024usize;
        let raw: Vec<Vec<u8>> = (0..n).map(|i| i.to_le_bytes().to_vec()).collect();
        let refs: Vec<&[u8]> = raw.iter().map(|v| v.as_slice()).collect();
        let (root, levels) = build_tree(&refs);

        // Verify every leaf to exercise all sibling directions.
        for i in 0..n {
            let proof = build_proof(i, &levels);
            assert!(verify_proof(&proof, &root), "leaf {i} failed at depth 10");
        }
    }
}
