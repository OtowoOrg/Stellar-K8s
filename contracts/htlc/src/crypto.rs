//! Gas-efficient SHA-256 pre-image validation for the HTLC contract.
//!
//! The Soroban host exposes SHA-256 as a **single host function call**,
//! making this the most cost-effective hash primitive available.  The
//! comparison is a constant-time byte-level check performed by the host
//! via `BytesN::eq`, ruling out timing-oracle side channels.
//!
//! # Hash-Collision Resistance
//! SHA-256 has a 2^256 output space.  Birthday-bound collision probability
//! for an individual swap is ≈ 2^(−128), which is computationally
//! infeasible.  Pre-image attacks on a known hash require ≈ 2^256 work.
//! Second-preimage attacks are equally infeasible (≈ 2^256).
//! These guarantees hold in the WASM execution environment because:
//!   - The host function delegates to a production SHA-256 implementation.
//!   - The contract stores the full 32-byte digest; truncated comparisons
//!     are never used.
//!   - The caller must supply the raw pre-image; the contract itself never
//!     reveals it outside of the `claim` function's on-chain transaction.

use soroban_sdk::{Bytes, BytesN, Env};

/// Returns `true` when `sha256(preimage) == expected_hash`.
///
/// The check uses the Soroban host's native `sha256` call (a single host
/// function invocation) and compares the 32-byte digest with the stored
/// `expected_hash`.  The comparison is performed inside the WASM runtime
/// via `BytesN<32>::eq`, which is a constant-number-of-bytes comparison —
/// there is no early-exit branch on individual bytes.
///
/// # Gas cost
/// - 1 host-function call for `env.crypto().sha256(preimage)` whose fee
///   scales linearly with the byte-length of `preimage`.
/// - 1 host-function call for the `BytesN<32>` equality check.
/// - Total: O(|preimage|) — well within the fee budget for typical 32-byte
///   pre-images used in atomic swaps.
#[inline]
pub fn verify_preimage(env: &Env, preimage: &Bytes, expected_hash: &BytesN<32>) -> bool {
    let digest: BytesN<32> = env.crypto().sha256(preimage);
    &digest == expected_hash
}

/// Computes the SHA-256 digest of a raw byte slice and returns it as a
/// `BytesN<32>`.  This is the canonical way for the initiator to derive the
/// `hashlock` parameter that is stored in the HTLC state.
///
/// # Usage (off-chain tooling / tests)
/// ```ignore
/// let preimage = Bytes::from_slice(&env, b"super-secret-32-byte-preimage!!!");
/// let hashlock = sha256_of(&env, &preimage);
/// // pass `hashlock` to `HtlcContract::lock(…)`
/// ```
#[inline]
pub fn sha256_of(env: &Env, data: &Bytes) -> BytesN<32> {
    env.crypto().sha256(data)
}
