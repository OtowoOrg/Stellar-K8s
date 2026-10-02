//! Off-chain Merkle-tree generator.
//!
//! This is the other half of the distributor: the code a campaign actually runs to
//! turn a CSV of `(address, allocation)` into the single root that gets committed
//! on-chain, plus the per-recipient proofs that get handed out.
//!
//! It is intentionally **not** part of the contract. It needs `std` and a native
//! SHA-256, and pulling it into `lib.rs` would drag both into the Wasm guest for no
//! reason. Instead it is included by path — with `use merkle_airdrop::claim::…`, so it
//! shares the *same* encoder the contract verifies against — by:
//!
//! * `tests/merkle_airdrop.rs` (`#[path = "../src/offchain.rs"]`)
//! * `benches/claim_bench.rs`
//! * `examples/generate_tree.rs`
//!
//! Because the preimages come from [`merkle_airdrop::claim`] rather than from a second
//! hand-written implementation, the tree this builds cannot drift from the tree the
//! contract accepts. The integration test asserts that equivalence explicitly on the
//! host's own hash implementation.

// Each consumer (integration test, benchmark, example) uses a different subset of this
// module, so per-target dead-code analysis would be noise rather than signal.
#![allow(dead_code)]

use merkle_airdrop::claim::{leaf_preimage, node_preimage, Digest, PAD_LEAF};
use sha2::{Digest as _, Sha256};
use std::vec::Vec;

/// A complete balanced Merkle tree plus every intermediate level.
///
/// Keeping the levels makes proof extraction O(depth) with no re-hashing, which is
/// what lets the 100 000-recipient test and the 20-level benchmark stay fast.
pub struct Tree {
    /// Padded leaves in index order (real allocations first, then [`PAD_LEAF`]).
    pub leaves: Vec<Digest>,
    /// `levels[0]` is [`Tree::leaves`]; each level is the folding of the one below it;
    /// the last level is the single root.
    pub levels: Vec<Vec<Digest>>,
}

impl Tree {
    /// Root commitment of the distribution.
    pub fn root(&self) -> Digest {
        self.levels[self.levels.len() - 1][0]
    }

    /// Number of siblings in every proof, i.e. `log2(padded leaf count)`.
    pub fn depth(&self) -> u32 {
        (self.levels.len() - 1) as u32
    }

    /// Extract the proof for the allocation at `index`.
    pub fn proof(&self, index: usize) -> Vec<Digest> {
        let mut proof = Vec::with_capacity(self.depth() as usize);
        let mut idx = index;
        for level in &self.levels[..self.levels.len() - 1] {
            // Sibling of `idx`; the verifier re-sorts the pair, so raw order is fine.
            proof.push(level[idx ^ 1]);
            idx >>= 1;
        }
        proof
    }
}

/// SHA-256 over an arbitrary preimage, using the native implementation.
pub fn sha256(preimage: &[u8]) -> Digest {
    Sha256::digest(preimage).into()
}

/// Leaf digest for one allocation — the off-chain twin of
/// [`merkle_airdrop::claim::hash_leaf`].
pub fn leaf_digest(index: u32, amount: i128, account_xdr: &[u8]) -> Digest {
    let (preimage, len) = leaf_preimage(index, amount, account_xdr);
    sha256(&preimage[..len])
}

/// Parent digest for two children — the off-chain twin of
/// [`merkle_airdrop::claim::hash_node`].
pub fn parent_digest(a: &Digest, b: &Digest) -> Digest {
    sha256(&node_preimage(a, b))
}

/// Build the tree for `accounts` / `amounts`, padding with [`PAD_LEAF`] up to a power
/// of two.
///
/// `accounts[i]` is the serialized address of recipient `i` (`Address::to_xdr`), so
/// callers control the exact bytes that go into the commitment.
pub fn build_tree(accounts: &[Vec<u8>], amounts: &[i128]) -> Tree {
    assert_eq!(
        accounts.len(),
        amounts.len(),
        "every recipient needs exactly one allocation"
    );
    assert!(
        !accounts.is_empty(),
        "a distribution needs at least one recipient"
    );

    let padded = padded_count(accounts.len() as u32) as usize;

    let mut leaves: Vec<Digest> = Vec::with_capacity(padded);
    for (i, (account, amount)) in accounts.iter().zip(amounts.iter()).enumerate() {
        leaves.push(leaf_digest(i as u32, *amount, account));
    }
    // Padding is deterministic: a padded slot holds the all-zero digest, which is not
    // the image of any SHA-256 output under this encoding and so can never be claimed.
    leaves.resize(padded, PAD_LEAF);

    let mut levels = vec![leaves.clone()];
    while levels[levels.len() - 1].len() > 1 {
        let current = &levels[levels.len() - 1];
        let mut next = Vec::with_capacity(current.len() / 2);
        for pair in current.as_chunks::<2>().0 {
            next.push(parent_digest(&pair[0], &pair[1]));
        }
        levels.push(next);
    }

    Tree { leaves, levels }
}

/// Round `n` up to a power of two (minimum 1).
pub fn padded_count(n: u32) -> u32 {
    let mut count = 1u32;
    while count < n {
        count <<= 1;
    }
    count
}

/// Serialize a 32-byte Ed25519 key as an account address, exactly as
/// `ScVal::Address` / `ScAddress::Account` / `PublicKey::Ed25519` does on the wire.
///
/// 44 bytes: the `ScVal` union tag, the `ScAddress::Account` variant, the `PublicKey`
/// variant, then the key.
pub fn account_xdr(key: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(&[0, 0, 0, 18]); // ScVal::Address
    out.extend_from_slice(&[0, 0, 0, 0]); // ScAddress::Account
    out.extend_from_slice(&[0, 0, 0, 0]); // PublicKey::PublicKeyTypeEd25519
    out.extend_from_slice(key);
    out
}

/// Serialize a 32-byte contract-id hash as a contract address (40 bytes: the `ScVal`
/// union tag, then `ScAddress::Contract`).
///
/// Both address kinds are valid recipients, and they hash to different leaves even when
/// their 32-byte payloads match — which is why the contract commits to the whole
/// serialized address rather than to the payload alone.
pub fn contract_xdr(hash: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(40);
    out.extend_from_slice(&[0, 0, 0, 18]); // ScVal::Address
    out.extend_from_slice(&[0, 0, 0, 1]); // ScAddress::Contract
    out.extend_from_slice(hash);
    out
}

/// Deterministic 32-byte recipient payload for `index`.
///
/// The bytes are spread across the whole key so that no two recipients and no two
/// subtrees share structure — a lazily-filled key would make the tree degenerate and
/// understate the real hashing cost. Derived purely arithmetically so the same recipient
/// has the same bytes in every process, Env and run.
pub fn recipient_payload(index: u32, salt: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    for (i, chunk) in key.chunks_mut(4).enumerate() {
        let mixed = index
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add((i as u32).wrapping_mul(0x85EB_CA6B))
            .wrapping_add(salt.wrapping_mul(0xC2B2_AE35));
        chunk.copy_from_slice(&mixed.to_be_bytes());
    }
    key
}

/// Serialized contract address for the synthetic recipient at `index`.
///
/// This is the encoding that goes into the tree: it needs no `Env`, which is what allows
/// a distribution to be hashed once and then claimed in as many fresh environments as a
/// benchmark needs.
pub fn recipient_xdr(index: u32, salt: u32) -> Vec<u8> {
    contract_xdr(&recipient_payload(index, salt))
}

/// Serialized **account** address for the synthetic recipient at `index`.
///
/// Used to exercise the 44-byte tail of the leaf encoding, which is the width a real
/// `G...` recipient produces.
pub fn synthetic_account(index: u32, salt: u32) -> Vec<u8> {
    account_xdr(&recipient_payload(index, salt))
}

/// Render a digest as lowercase hex (for logs, fixtures and the example output).
pub fn to_hex(digest: &Digest) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Decode a 64-character lowercase hex digest.
pub fn from_hex(hex: &str) -> Digest {
    assert_eq!(hex.len(), 64, "a digest is 64 hex characters");
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("valid hex");
    }
    out
}
