//! Binary Merkle commitment over a ledger's transaction results.
//!
//! ```text
//! leaf  = sha256(0x00 || tx_hash || result_code_be32)
//! node  = sha256(0x01 || left || right)
//! empty = [0; 32]                  (padding leaf)
//! ```
//!
//! The tree has `depth = ceil(log2(n))` levels (0 for a single leaf) and is
//! padded to `2^depth` leaves with `empty`. Domain separation prevents a node
//! from being passed off as a leaf, and binding the leaf index to the path
//! directions pins each proof to one position.
//!
//! Verification is the hot path. It copies the whole proof into linear
//! memory with one host call and reuses a single 65-byte stack buffer, so
//! each level costs exactly one `sha256` host call plus one buffer transfer
//! each way.

use alloc::vec::Vec;
use soroban_sdk::{Bytes, Env};

/// Supports up to 2^16 = 65 536 transactions per ledger.
pub const MAX_DEPTH: u32 = 16;

const LEAF_PREFIX: u8 = 0x00;
const NODE_PREFIX: u8 = 0x01;
pub const EMPTY: [u8; 32] = [0u8; 32];

fn hash_node(env: &Env, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 65];
    buf[0] = NODE_PREFIX;
    buf[1..33].copy_from_slice(left);
    buf[33..].copy_from_slice(right);
    env.crypto()
        .sha256(&Bytes::from_array(env, &buf))
        .to_array()
}

pub fn leaf_hash(env: &Env, tx_hash: &[u8; 32], result_code: i32) -> [u8; 32] {
    let mut buf = [0u8; 37];
    buf[0] = LEAF_PREFIX;
    buf[1..33].copy_from_slice(tx_hash);
    buf[33..].copy_from_slice(&result_code.to_be_bytes());
    env.crypto()
        .sha256(&Bytes::from_array(env, &buf))
        .to_array()
}

/// Number of levels needed for `leaf_count` leaves.
pub fn depth_for(leaf_count: u32) -> u32 {
    if leaf_count <= 1 {
        0
    } else {
        32 - (leaf_count - 1).leading_zeros()
    }
}

/// Root over `leaves`, padded with `EMPTY`. An empty set has root `EMPTY`.
pub fn root(env: &Env, mut level: Vec<[u8; 32]>) -> [u8; 32] {
    if level.is_empty() {
        return EMPTY;
    }
    let mut empty = EMPTY;
    for _ in 0..depth_for(level.len() as u32) {
        let pairs = level.len().div_ceil(2);
        for i in 0..pairs {
            let left = level[2 * i];
            let right = level.get(2 * i + 1).copied().unwrap_or(empty);
            level[i] = hash_node(env, &left, &right);
        }
        level.truncate(pairs);
        empty = hash_node(env, &empty, &empty);
    }
    level[0]
}

/// Recomputes the root from `leaf` at `index` using the concatenated
/// sibling hashes in `proof` (bottom-up, 32 bytes each).
///
/// The caller must have checked `proof.len() == 32 * depth` and
/// `depth <= MAX_DEPTH`.
pub fn compute_root(
    env: &Env,
    leaf: [u8; 32],
    mut index: u32,
    depth: u32,
    proof: &Bytes,
) -> [u8; 32] {
    let mut siblings = [0u8; (32 * MAX_DEPTH) as usize];
    let proof_len = (32 * depth) as usize;
    proof.copy_into_slice(&mut siblings[..proof_len]);

    let mut buf = [0u8; 65];
    buf[0] = NODE_PREFIX;
    let mut node = leaf;
    for sibling in siblings[..proof_len].chunks_exact(32) {
        if index & 1 == 0 {
            buf[1..33].copy_from_slice(&node);
            buf[33..].copy_from_slice(sibling);
        } else {
            buf[1..33].copy_from_slice(sibling);
            buf[33..].copy_from_slice(&node);
        }
        node = env
            .crypto()
            .sha256(&Bytes::from_array(env, &buf))
            .to_array();
        index >>= 1;
    }
    node
}

/// Builds the sibling path for `index`: the reference implementation for
/// off-chain provers and tests. Not exported as a contract function, so it
/// is stripped from the WASM.
pub fn proof(env: &Env, leaves: &[[u8; 32]], mut index: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    let mut empty = EMPTY;
    for _ in 0..depth_for(leaves.len() as u32) {
        let sibling = level.get(index ^ 1).copied().unwrap_or(empty);
        out.extend_from_slice(&sibling);
        let pairs = level.len().div_ceil(2);
        let next: Vec<[u8; 32]> = (0..pairs)
            .map(|i| {
                hash_node(
                    env,
                    &level[2 * i],
                    &level.get(2 * i + 1).copied().unwrap_or(empty),
                )
            })
            .collect();
        level = next;
        empty = hash_node(env, &empty, &empty);
        index >>= 1;
    }
    out
}
