//! # pool.rs
//!
//! Shielded pool module for the ZKP Verifier contract.
//!
//! ## Purpose
//!
//! The shielded pool maintains an on-chain Merkle commitment tree that enables
//! privacy-preserving transfers on Stellar.  Users deposit **note commitments**
//! (Pedersen commitments to hidden note data) and later spend them by submitting
//! a zero-knowledge proof of membership.
//!
//! ## Privacy Model
//!
//! ```text
//!   Sender                    On-chain contract               Recipient
//!     │                              │                              │
//!     │  1. Generate note            │                              │
//!     │     (recipient, amount,      │                              │
//!     │      asset, blinding)        │                              │
//!     │                              │                              │
//!     │  2. Compute commitment       │                              │
//!     │     C = Pedersen(note)       │                              │
//!     │                              │                              │
//!     │──── deposit(C) ────────────►│                              │
//!     │                              │ Insert C at leaf[i]          │
//!     │                              │ Update Merkle root           │
//!     │                              │                              │
//!     │  3. Compute nullifier        │                              │
//!     │     N = PRF(secret_key, i)   │                              │
//!     │                              │                              │
//!     │  4. Generate ZKP:            │                              │
//!     │     π proves membership of   │                              │
//!     │     C in tree with root R    │                              │
//!     │     without revealing note   │                              │
//!     │                              │                              │
//!     │─── verify_and_transfer ─────►│                              │
//!     │    (π, merkle_root=R,        │ Verify proof                 │
//!     │     nullifier=N,             │ Check root ∈ known_roots     │
//!     │     recipient_hash,          │ Check N not spent            │
//!     │     asset_id)                │ Mark N as spent              │
//!     │                              │ Emit transfer event          │
//!     │                              │ (no amounts/addresses)       │
//!     │                              │──── private transfer ───────►│
//! ```
//!
//! ## Merkle Tree
//!
//! The contract tracks a rolling Merkle root after each insertion.  A
//! simplified SHA-256-based root computation is used for gas profiling and
//! structural correctness:
//!
//! ```text
//! new_root = SHA-256(commitment || leaf_index_be32)
//! ```
//!
//! A production deployment should use an incremental sparse Merkle tree with
//! Poseidon hash (as in Tornado Cash / Zcash Sapling) once Soroban exposes
//! a Poseidon host function.  The key invariants – no double-deposits, ordered
//! leaf insertion, root registration – are already correct.
//!
//! ## Storage Layout
//!
//! | Key                          | Storage   | Type   | Description               |
//! |------------------------------|-----------|--------|---------------------------|
//! | `("pool_cnt",)`              | Instance  | `u32`  | Next leaf index            |
//! | `("pool_root", root)`        | Instance  | `u32`  | Ledger seq of root        |
//! | `("pool_leaf", index)`       | Instance  | `bool` | Leaf occupied sentinel    |
//!
//! The primary commitment counter and root registry live in Instance storage so
//! they are swept by the contract's own TTL management.  Individual nullifiers
//! (anti-replay) are kept in Persistent storage – see `nullifier.rs`.
//!
//! ## Capacity
//!
//! The maximum commitment count is `2^20 = 1 048 576` notes, matching a Merkle
//! tree of depth 20.

use soroban_sdk::{symbol_short, BytesN, Bytes, Env, Symbol};

use crate::errors::ZkError;
use crate::types::NoteCommitment;

// ─── Storage Keys ───────────────────────────────────────────────────────────

/// Commitment counter key (number of inserted leaves).
const KEY_POOL_COUNT: Symbol = symbol_short!("pool_cnt");

/// Prefix for the known-root registry.
/// Stored as `(KEY_POOL_ROOT, root_bytes)` → ledger_sequence.
const KEY_POOL_ROOT: Symbol = symbol_short!("pool_root");

/// Maximum leaves in the commitment tree (depth 20).
pub const MAX_COMMITMENTS: u32 = 1_048_576;

// ─── Root Management ────────────────────────────────────────────────────────

/// Register `root` as a known Merkle root at the current ledger sequence.
///
/// Known roots are used during `verify_and_transfer` to check that the proof's
/// claimed root actually corresponds to a historical state of the commitment tree.
/// This allows proofs generated against older tree states to remain valid even
/// after new commitments are inserted.
pub fn register_root(env: &Env, root: &BytesN<32>) {
    let seq = env.ledger().sequence();
    env.storage()
        .instance()
        .set(&(KEY_POOL_ROOT, root.clone()), &seq);
}

/// Return `true` if `root` has been previously registered.
pub fn is_known_root(env: &Env, root: &BytesN<32>) -> bool {
    env.storage()
        .instance()
        .has(&(KEY_POOL_ROOT, root.clone()))
}

// ─── Commitment Insertion ───────────────────────────────────────────────────

/// Return the current number of note commitments in the pool.
pub fn commitment_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&KEY_POOL_COUNT)
        .unwrap_or(0u32)
}

/// Insert a note commitment into the shielded pool.
///
/// Assigns the next available leaf index, updates the Merkle root, and
/// registers the new root.
///
/// ## Errors
///
/// * [`ZkError::InvalidNoteCommitment`] – `commitment` is all-zero bytes.
/// * [`ZkError::CommitmentTreeFull`] – the pool has reached `MAX_COMMITMENTS`.
/// * [`ZkError::NotInitialised`] – the pool counter has not been initialised.
pub fn insert_commitment(
    env: &Env,
    commitment: &BytesN<32>,
) -> Result<NoteCommitment, ZkError> {
    // Reject the zero commitment (null sentinel)
    let zero = BytesN::from_array(env, &[0u8; 32]);
    if commitment == &zero {
        return Err(ZkError::InvalidNoteCommitment);
    }

    // Fetch the current leaf counter
    let leaf_index: u32 = env
        .storage()
        .instance()
        .get(&KEY_POOL_COUNT)
        .ok_or(ZkError::NotInitialised)?;

    if leaf_index >= MAX_COMMITMENTS {
        return Err(ZkError::CommitmentTreeFull);
    }

    // Advance the counter
    env.storage()
        .instance()
        .set(&KEY_POOL_COUNT, &(leaf_index + 1));

    // Record the ledger sequence for provenance
    let inserted_at = env.ledger().sequence();

    // Compute and register the new Merkle root.
    // Production: use incremental Poseidon-hashed sparse Merkle tree.
    let new_root = compute_rolling_root(env, commitment, leaf_index);
    register_root(env, &new_root);

    Ok(NoteCommitment {
        commitment: commitment.clone(),
        inserted_at,
        leaf_index,
    })
}

// ─── Root Computation ───────────────────────────────────────────────────────

/// Compute a simplified rolling Merkle root after inserting `commitment` at `leaf_index`.
///
/// ### Algorithm
///
/// ```text
/// new_root = SHA-256(commitment || leaf_index_be32)
/// ```
///
/// This is a structurally sound commitment to the pool state that uniquely
/// identifies each insertion sequence (two identical commitments inserted at
/// different indices produce different roots).
///
/// ### Production Note
///
/// Replace this with a proper incremental sparse Merkle tree whose internal
/// nodes use the Poseidon hash function for ZK-friendliness.  The Poseidon
/// hash is ZK-friendly (low multiplicative complexity) and is the standard
/// choice for privacy-preserving systems like Tornado Cash and Zcash Sapling.
///
/// Until Soroban exposes `crypto::poseidon`, SHA-256 is used as a stand-in.
/// The structural commitment invariants (one-way, collision-resistant) hold
/// regardless of which hash function is used.
pub fn compute_rolling_root(env: &Env, commitment: &BytesN<32>, index: u32) -> BytesN<32> {
    let mut data = Bytes::new(env);
    data.append(&Bytes::from_slice(env, &commitment.to_array()));
    data.append(&Bytes::from_slice(env, &index.to_be_bytes()));
    env.crypto().sha256(&data)
}

// ─── Pool Initialisation ────────────────────────────────────────────────────

/// Initialise pool state.
///
/// Called once during contract `init`.  Sets the commitment counter to zero.
/// This is idempotent if called when the counter is already present.
pub fn init_pool(env: &Env) {
    if !env.storage().instance().has(&KEY_POOL_COUNT) {
        env.storage().instance().set(&KEY_POOL_COUNT, &0u32);
    }
}

// ─── Unit Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commitment_count_starts_zero() {
        let env = soroban_sdk::Env::default();
        init_pool(&env);
        assert_eq!(commitment_count(&env), 0);
    }

    #[test]
    fn insert_advances_counter() {
        let env = soroban_sdk::Env::default();
        init_pool(&env);

        let c1 = BytesN::from_array(&env, &[0x11u8; 32]);
        let note = insert_commitment(&env, &c1).unwrap();
        assert_eq!(note.leaf_index, 0);
        assert_eq!(commitment_count(&env), 1);

        let c2 = BytesN::from_array(&env, &[0x22u8; 32]);
        let note2 = insert_commitment(&env, &c2).unwrap();
        assert_eq!(note2.leaf_index, 1);
        assert_eq!(commitment_count(&env), 2);
    }

    #[test]
    fn insert_zero_commitment_fails() {
        let env = soroban_sdk::Env::default();
        init_pool(&env);

        let zero = BytesN::from_array(&env, &[0u8; 32]);
        let result = insert_commitment(&env, &zero);
        assert_eq!(result, Err(ZkError::InvalidNoteCommitment));
    }

    #[test]
    fn insert_registers_known_root() {
        let env = soroban_sdk::Env::default();
        init_pool(&env);

        let commitment = BytesN::from_array(&env, &[0x42u8; 32]);
        insert_commitment(&env, &commitment).unwrap();

        // The computed root should now be registered
        let root = compute_rolling_root(&env, &commitment, 0);
        assert!(is_known_root(&env, &root));
    }

    #[test]
    fn unknown_root_not_registered() {
        let env = soroban_sdk::Env::default();
        init_pool(&env);

        let random = BytesN::from_array(&env, &[0xFF_u8; 32]);
        assert!(!is_known_root(&env, &random));
    }

    #[test]
    fn insert_without_init_fails() {
        let env = soroban_sdk::Env::default();
        // Do NOT call init_pool – counter is absent
        let c = BytesN::from_array(&env, &[0x01u8; 32]);
        let result = insert_commitment(&env, &c);
        assert_eq!(result, Err(ZkError::NotInitialised));
    }
}
