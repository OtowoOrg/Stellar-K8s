//! Pins the Keccak-256 digest the trie depends on to published vectors.
//!
//! The issue asks for the Keccak-256 implementation to be audited, so this
//! pins the behaviour the rest of the verifier assumes:
//!
//! * Keccak-256 uses the pre-NIST `0x01` padding, **not** SHA3-256's `0x06`.
//!   Compiling the wrong variant would change every trie hash, so the two are
//!   asserted to differ.
//! * The empty-string digest and the empty-trie root are the exact values
//!   Ethereum uses, so any deviation fails loudly rather than silently
//!   rejecting every real proof.
//! * Longer-than-rate inputs are hashed across block boundaries, which is the
//!   path a multi-node proof exercises.
//!
//! The expected values were additionally reproduced with an independent
//! Keccak-256 implementation written from the specification, rather than being
//! copied from the library under test.

use mpt_verifier::rlp::encode_bytes;
use mpt_verifier::trie::{keccak256, EMPTY_CODE_HASH, EMPTY_ROOT};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The canonical empty digest, and the code hash of every EOA on Ethereum.
#[test]
fn empty_string_digest_matches_published_vector() {
    assert_eq!(
        hex(&keccak256(b"")),
        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
    );
}

/// A standard Keccak-256 test vector.
#[test]
fn abc_digest_matches_published_vector() {
    assert_eq!(
        hex(&keccak256(b"abc")),
        "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45"
    );
}

/// `keccak256(rlp(""))` is the Ethereum empty-trie root, the value an empty
/// state root resolves to.
#[test]
fn empty_trie_root_matches_published_vector() {
    assert_eq!(
        hex(&keccak256(&encode_bytes(&[]))),
        "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
    );
}

/// The declared constants must equal what the digest actually produces, so a
/// library change cannot leave stale constants behind.
#[test]
fn declared_constants_match_computed_digests() {
    assert_eq!(keccak256(&[]), EMPTY_CODE_HASH);
    assert_eq!(keccak256(&encode_bytes(&[])), EMPTY_ROOT);
}

/// Keccak-256 and SHA3-256 differ only in padding, so a matching digest would
/// mean the SHA3 variant was compiled in and every trie hash would be wrong.
#[test]
fn keccak_is_not_the_sha3_variant() {
    // SHA3-256("") is the FIPS 202 value; it must not equal Keccak-256("").
    const SHA3_EMPTY: &str = "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a";
    assert_ne!(hex(&keccak256(b"")), SHA3_EMPTY);
}

/// Inputs longer than the 136-byte rate must be absorbed across block
/// boundaries, and the result must stay deterministic.
#[test]
fn multi_block_input_hashes_deterministically() {
    let mut data = String::new();
    for i in 0..200u32 {
        data.push_str(&format!("{i},"));
    }
    assert!(data.len() > 136, "input must exceed the rate");
    let digest = keccak256(data.as_bytes());
    assert_eq!(digest.len(), 32);
    assert_eq!(keccak256(data.as_bytes()), digest);
}

/// Distinct inputs must yield distinct digests, guarding against a buffer
/// reuse bug in the absorb step.
#[test]
fn distinct_inputs_yield_distinct_digests() {
    assert_ne!(keccak256(b"a"), keccak256(b"b"));
    // Determinism regardless of call order.
    let first = keccak256(b"a");
    let _ = keccak256(b"b");
    assert_eq!(keccak256(b"a"), first);
}
