//! Off-chain state Merkle proof verification primitive.
//!
//! Verifies SHA-256 and Keccak-256 Merkle proofs against on-chain root hashes,
//! with support for both single-element and multi-element proofs, and automatic
//! expiration of historic roots.
//!
//! # Sub-modules
//!
//! - [`verify`] — hash primitives, proof generation, and proof verification
//! - [`lib`]    — on-chain root registry and verification entry points

pub mod lib;
pub mod verify;

pub use lib::{MerkleError, MerkleRoot, MerkleVerifier};
pub use verify::{
    compute_root, compute_root_from_hashes, generate_proof, hash_leaf, hash_node, verify_multiproof,
    verify_proof, HashAlgorithm, LeafProof, ProofElement, ProofSide, LEAF_PREFIX, NODE_PREFIX,
};
