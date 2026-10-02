//! Validation against a real Ethereum mainnet state proof.
//!
//! The fixture in `tests/fixtures/mainnet_weth_state_proof.json` was captured
//! from a public Ethereum mainnet node using `eth_getProof` and
//! `eth_getBlockByNumber`. It records the block hash, state root, the account
//! proof and one storage-slot proof for the WETH contract.
//!
//! These tests deliberately exercise the verifier against bytes Ethereum
//! actually produced, not only against proofs this test suite built itself.
//! The fixture's `_comment` field documents how to re-capture it.

use std::vec::Vec;

use mpt_verifier::account::{decode_account, decode_storage_value, parse_address};
use mpt_verifier::trie::{keccak256, verify_proof, Limits, TrieError, TrieOutcome};

const FIXTURE: &str = include_str!("fixtures/mainnet_weth_state_proof.json");

fn hex_decode(s: &str) -> Vec<u8> {
    let s = s.trim().trim_matches('"');
    let s = s.strip_prefix("0x").unwrap_or(s);
    // Ethereum renders scalars without a leading zero, so a single-nibble
    // value such as "0x1" is valid JSON but odd-length hex. Left-pad it: the
    // numeric value is what matters, not the display width.
    let padded;
    let s = if s.len() % 2 == 1 {
        padded = format!("0{s}");
        padded.as_str()
    } else {
        s
    };
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

/// Reads `"name": "value"` from the fixture.
fn hex_field(name: &str) -> Vec<u8> {
    let needle = format!("\"{name}\": \"");
    let start = FIXTURE
        .find(&needle)
        .unwrap_or_else(|| panic!("fixture missing field {name}"))
        + needle.len();
    let end = FIXTURE[start..]
        .find('"')
        .expect("unterminated fixture string")
        + start;
    hex_decode(&FIXTURE[start..end])
}

/// Reads `"name": ["0x..", "0x.."]` from the fixture.
fn hex_array_field(name: &str) -> Vec<Vec<u8>> {
    let needle = format!("\"{name}\": [");
    let start = FIXTURE
        .find(&needle)
        .unwrap_or_else(|| panic!("fixture missing array {name}"))
        + needle.len();
    let end = FIXTURE[start..]
        .find(']')
        .expect("unterminated fixture array")
        + start;
    FIXTURE[start..end].split(',').map(hex_decode).collect()
}

fn hex32(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(bytes);
    out
}

fn limits() -> Limits {
    Limits::default()
}

fn state_root() -> [u8; 32] {
    hex32(&hex_field("state_root"))
}

fn weth_address() -> [u8; 20] {
    parse_address(&hex_field("address")).expect("valid address")
}

fn account_proof() -> Vec<Vec<u8>> {
    hex_array_field("account_proof")
}

fn storage_proof() -> Vec<Vec<u8>> {
    hex_array_field("storage_proof")
}

/// The account proof must resolve to WETH's account leaf under the real state
/// root, and the decoded fields must match what the node reported.
#[test]
fn real_mainnet_account_proof_verifies() {
    let key = keccak256(&weth_address());
    let proof = account_proof();
    assert_eq!(proof.len(), 9, "fixture should carry the 9 proof nodes");

    let outcome = verify_proof(&proof, &state_root(), &key, &limits())
        .expect("mainnet account proof verifies");
    let value = match outcome {
        TrieOutcome::Present(v) => v,
        TrieOutcome::Absent => panic!("WETH account must be present at the state root"),
    };

    let account = decode_account(&value, &limits().rlp()).expect("account value decodes");

    let mut expected_nonce = 0u64;
    for b in hex_field("account_nonce") {
        expected_nonce = (expected_nonce << 8) | b as u64;
    }
    assert_eq!(account.nonce, expected_nonce, "nonce mismatch");
    assert_eq!(
        hex_field("account_balance"),
        account.balance,
        "balance mismatch"
    );
    assert_eq!(
        hex32(&hex_field("account_code_hash")),
        account.code_hash,
        "code hash mismatch"
    );
    assert!(!account.is_eoa(), "WETH is a contract");
}

/// The storage-slot proof must verify against the storage root that the account
/// leaf itself commits to, which is what binds the two proofs together.
#[test]
fn real_mainnet_storage_proof_verifies_against_account_storage_root() {
    let account_key = keccak256(&weth_address());
    let outcome = verify_proof(&account_proof(), &state_root(), &account_key, &limits())
        .expect("account proof verifies");
    let account_value = match outcome {
        TrieOutcome::Present(v) => v,
        TrieOutcome::Absent => panic!("account must be present"),
    };
    let account = decode_account(&account_value, &limits().rlp()).expect("account decodes");

    // Storage tries are keyed by the Keccak-256 of the mapping slot key. The
    // fixture records that slot key -- `keccak256(abi.encode(address, slot))`
    // as produced for a balanceOf mapping -- and the trie path is the hash of
    // it, so it is hashed here rather than used verbatim.
    let slot_key = keccak256(&hex_field("storage_slot_key"));
    let proof = storage_proof();
    assert_eq!(proof.len(), 9, "fixture should carry 9 storage nodes");

    let outcome = verify_proof(&proof, &account.storage_root, &slot_key, &limits())
        .expect("storage proof verifies against the account storage root");
    let value = match outcome {
        TrieOutcome::Present(v) => v,
        TrieOutcome::Absent => panic!("holder balance slot must be present"),
    };

    let decoded = decode_storage_value(&value, &limits().rlp()).expect("storage value decodes");

    let mut expected = hex_field("storage_value");
    while expected.len() > 1 && expected[0] == 0 {
        expected.remove(0);
    }
    assert_eq!(decoded, expected, "storage value mismatch");
    assert!(!decoded.is_empty(), "holder balance should be non-empty");
}

/// Every node in the proof is bound to the trie by its Keccak-256 hash, so a
/// single flipped bit anywhere must invalidate verification.
#[test]
fn tampering_with_any_proof_node_breaks_verification() {
    let key = keccak256(&weth_address());
    let proof = account_proof();
    assert!(verify_proof(&proof, &state_root(), &key, &limits()).is_ok());

    for (i, node) in proof.iter().enumerate() {
        if node.is_empty() {
            continue;
        }
        for byte_index in [0usize, node.len() / 2, node.len() - 1] {
            let mut tampered: Vec<Vec<u8>> = proof.clone();
            tampered[i][byte_index] ^= 0x01;
            assert!(
                verify_proof(&tampered, &state_root(), &key, &limits()).is_err(),
                "tampering node {i} byte {byte_index} should invalidate the proof"
            );
        }
    }
}

/// The state root is the caller's commitment, not the prover's, so a proof
/// that is internally consistent but offered against a different root fails.
#[test]
fn proof_against_wrong_state_root_is_rejected() {
    let key = keccak256(&weth_address());
    let mut wrong_root = state_root();
    wrong_root[31] ^= 0xff;
    assert_eq!(
        verify_proof(&account_proof(), &wrong_root, &key, &limits()),
        Err(TrieError::RootNotFound)
    );
}

/// Asking for a different address against the same proof must never return
/// WETH's account data.
#[test]
fn proof_for_another_address_does_not_leak_account_data() {
    let mut other = weth_address();
    other[19] ^= 0x01;
    let other_key = keccak256(&other);

    match verify_proof(&account_proof(), &state_root(), &other_key, &limits()) {
        Err(TrieError::KeyMismatch) | Err(TrieError::ChildNotFound) => {}
        Err(other_err) => panic!("unexpected error: {other_err:?}"),
        Ok(TrieOutcome::Present(_)) => {
            panic!("must not return WETH's account for a different address")
        }
        Ok(TrieOutcome::Absent) => {}
    }
}

/// A proof truncated by dropping its root node must be refused rather than
/// silently treated as a valid (empty) proof.
#[test]
fn truncated_proof_is_rejected() {
    let key = keccak256(&weth_address());
    let mut proof = account_proof();
    proof.remove(0);
    assert!(verify_proof(&proof, &state_root(), &key, &limits()).is_err());
}
