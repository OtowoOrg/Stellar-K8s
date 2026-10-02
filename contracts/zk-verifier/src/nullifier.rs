//! # nullifier.rs
//!
//! Nullifier registry backed by Soroban **Persistent Storage**.
//!
//! ## Purpose
//!
//! A nullifier is a cryptographic one-time token derived inside the ZK circuit
//! from the note's secret key and its Merkle-tree leaf index:
//!
//! ```text
//! nullifier = PRF(note_secret_key, leaf_index)
//! ```
//!
//! Recording it prevents a spent note from being re-used (replay attacks).
//!
//! ## Storage model
//!
//! Each nullifier is stored as a `Persistent` entry:
//!
//! ```text
//! key:   (Symbol("nf"), BytesN<32>)   // compact ledger key
//! value: u64                           // ledger sequence at which it was spent
//! ```
//!
//! Persistent storage survives ledger expiry, meaning nullifiers survive
//! indefinitely – even through contract upgrades – as long as the TTL is
//! maintained via a TTL-bumper.
//!
//! ## Replay-attack prevention
//!
//! The `spend` function is the only way to mark a nullifier as used.  It is
//! called *after* a successful proof verification and *before* any transfer is
//! executed.  This ordering is enforced by the contract's state machine in
//! `lib.rs`.

use soroban_sdk::{symbol_short, BytesN, Env, Symbol};

use crate::errors::ZkError;

// ─── Storage Key ───────────────────────────────────────────────────────────

/// Namespace prefix for all nullifier entries.  Using a short symbol keeps
/// ledger keys compact, reducing storage fees.
const NULLIFIER_NS: Symbol = symbol_short!("nf");

/// Build the composite ledger key for a nullifier.
///
/// We store `(NULLIFIER_NS, hash)` as the key so the nullifier entries are
/// namespaced away from all other contract storage keys (e.g. admin, vk, root).
#[inline]
fn nullifier_key(hash: &BytesN<32>) -> (Symbol, BytesN<32>) {
    (NULLIFIER_NS, hash.clone())
}

// ─── Public API ────────────────────────────────────────────────────────────

/// Check whether `hash` has already been spent.
///
/// Returns `true` if the nullifier is present in Persistent storage.
pub fn is_spent(env: &Env, hash: &BytesN<32>) -> bool {
    env.storage()
        .persistent()
        .has(&nullifier_key(hash))
}

/// Mark `hash` as spent by writing the current ledger sequence number.
///
/// # Errors
///
/// * [`ZkError::NullifierIsZero`] – if `hash` is all-zero bytes (a sentinel
///   that the circuit should never produce).
/// * [`ZkError::NullifierAlreadySpent`] – if `hash` was already recorded.
///
/// # Panics
///
/// Does not panic.  All failure paths return `Err`.
pub fn spend(env: &Env, hash: &BytesN<32>) -> Result<(), ZkError> {
    // Guard: the zero nullifier is never valid
    let zero = BytesN::from_array(env, &[0u8; 32]);
    if hash == &zero {
        return Err(ZkError::NullifierIsZero);
    }

    let key = nullifier_key(hash);

    if env.storage().persistent().has(&key) {
        return Err(ZkError::NullifierAlreadySpent);
    }

    let ledger_seq: u32 = env.ledger().sequence();
    env.storage().persistent().set(&key, &ledger_seq);

    // Extend the TTL of this entry so it survives indefinitely.
    // We use the maximum TTL extension available in Soroban (roughly
    // 3 years of ledger history at 5 seconds / ledger).
    extend_nullifier_ttl(env, hash);

    Ok(())
}

/// Return the ledger sequence at which `hash` was spent, or `None` if not
/// spent.
pub fn spent_at(env: &Env, hash: &BytesN<32>) -> Option<u32> {
    env.storage()
        .persistent()
        .get::<(Symbol, BytesN<32>), u32>(&nullifier_key(hash))
}

/// Extend the TTL of an existing nullifier entry.
///
/// This should be called periodically by a TTL-bumper service so nullifiers
/// never expire.  Soroban Persistent entries have a minimum TTL but can expire
/// if nobody extends them – for a security-critical registry we must ensure
/// they live forever.
///
/// TTL extension parameters (Protocol 21):
/// * `threshold_ledgers` = 0   → extend unconditionally
/// * `extend_to_ledgers` = 18_460_800 ≈ 3 years at 5 s/ledger
pub fn extend_nullifier_ttl(env: &Env, hash: &BytesN<32>) {
    const TTL_EXTEND_THRESHOLD: u32 = 0;
    const TTL_EXTEND_TO: u32 = 18_460_800; // ~3 years

    let key = nullifier_key(hash);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_EXTEND_THRESHOLD, TTL_EXTEND_TO);
    }
}

/// Batch-extend TTLs for a list of nullifiers.
///
/// Useful for a maintenance transaction that bumps many nullifiers at once
/// rather than one per call.
pub fn batch_extend_ttl(env: &Env, hashes: &soroban_sdk::Vec<BytesN<32>>) {
    for hash in hashes.iter() {
        extend_nullifier_ttl(env, &hash);
    }
}
