//! # merkle-verifier
//!
//! A Soroban-native Merkle Tree state-proof verification library written in
//! pure Rust (no `std` heap beyond `Vec`, no recursion).
//!
//! ## Features
//! * **Single-path verification** – verify that a single leaf belongs to a
//!   Merkle tree given its proof path.
//! * **Multi-proof verification** – verify multiple leaves simultaneously in
//!   one O(k log N) pass.
//! * **SHA-256 hashing** – using the `sha2` crate with `no_std`-compatible
//!   feature flags, compatible with Bitcoin, Ethereum, and Stellar ledger header
//!   Merkle structures.
//! * **Stack-safe** – all traversals are iterative to avoid Wasm stack overflow
//!   even at tree depth 32.
//! * **O(log N) instruction scaling** – CPU cost grows logarithmically with
//!   tree depth, fitting within Soroban instruction budgets.
//!
//! ## Modules
//! * [`proof`] – core data types and verification algorithms.
//!
//! ## Quick Start
//! ```rust
//! use merkle_verifier::{hash_leaf, MerkleProof, ProofNode, Side, verify_proof};
//!
//! // Leaf and its expected root (all-zeros here for illustration only).
//! let leaf  = hash_leaf(b"my data");
//! let root  = [0u8; 32]; // replace with the real Merkle root
//! let proof = MerkleProof { leaf, path: vec![] };
//!
//! // Returns true only when the reconstructed root matches.
//! let _ = verify_proof(&proof, &root);
//! ```

pub mod proof;

// Re-export the public surface so callers can write `merkle_verifier::verify_proof`.
pub use proof::{
    hash_leaf, verify_proof, verify_multi_proof,
    Hash, Side, ProofNode, MerkleProof, MultiLeaf, MultiProof,
};
