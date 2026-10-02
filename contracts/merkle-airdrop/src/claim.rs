//! Canonical Merkle-tree encoding and on-chain proof verification.
//!
//! This module is the *entire* trust surface of the distributor: it defines how a
//! distribution tree is built and how a claimant proves membership in it. It is
//! deliberately pure — it touches no storage and no token — so that it can be
//! unit-tested in isolation and reused verbatim by off-chain tooling.
//!
//! # Why a Merkle distributor at all
//!
//! Paying `N` recipients costs `N` ledger writes plus `N` token transfers, which is
//! why a token airdrop to 100 000+ accounts is economically impossible to run as a
//! loop. Instead the whole distribution is reduced off-chain to a single 32-byte
//! commitment (the Merkle root) that the contract stores once. Each claimant then
//! pays for exactly one proof verification and one transfer, and the contract's
//! storage stays `O(1)` in the number of recipients.
//!
//! # Canonical tree
//!
//! ```text
//! leaf(i)    = SHA256( 0x00 ‖ i_be32 ‖ amount_be128 ‖ account_xdr )
//! node(a, b) = SHA256( 0x01 ‖ min(a, b) ‖ max(a, b) )
//! root       = the single digest remaining after folding adjacent pairs
//! ```
//!
//! Every byte is fixed by this module so that an off-chain generator and the
//! on-chain verifier cannot drift apart:
//!
//! * **Domain separation (`0x00` / `0x01`).** A leaf and an internal node are hashed
//!   from different tag bytes, so an internal node can never be presented as a leaf
//!   (nor vice versa). Without the tags the tree is vulnerable to the classic
//!   second-preimage attack, in which a forged proof proves membership of an
//!   *intermediate* node and thereby mints value that nobody was allocated.
//! * **Sorted child pairs.** Folding `min ‖ max` removes the need for a
//!   direction bitmap alongside the proof. The proof is therefore a plain
//!   `Vec<BytesN<32>>`, which is fewer bytes to decode out of calldata and one fewer
//!   branch per level than a direction-tagged encoding — and, more importantly, it
//!   removes an entire class of client bugs where the direction bits disagree with
//!   the sibling ordering.
//! * **The index is hashed into the leaf.** The claim flag is keyed by leaf index,
//!   so the index must be authenticated by the proof. Hashing it means a claimant
//!   cannot re-present the same allocation under a different index to find an unset
//!   flag — the recomputed root would no longer match.
//! * **The amount is hashed into the leaf.** The contract pays exactly the amount the
//!   tree committed to, so the claimant can neither inflate nor round their payout.
//! * **Zero padding to a power of two.** Allocations are padded with the all-zero
//!   digest (`PAD_LEAF`) until the leaf count is a power of two, which keeps the
//!   tree perfectly balanced so that every proof has the same length
//!   (`log2(padded_count)`) and every claim has the same cost. A padded slot cannot
//!   be claimed: it is not the image of any `SHA256` output under this encoding, so
//!   no `(index, amount, account)` triple can reproduce it.
//!
//! # Instruction-cost notes
//!
//! Verification is `1 + depth` SHA-256 invocations and nothing else: the loop is a
//! flat iteration over the proof with no recursion, no hashing of user-controlled
//! byte lengths beyond the fixed 65-byte preimage, and no storage access. Proof depth
//! is capped at [`MAX_PROOF_DEPTH`], which both bounds the loop and documents the
//! largest distribution the contract supports. `benches/claim_bench.rs` measures the
//! real instruction and fee cost at every depth up to that cap.

use soroban_sdk::{xdr::ToXdr, Address, Bytes, BytesN, Env, Vec};

/// Domain-separation tag prefixed to every **leaf** preimage.
pub const LEAF_TAG: u8 = 0x00;

/// Domain-separation tag prefixed to every **internal node** preimage.
pub const NODE_TAG: u8 = 0x01;

/// Width of a Merkle digest in bytes (SHA-256).
pub const DIGEST_LEN: usize = 32;

/// A Merkle digest, as a plain byte array.
///
/// The contract carries digests around as `BytesN<DIGEST_LEN>` (the storage and
/// calldata representation); this is the same value on the stack, which is what the
/// encoders and off-chain tooling work with.
pub type Digest = [u8; DIGEST_LEN];

/// Deepest proof the verifier will evaluate.
///
/// `2^20 = 1_048_576` padded leaves, i.e. a distribution of just over one million
/// recipients. Capping the depth keeps the worst-case cost of a single claim bounded
/// and known, which is what lets the airdrop be priced before it is announced.
pub const MAX_PROOF_DEPTH: u32 = 20;

/// Largest padded leaf count a distribution may declare.
pub const MAX_LEAF_COUNT: u32 = 1 << MAX_PROOF_DEPTH;

/// Bits packed into one claim-flag storage bucket.
///
/// 128 allocations share one ledger entry, so a 1 048 576-recipient distribution
/// needs at most 8 192 claim entries instead of one per recipient.
pub const BUCKET_BITS: u32 = 128;

/// Fixed-width part of a leaf preimage: `0x00 ‖ index_be32 ‖ amount_be128`.
pub const LEAF_HEADER_LEN: usize = 1 + 4 + 16;

/// Widest serialized account: an `ScVal::Address` carrying an account address
/// (`ScVal` tag 4 ‖ `ScAddress::Account` 4 ‖ `PublicKey::Ed25519` 4 ‖ 32-byte key).
///
/// A contract address serializes to 40 bytes, which is why the encoding is
/// length-implied rather than fixed — see [`leaf_preimage`].
pub const ACCOUNT_XDR_MAX_LEN: usize = 4 + 4 + 4 + DIGEST_LEN;

/// Widest possible leaf preimage.
pub const LEAF_MAX_LEN: usize = LEAF_HEADER_LEN + ACCOUNT_XDR_MAX_LEN;

/// Internal-node preimage length: `0x01 ‖ child ‖ child`.
pub const NODE_LEN: usize = 1 + 2 * DIGEST_LEN;

/// The digest used to pad a tree up to a power of two.
pub const PAD_LEAF: Digest = [0u8; DIGEST_LEN];

// The documented address encoding widths are load-bearing: off-chain generators
// reproduce them byte for byte. Fail the build rather than a claim if they drift.
const _: () = assert!(DIGEST_LEN == 32);
const _: () = assert!(ACCOUNT_XDR_MAX_LEN == 44);
const _: () = assert!(LEAF_HEADER_LEN == 21);
const _: () = assert!(LEAF_MAX_LEN == 65);
const _: () = assert!(NODE_LEN == 65);
const _: () = assert!(MAX_LEAF_COUNT == 1_048_576);

// ---------------------------------------------------------------------------
// Canonical encodings
// ---------------------------------------------------------------------------

/// Build the canonical leaf preimage for `(index, amount, account)`.
///
/// Returns the filled buffer together with the number of valid bytes in it.
///
/// `account_xdr` is the serialized address as produced by [`Address::to_xdr`]. The
/// address is the **final** field and its width is implied by its XDR type tag
/// (44 bytes for an account, 40 for a contract), so the encoding stays unambiguous:
/// the leading fields are fixed width, hence two distinct `(index, amount, account)`
/// triples can never serialize to the same byte string, and the XDR type tag means an
/// account address and a contract address carrying identical 32-byte payloads still
/// hash differently.
///
/// All lengths are big-endian so that the encoding is language- and platform-neutral:
/// it can be reproduced by a JavaScript or Go generator byte for byte.
///
/// # Panics
///
/// If `account_xdr` is wider than [`ACCOUNT_XDR_MAX_LEN`]. This is unreachable for any
/// value produced by [`Address::to_xdr`] on the current protocol; a panic (and thus a
/// reverted transaction) is the correct fail-closed behaviour if a future address type
/// ever exceeds it.
#[inline]
pub fn leaf_preimage(index: u32, amount: i128, account_xdr: &[u8]) -> ([u8; LEAF_MAX_LEN], usize) {
    assert!(
        account_xdr.len() <= ACCOUNT_XDR_MAX_LEN,
        "address XDR wider than the canonical maximum"
    );

    let mut buf = [0u8; LEAF_MAX_LEN];
    buf[0] = LEAF_TAG;
    buf[1..5].copy_from_slice(&index.to_be_bytes());
    buf[5..LEAF_HEADER_LEN].copy_from_slice(&amount.to_be_bytes());

    let end = LEAF_HEADER_LEN + account_xdr.len();
    buf[LEAF_HEADER_LEN..end].copy_from_slice(account_xdr);

    (buf, end)
}

/// Build the canonical internal-node preimage for the children `a` and `b`.
///
/// Children are folded in ascending byte order (`min ‖ max`), which is what makes the
/// proof direction-free.
#[inline]
pub fn node_preimage(a: &Digest, b: &Digest) -> [u8; NODE_LEN] {
    let mut buf = [0u8; NODE_LEN];
    buf[0] = NODE_TAG;

    let (lo, hi) = if a.as_slice() <= b.as_slice() {
        (a, b)
    } else {
        (b, a)
    };
    buf[1..1 + DIGEST_LEN].copy_from_slice(lo);
    buf[1 + DIGEST_LEN..NODE_LEN].copy_from_slice(hi);

    buf
}

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

/// Hash a preimage with the host's SHA-256 implementation.
///
/// The SDK hands the host a single `Bytes` value per call. Building the preimage in a
/// stack buffer and copying it across once means exactly one host call per level on
/// top of the hash itself, instead of the two or three an append-based construction
/// would need.
#[inline]
fn hash_preimage(env: &Env, preimage: &[u8]) -> BytesN<DIGEST_LEN> {
    let bytes = Bytes::from_slice(env, preimage);
    env.crypto().sha256(&bytes).to_bytes()
}

/// Hash a leaf for `(index, amount, account)`.
#[inline]
pub fn hash_leaf(env: &Env, index: u32, amount: i128, account: &Address) -> BytesN<DIGEST_LEN> {
    let account_xdr = account.to_xdr(env);
    let mut account_buf = [0u8; ACCOUNT_XDR_MAX_LEN];
    let account_len = account_xdr.len() as usize;
    account_xdr.copy_into_slice(&mut account_buf[..account_len]);

    let (preimage, len) = leaf_preimage(index, amount, &account_buf[..account_len]);
    hash_preimage(env, &preimage[..len])
}

/// Fold two digests into their parent.
#[inline]
pub fn hash_node(env: &Env, a: &Digest, b: &Digest) -> BytesN<DIGEST_LEN> {
    hash_preimage(env, &node_preimage(a, b))
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// Recompute the root implied by `leaf` and `proof`, and report whether it equals
/// `expected_root`.
///
/// No early exit is possible or desirable: the root is only known once the whole path
/// has been folded, so the cost is exactly `proof.len()` hashes and is independent of
/// where — or whether — the proof eventually fails. That gives a claim a worst case
/// that can be computed from the calldata size alone.
pub fn verify_proof(
    env: &Env,
    leaf: &BytesN<DIGEST_LEN>,
    proof: &Vec<BytesN<DIGEST_LEN>>,
    expected_root: &BytesN<DIGEST_LEN>,
) -> bool {
    let mut node = leaf.to_array();

    for sibling in proof.iter() {
        node = hash_node(env, &node, &sibling.to_array()).to_array();
    }

    node.as_slice() == expected_root.to_array().as_slice()
}

// ---------------------------------------------------------------------------
// Claim-flag bitmap helpers
// ---------------------------------------------------------------------------

/// Index of the storage bucket holding the claim flag for `leaf_index`.
#[inline]
pub const fn bucket_of(leaf_index: u32) -> u32 {
    leaf_index / BUCKET_BITS
}

/// Single-bit mask of `leaf_index` inside its bucket.
#[inline]
pub const fn mask_of(leaf_index: u32) -> u128 {
    1u128 << (leaf_index % BUCKET_BITS)
}

/// Smallest power of two that can hold `leaf_count` allocations.
///
/// This is the number of leaves the tree is actually built over (allocations plus
/// padding), and therefore `log2` of it is the length of every proof.
pub const fn padded_leaf_count(leaf_count: u32) -> u32 {
    let mut n = 1u32;
    while n < leaf_count {
        n <<= 1;
    }
    n
}

/// Length of every proof in a tree holding `leaf_count` allocations.
///
/// Exposed to clients so a UI can pre-flight a claim (and size the calldata) without
/// having to reimplement the tree geometry.
pub const fn proof_depth_for(leaf_count: u32) -> u32 {
    let padded = padded_leaf_count(leaf_count);
    let mut depth = 0u32;
    let mut n = padded;
    while n > 1 {
        n >>= 1;
        depth += 1;
    }
    depth
}

// ---------------------------------------------------------------------------
// Unit tests for the pure encoding
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn leaf_and_node_preimages_are_tag_domain_separated() {
        let (leaf, len) = leaf_preimage(1, 2, &[]);
        assert_eq!(len, LEAF_HEADER_LEN);
        assert_eq!(leaf[0], LEAF_TAG);

        let node = node_preimage(&[7u8; DIGEST_LEN], &[9u8; DIGEST_LEN]);
        assert_eq!(node.len(), NODE_LEN);
        assert_eq!(node[0], NODE_TAG);

        // A leaf preimage and a node preimage can never be equal even when their
        // payload bytes coincide, because the tag differs.
        assert_ne!(leaf[0], node[0]);
    }

    #[test]
    fn leaf_preimage_encodes_index_and_amount_big_endian() {
        let (buf, len) = leaf_preimage(
            0x0102_0304,
            0x0506_0708_090a_0b0c_0d0e_0f10_1112_1314i128,
            &[],
        );
        assert_eq!(len, LEAF_HEADER_LEN);
        assert_eq!(&buf[1..5], &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(buf[5], 0x05);
        assert_eq!(buf[20], 0x14);
    }

    #[test]
    fn node_preimage_is_order_independent() {
        let a = [1u8; DIGEST_LEN];
        let b = [2u8; DIGEST_LEN];
        assert_eq!(node_preimage(&a, &b), node_preimage(&b, &a));
    }

    #[test]
    fn node_preimage_orders_children_ascending() {
        let lo = [1u8; DIGEST_LEN];
        let hi = [2u8; DIGEST_LEN];
        let buf = node_preimage(&hi, &lo);
        assert_eq!(&buf[1..1 + DIGEST_LEN], lo.as_slice());
        assert_eq!(&buf[1 + DIGEST_LEN..], hi.as_slice());
    }

    #[test]
    fn address_xdr_widths_match_the_documented_format() {
        let env = Env::default();
        // `Address::generate` produces contract addresses in tests, which serialize
        // to 40 bytes: the tag + `ScAddress::Contract` + a 32-byte hash.
        let contract_like = Address::generate(&env);
        assert_eq!(
            contract_like.to_xdr(&env).len() as usize,
            ACCOUNT_XDR_MAX_LEN - 4
        );
    }

    #[test]
    fn padded_leaf_count_rounds_up_to_a_power_of_two() {
        assert_eq!(padded_leaf_count(1), 1);
        assert_eq!(padded_leaf_count(2), 2);
        assert_eq!(padded_leaf_count(3), 4);
        assert_eq!(padded_leaf_count(100_000), 131_072);
        assert_eq!(padded_leaf_count(MAX_LEAF_COUNT), MAX_LEAF_COUNT);
        // Already a power of two: unchanged.
        assert_eq!(padded_leaf_count(1 << 17), 1 << 17);
    }

    #[test]
    fn proof_depth_matches_the_tree_geometry() {
        assert_eq!(proof_depth_for(1), 0);
        assert_eq!(proof_depth_for(2), 1);
        assert_eq!(proof_depth_for(3), 2);
        // 100 000 recipients pad to 131 072 leaves -> 17 siblings.
        assert_eq!(proof_depth_for(100_000), 17);
        assert_eq!(proof_depth_for(MAX_LEAF_COUNT), MAX_PROOF_DEPTH);
    }

    #[test]
    fn bucket_and_mask_partition_the_index_space() {
        // Consecutive indices share a bucket and use distinct bits.
        assert_eq!(bucket_of(0), 0);
        assert_eq!(bucket_of(127), 0);
        assert_eq!(bucket_of(128), 1);
        assert_eq!(mask_of(0), 1);
        assert_eq!(mask_of(1), 2);
        assert_eq!(mask_of(127), 1u128 << 127);

        // No two indices can share a flag slot.
        for index in [0u32, 1, 63, 64, 127, 128, 129, 1_000_000] {
            let bit = mask_of(index);
            assert_ne!(bit, 0, "mask must be non-zero for index {index}");
            assert_eq!(bit.count_ones(), 1, "mask must be a single bit");
        }
    }
}
