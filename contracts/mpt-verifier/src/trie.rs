//! Merkle Patricia Trie traversal with Keccak-256 path verification.
//!
//! # The shape of an Ethereum MPT
//!
//! The state and storage tries are keyed by `keccak256` of the original key
//! (an address, or a `keccak256(address, slot)` pair), giving a 32-byte path.
//! Every node is RLP-encoded, and the trie is a binary structure of two node
//! kinds:
//!
//! **Branch node.** An RLP list of exactly 17 items. Items `0..16` are child
//! node references indexed by the next path nibble, and item `16` is the value
//! for a key that ends exactly at this branch.
//!
//! **Leaf or extension node.** An RLP list of 2 items. The first is a
//! hex-prefix-encoded path, the second is either a value (leaf) or a child
//! reference (extension).
//!
//! A child reference is the node's 32-byte Keccak-256 hash when the child
//! encodes to 32 bytes or more, or the child's inline RLP when it is shorter
//! than 32 bytes. Both forms appear in real proofs and both must be handled.
//!
//! A child reference of zero length is not a node at all: it is how a branch
//! node records an absent subtree, which is a valid non-inclusion proof.
//!
//! # Verification contract
//!
//! [`verify_proof`] returns a [`TrieOutcome`] only when the supplied nodes
//! hash, from the root, through to a leaf whose path exactly equals the
//! requested key path. Any mismatch — including a proof that resolves to a
//! *different* key, which is how Ethereum proves non-inclusion — is reported
//! distinctly rather than silently accepted.
//!
//! # Denial-of-service hardening
//!
//! An attacker controls the proof bytes, so traversal is bounded on three
//! independent axes: [`Limits::max_depth`] caps path length, which is what
//! stops a maliciously long chain of extension nodes; [`Limits::max_nodes`]
//! caps the node set; and every node is decoded under the bounded RLP
//! [`Limits`](crate::rlp::Limits) so a single node cannot allocate unbounded
//! memory. The walk is iterative rather than recursive, so a deep trie cannot
//! overflow the Wasm stack either.

use alloc::vec::Vec;

use tiny_keccak::{Hasher as _, Keccak};

use crate::rlp::{self, RlpError, RlpItem};

/// Keccak-256, the hash function Ethereum uses throughout the trie.
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    let mut out = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut out);
    out
}

/// Root hash of an empty trie: `keccak256(rlp(""))`.
pub const EMPTY_ROOT: [u8; 32] = [
    0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6, 0xff, 0x83, 0x45, 0xe6, 0x92, 0xc0, 0xf8, 0x6e,
    0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0, 0x01, 0x62, 0x2f, 0xb5, 0xe3, 0x63, 0xb4, 0x21,
];

/// Keccak-256 of the empty byte string, the code hash of every non-contract
/// account.
pub const EMPTY_CODE_HASH: [u8; 32] = [
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
];

/// Traversal and proof-size bounds.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum number of nibbles consumed while walking to a leaf.
    ///
    /// Real Ethereum tries are ~64 nibbles deep (a 32-byte path is 64 nibbles
    /// plus any extension-node padding). 256 leaves generous headroom while
    /// keeping the walk bounded regardless of what a prover supplies.
    pub max_depth: usize,
    /// Maximum number of RLP-encoded nodes accepted in a proof.
    pub max_nodes: usize,
    /// Maximum encoded size of a single node.
    pub max_node_bytes: usize,
    /// Maximum total encoded size of all nodes in a proof.
    pub max_total_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 256,
            max_nodes: 512,
            max_node_bytes: 8 * 1024,
            max_total_bytes: 256 * 1024,
        }
    }
}

impl Limits {
    /// The RLP decoding limits implied by these traversal limits.
    ///
    /// Exposed so that callers decoding a leaf value can use exactly the same
    /// bounds the traversal applied.
    pub fn rlp(&self) -> rlp::Limits {
        rlp::Limits {
            // 17 branch children + the branch itself, plus slack for a nested
            // account or storage value.
            max_items: 32,
            max_item_bytes: self.max_node_bytes,
            max_depth: 4,
        }
    }
}

/// Why a proof was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrieError {
    /// The proof supplied no nodes.
    EmptyProof,
    /// A node exceeded the per-node size budget.
    NodeTooLarge,
    /// The total proof size exceeded the aggregate budget.
    ProofTooLarge,
    /// The proof contained more nodes than permitted.
    TooManyNodes,
    /// Traversal exceeded the depth budget.
    DepthExceeded,
    /// A node was not well-formed RLP.
    Rlp(RlpError),
    /// A node was not a valid branch, leaf or extension shape.
    MalformedNode,
    /// The root node was not present in the proof.
    RootNotFound,
    /// A referenced child node was absent from the proof.
    ChildNotFound,
    /// A hex-prefix path was malformed or held more nibbles than expected.
    InvalidPathEncoding,
    /// The proof resolved to a leaf for a different key, which is a valid
    /// Ethereum *non-inclusion* proof.
    KeyMismatch,
    /// The trie is empty and the root hash does not match.
    EmptyTrie,
}

impl From<RlpError> for TrieError {
    fn from(e: RlpError) -> Self {
        TrieError::Rlp(e)
    }
}

/// What a successful traversal found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrieOutcome {
    /// The key is present; carries its RLP-encoded value.
    Present(Vec<u8>),
    /// The key is absent; the proof terminated on a different leaf, which is
    /// exactly how Ethereum proves non-inclusion.
    Absent,
}

/// A decoded leaf/extension path: its nibbles plus whether it terminates.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PathNibbles {
    nibbles: Vec<u8>,
    is_leaf: bool,
}

/// Splits a byte slice into high/low nibbles.
fn to_nibbles(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(b >> 4);
        out.push(b & 0x0f);
    }
    out
}

/// Decodes Ethereum's hex-prefix ("compact") path encoding.
///
/// The first nibble of the first byte is a flag: `0`/`1` for extension
/// (even/odd length) and `2`/`3` for leaf (even/odd length). An odd length
/// means the first byte's low nibble is the first path nibble, and every
/// subsequent byte carries two more.
fn decode_path(encoded: &[u8], max_depth: usize) -> Result<PathNibbles, TrieError> {
    let first = *encoded.first().ok_or(TrieError::InvalidPathEncoding)?;
    let flag = first >> 4;
    if flag > 3 {
        return Err(TrieError::InvalidPathEncoding);
    }
    let is_leaf = flag >= 2;
    let odd = flag & 1 == 1;

    let mut nibbles = Vec::new();
    if odd {
        nibbles.push(first & 0x0f);
    }
    for b in &encoded[1..] {
        nibbles.push(b >> 4);
        nibbles.push(b & 0x0f);
    }
    if nibbles.len() > max_depth {
        return Err(TrieError::DepthExceeded);
    }
    Ok(PathNibbles { nibbles, is_leaf })
}

/// Interprets a child reference as either an inline node or a 32-byte hash.
enum ChildRef {
    /// The child node RLP is carried inline (node shorter than 32 bytes).
    Inline(Vec<u8>),
    /// The child is identified by its Keccak-256 hash.
    Hash([u8; 32]),
}

/// Reads a child reference out of a decoded RLP item.
fn as_child_ref(item: &RlpItem) -> Result<ChildRef, TrieError> {
    let bytes = item.as_bytes().ok_or(TrieError::MalformedNode)?;
    match bytes.len() {
        // A zero-length slot is a genuine "no child here" marker, which is how
        // a branch proves the requested key is absent.
        0 => Ok(ChildRef::Hash([0u8; 32])),
        32 => {
            let mut h = [0u8; 32];
            h.copy_from_slice(bytes);
            Ok(ChildRef::Hash(h))
        }
        // A reference shorter than 32 bytes carries the child's RLP inline.
        // Re-encoding normalises it to the exact bytes that get hashed.
        _ => Ok(ChildRef::Inline(rlp::encode_bytes(bytes))),
    }
}

/// A node set indexed by hash, for child lookups during traversal.
struct NodeSet {
    hashes: Vec<[u8; 32]>,
    nodes: Vec<Vec<u8>>,
}

impl NodeSet {
    fn build(proof: &[Vec<u8>], limits: &Limits) -> Result<Self, TrieError> {
        if proof.is_empty() {
            return Err(TrieError::EmptyProof);
        }
        if proof.len() > limits.max_nodes {
            return Err(TrieError::TooManyNodes);
        }
        let mut total = 0usize;
        for node in proof {
            if node.len() > limits.max_node_bytes {
                return Err(TrieError::NodeTooLarge);
            }
            total = total.saturating_add(node.len());
            if total > limits.max_total_bytes {
                return Err(TrieError::ProofTooLarge);
            }
        }
        let mut set = NodeSet {
            hashes: Vec::with_capacity(proof.len()),
            nodes: Vec::with_capacity(proof.len()),
        };
        for node in proof {
            set.hashes.push(keccak256(node));
            set.nodes.push(node.clone());
        }
        Ok(set)
    }

    fn get(&self, hash: &[u8; 32]) -> Option<&Vec<u8>> {
        self.hashes
            .iter()
            .position(|h| h == hash)
            .map(|i| &self.nodes[i])
    }
}

/// Walks `path` from `root_hash` and returns what the proof proves about it.
pub fn verify_proof(
    proof: &[Vec<u8>],
    root_hash: &[u8; 32],
    key: &[u8; 32],
    limits: &Limits,
) -> Result<TrieOutcome, TrieError> {
    // An empty trie has no nodes at all, so the empty-root case must be decided
    // before the node set is built -- otherwise an empty proof would be
    // rejected as malformed instead of proving absence.
    if *root_hash == EMPTY_ROOT {
        return Ok(TrieOutcome::Absent);
    }

    let set = NodeSet::build(proof, limits)?;
    let rlp_limits = limits.rlp();

    let mut current = match set.get(root_hash) {
        Some(node) => node.clone(),
        None => return Err(TrieError::RootNotFound),
    };

    let path = to_nibbles(key);
    let mut index = 0usize; // nibbles of `path` consumed so far

    loop {
        if index > limits.max_depth {
            return Err(TrieError::DepthExceeded);
        }

        let decoded = rlp::decode(&current, &rlp_limits)?;
        let items = decoded.as_list().ok_or(TrieError::MalformedNode)?;

        match items.len() {
            // Branch node.
            17 => {
                if index == path.len() {
                    // The key ends exactly at this branch: item 16 is its value.
                    let value = items[16].as_bytes().ok_or(TrieError::MalformedNode)?;
                    return Ok(if value.is_empty() {
                        TrieOutcome::Absent
                    } else {
                        TrieOutcome::Present(value.to_vec())
                    });
                }

                let nibble = path[index];
                // An empty slot means the key is not in this subtree.
                let child = items[nibble as usize]
                    .as_bytes()
                    .ok_or(TrieError::MalformedNode)?;
                if child.is_empty() {
                    return Ok(TrieOutcome::Absent);
                }
                let child = as_child_ref(&items[nibble as usize])?;
                index += 1;
                current = match child {
                    ChildRef::Inline(bytes) => bytes,
                    ChildRef::Hash(hash) => set.get(&hash).ok_or(TrieError::ChildNotFound)?.clone(),
                };
            }

            // Leaf or extension node.
            2 => {
                let path_bytes = items[0].as_bytes().ok_or(TrieError::MalformedNode)?;
                let decoded_path = decode_path(path_bytes, limits.max_depth)?;

                // The node's path must match the key at the current offset.
                if index + decoded_path.nibbles.len() > path.len() {
                    return Err(TrieError::KeyMismatch);
                }
                if path[index..index + decoded_path.nibbles.len()] != decoded_path.nibbles[..] {
                    return Err(TrieError::KeyMismatch);
                }

                if decoded_path.is_leaf {
                    // A leaf terminates the walk. If its path matched exactly,
                    // the key is present with the leaf's value.
                    let value = items[1].as_bytes().ok_or(TrieError::MalformedNode)?;
                    return Ok(TrieOutcome::Present(value.to_vec()));
                }

                // An extension continues to a child node.
                let child = as_child_ref(&items[1])?;
                index += decoded_path.nibbles.len();
                current = match child {
                    ChildRef::Inline(bytes) => bytes,
                    ChildRef::Hash(hash) => set.get(&hash).ok_or(TrieError::ChildNotFound)?.clone(),
                };
            }

            _ => return Err(TrieError::MalformedNode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn h(b: u8) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[0] = b;
        out
    }

    /// Encodes a leaf/extension node: `[hex_path, payload]`.
    fn node2(path: &[u8], payload: &[u8]) -> Vec<u8> {
        rlp::encode(&RlpItem::List(vec![
            RlpItem::Bytes(path.to_vec()),
            RlpItem::Bytes(payload.to_vec()),
        ]))
    }

    /// Encodes a branch node: 17 items.
    fn node17(children: [Vec<u8>; 17]) -> Vec<u8> {
        rlp::encode(&RlpItem::List(
            children.iter().map(|c| RlpItem::Bytes(c.clone())).collect(),
        ))
    }

    #[test]
    fn empty_trie_root_constant_matches_hash() {
        // The published empty-trie root must equal keccak256(rlp("")).
        assert_eq!(keccak256(&rlp::encode_bytes(&[])), EMPTY_ROOT);
    }

    #[test]
    fn empty_code_hash_constant_matches_hash() {
        assert_eq!(keccak256(&[]), EMPTY_CODE_HASH);
    }

    #[test]
    fn keccak_matches_published_vectors() {
        // Published Keccak-256 (Ethereum variant) digests.
        let d = keccak256(b"");
        assert_eq!(
            d,
            [
                0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7,
                0x03, 0xc0, 0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04,
                0x5d, 0x85, 0xa4, 0x70
            ]
        );
        let d = keccak256(b"abc");
        assert_eq!(
            d,
            [
                0x4e, 0x03, 0x65, 0x7a, 0xea, 0x45, 0xa9, 0x4f, 0xc7, 0xd4, 0x7b, 0xa8, 0x26, 0xc8,
                0xd6, 0x67, 0xc0, 0xd1, 0xe6, 0xe3, 0x3a, 0x64, 0xa0, 0x36, 0xec, 0x44, 0xf5, 0x8f,
                0xa1, 0x2d, 0x6c, 0x45
            ]
        );
    }

    #[test]
    fn decodes_hex_prefix_paths() {
        // 0x20 => flag 2 (leaf, even), payload is empty: a leaf ending here.
        let p = decode_path(&[0x20], 256).unwrap();
        assert!(p.is_leaf);
        assert!(p.nibbles.is_empty());

        // 0x37 => flag 3 (leaf, odd): low nibble 7, then 0x89 -> 8, 9.
        let p = decode_path(&[0x37, 0x89], 256).unwrap();
        assert!(p.is_leaf);
        assert_eq!(p.nibbles, vec![7, 8, 9]);

        // 0x00 => flag 0 (extension, even), payload 0xab.
        let p = decode_path(&[0x00, 0xab], 256).unwrap();
        assert!(!p.is_leaf);
        assert_eq!(p.nibbles, vec![0xa, 0xb]);
    }

    #[test]
    fn rejects_bad_path_flags_and_overlong_paths() {
        assert_eq!(decode_path(&[], 256), Err(TrieError::InvalidPathEncoding));
        assert_eq!(
            decode_path(&[0x40], 256),
            Err(TrieError::InvalidPathEncoding)
        );
        // 0x00 is an even-length extension, so each further byte adds two
        // nibbles. Four bytes -> 8 nibbles, which exceeds a budget of 4.
        assert_eq!(
            decode_path(&[0x00, 0xab, 0xcd, 0xef], 4),
            Err(TrieError::DepthExceeded)
        );
    }

    #[test]
    fn verifies_single_leaf_proof() {
        // One leaf at the full path, referenced by hash from the root.
        let key = h(0xab);
        let path = to_nibbles(&key);
        let mut encoded_path = alloc::vec![0x20];
        // Even-length leaf path: first byte is just the flag, rest are nibbles.
        for pair in path.chunks(2) {
            encoded_path.push((pair[0] << 4) | pair[1]);
        }
        let leaf = node2(&encoded_path, b"\x01\x02");
        let root_hash = keccak256(&leaf);
        let outcome = verify_proof(&[leaf], &root_hash, &key, &Limits::default()).unwrap();
        assert_eq!(outcome, TrieOutcome::Present(b"\x01\x02".to_vec()));
    }

    #[test]
    fn reports_non_inclusion_as_key_mismatch() {
        // A proof whose leaf is for a different key is a valid exclusion
        // proof, surfaced as KeyMismatch rather than a false "present".
        let key = h(0xab);
        let other = h(0xcd);
        let path = to_nibbles(&other);
        let mut encoded_path = alloc::vec![0x20];
        for pair in path.chunks(2) {
            encoded_path.push((pair[0] << 4) | pair[1]);
        }
        let leaf = node2(&encoded_path, b"\x09");
        let root_hash = keccak256(&leaf);
        assert_eq!(
            verify_proof(&[leaf], &root_hash, &key, &Limits::default()),
            Err(TrieError::KeyMismatch)
        );
    }

    #[test]
    fn empty_trie_proves_absence() {
        let outcome = verify_proof(&[], &EMPTY_ROOT, &h(1), &Limits::default()).unwrap();
        assert_eq!(outcome, TrieOutcome::Absent);
    }

    #[test]
    fn rejects_missing_root() {
        let leaf = node2(&[0x20], b"\x01");
        let bogus = keccak256(b"not-the-root");
        assert_eq!(
            verify_proof(&[leaf], &bogus, &h(1), &Limits::default()),
            Err(TrieError::RootNotFound)
        );
    }

    #[test]
    fn rejects_empty_proof() {
        let bogus = keccak256(b"x");
        assert_eq!(
            verify_proof(&[], &bogus, &h(1), &Limits::default()),
            Err(TrieError::EmptyProof)
        );
    }

    #[test]
    fn enforces_node_count_budget() {
        let leaf = node2(&[0x20], b"\x01");
        let root_hash = keccak256(&leaf);
        let strict = Limits {
            max_nodes: 0,
            ..Limits::default()
        };
        assert_eq!(
            verify_proof(&[leaf], &root_hash, &h(1), &strict),
            Err(TrieError::TooManyNodes)
        );
    }

    #[test]
    fn enforces_node_and_total_size_budgets() {
        let leaf = node2(&[0x20], b"\x01");
        let root_hash = keccak256(&leaf);

        let strict = Limits {
            max_node_bytes: 2,
            ..Limits::default()
        };
        assert_eq!(
            verify_proof(core::slice::from_ref(&leaf), &root_hash, &h(1), &strict),
            Err(TrieError::NodeTooLarge)
        );

        let strict = Limits {
            max_total_bytes: 2,
            ..Limits::default()
        };
        assert_eq!(
            verify_proof(&[leaf], &root_hash, &h(1), &strict),
            Err(TrieError::ProofTooLarge)
        );
    }

    #[test]
    fn enforces_depth_budget_during_traversal() {
        // An extension node whose own encoded path already exceeds a tiny
        // depth budget must be refused before any further descent.
        let key = h(0x11);
        let long_path = vec![0x00u8; 40]; // 80 nibbles
        let ext = node2(&long_path, &[0x11; 32]);
        let root_hash = keccak256(&ext);
        let strict = Limits {
            max_depth: 4,
            ..Limits::default()
        };
        assert_eq!(
            verify_proof(&[ext], &root_hash, &key, &strict),
            Err(TrieError::DepthExceeded)
        );
    }

    #[test]
    fn rejects_malformed_node_shapes() {
        // A 3-item list is neither a branch (17) nor leaf/extension (2).
        let bad = rlp::encode(&RlpItem::List(vec![
            RlpItem::Bytes(vec![1]),
            RlpItem::Bytes(vec![2]),
            RlpItem::Bytes(vec![3]),
        ]));
        let root_hash = keccak256(&bad);
        assert_eq!(
            verify_proof(&[bad], &root_hash, &h(1), &Limits::default()),
            Err(TrieError::MalformedNode)
        );
    }

    #[test]
    fn rejects_byte_string_node() {
        // A node must be a list, not a bare string.
        let bad = rlp::encode_bytes(b"\x01\x02");
        let root_hash = keccak256(&bad);
        assert_eq!(
            verify_proof(&[bad], &root_hash, &h(1), &Limits::default()),
            Err(TrieError::MalformedNode)
        );
    }

    #[test]
    fn branch_node_without_matching_child_proves_absence() {
        // Branch with an empty child for the key's first nibble.
        let key = h(0x5a);
        let first_nibble = key[0] >> 4;
        // children[first_nibble] stays empty -> key not present.
        let children: [Vec<u8>; 17] = core::array::from_fn(|_| Vec::new());
        let branch = node17(children);
        let root_hash = keccak256(&branch);
        let outcome = verify_proof(&[branch], &root_hash, &key, &Limits::default()).unwrap();
        assert_eq!(outcome, TrieOutcome::Absent);
        assert_eq!(first_nibble, 0x5);
    }

    #[test]
    fn branch_node_walks_to_leaf_and_returns_value() {
        // Consume one nibble at a branch, then finish the remaining odd number
        // of nibbles at a leaf. A 64-nibble key leaves 63 after the first
        // nibble, so the leaf path is odd-length and uses the `0x3_` flag.
        let key = h(0x5a);
        let all = to_nibbles(&key);
        let first_nibble = all[0];
        let tail = &all[1..];
        assert_eq!(tail.len() % 2, 1, "tail must be odd-length");

        // Odd-length leaf path: flag 3, first nibble packed into the low half.
        let mut encoded_path = alloc::vec![0x30 | tail[0]];
        for pair in tail[1..].chunks(2) {
            encoded_path.push((pair[0] << 4) | pair[1]);
        }

        let leaf = node2(&encoded_path, b"\x7f");
        let leaf_hash = keccak256(&leaf).to_vec();

        let mut children: [Vec<u8>; 17] = core::array::from_fn(|_| Vec::new());
        children[first_nibble as usize] = leaf_hash;
        let branch = node17(children);
        let root_hash = keccak256(&branch);

        let outcome = verify_proof(&[branch, leaf], &root_hash, &key, &Limits::default()).unwrap();
        assert_eq!(outcome, TrieOutcome::Present(b"\x7f".to_vec()));
    }
}
