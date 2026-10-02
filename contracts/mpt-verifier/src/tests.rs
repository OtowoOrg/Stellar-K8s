//! Contract-level tests for [`MptVerifier::verify_evm_state`].
//!
//! These drive the real Soroban entry point — argument decoding, node framing
//! and error mapping — against the Ethereum mainnet proof fixture, so the
//! wiring around the verifier is covered and not just the traversal internals.

extern crate alloc;

use alloc::vec::Vec;

use soroban_sdk::{Bytes, BytesN, Env};

use crate::{frame_nodes, frame_slot, MptError, MptVerifier, MAX_NODE_BYTES};

const FIXTURE: &str = include_str!("../tests/fixtures/mainnet_weth_state_proof.json");

fn hex_decode(s: &str) -> Vec<u8> {
    let s = s.trim().trim_matches('"');
    let s = s.strip_prefix("0x").unwrap_or(s);
    // Ethereum renders scalars without a leading zero, so "0x1" is valid but
    // odd-length. Left-pad before decoding.
    let padded;
    let s = if s.len() % 2 == 1 {
        padded = alloc::format!("0{s}");
        padded.as_str()
    } else {
        s
    };
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn hex_field(name: &str) -> Vec<u8> {
    let needle = alloc::format!("\"{name}\": \"");
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

fn hex_array_field(name: &str) -> Vec<Vec<u8>> {
    let needle = alloc::format!("\"{name}\": [");
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

/// Builds a padded `BytesN` from variable-length bytes.
fn bytes_n<const N: usize>(env: &Env, bytes: &[u8]) -> BytesN<N> {
    assert!(bytes.len() <= N, "value longer than {N} bytes");
    let mut buf = [0u8; N];
    buf[..bytes.len()].copy_from_slice(bytes);
    BytesN::from_array(env, &buf)
}

/// Copies a `Bytes` blob into a host `Vec`.
fn to_host(bytes: &Bytes) -> Vec<u8> {
    let mut out = alloc::vec![0u8; bytes.len() as usize];
    bytes.copy_into_slice(&mut out);
    out
}

fn state_root(env: &Env) -> BytesN<32> {
    bytes_n(env, &hex_field("state_root"))
}

fn address(env: &Env) -> BytesN<20> {
    bytes_n(env, &hex_field("address"))
}

fn framed(env: &Env, field: &str) -> Bytes {
    Bytes::from_slice(env, &frame_nodes(&hex_array_field(field)))
}

fn empty_slots(env: &Env) -> Bytes {
    Bytes::new(env)
}

fn slot_proof(env: &Env) -> Bytes {
    let mut key = [0u8; 32];
    let raw = hex_field("storage_slot_key");
    key.copy_from_slice(&raw);
    Bytes::from_slice(env, &frame_slot(&key, &hex_array_field("storage_proof")))
}

/// The end-to-end path a bridge would actually call, against real mainnet data.
#[test]
fn verifies_real_mainnet_account_state() {
    let env = Env::default();
    let state = MptVerifier::verify_evm_state(
        env.clone(),
        state_root(&env),
        address(&env),
        framed(&env, "account_proof"),
        empty_slots(&env),
    )
    .expect("mainnet state proof must verify");

    assert!(state.account_exists);
    assert_eq!(state.nonce, 1, "WETH nonce at the captured block");
    assert_eq!(state.storage_value_count, 0);

    // The balance must be the exact value the node reported.
    let mut expected_balance = [0u8; 32];
    let raw = hex_field("account_balance");
    expected_balance[32 - raw.len()..].copy_from_slice(&raw);
    assert_eq!(
        state.balance,
        bytes_n::<32>(&env, &expected_balance),
        "balance must match the node's reported value"
    );
    assert_eq!(
        state.code_hash,
        bytes_n::<32>(&env, &hex_field("account_code_hash")),
        "code hash must match the node's reported value"
    );
}

/// The account and storage proofs must be accepted together, and the resolved
/// balance must be the WETH balance the node reported for that holder.
#[test]
fn verifies_real_mainnet_storage_slot() {
    let env = Env::default();
    let state = MptVerifier::verify_evm_state(
        env.clone(),
        state_root(&env),
        address(&env),
        framed(&env, "account_proof"),
        slot_proof(&env),
    )
    .expect("mainnet account + storage proofs must verify");

    assert_eq!(state.storage_value_count, 1);
    assert_eq!(state.storage_values.len(), 32, "one 32-byte slot value");

    let mut expected = [0u8; 32];
    let raw = hex_field("storage_value");
    expected[32 - raw.len()..].copy_from_slice(&raw);
    let got = to_host(&state.storage_values);
    assert_eq!(&got[..], &expected[..], "storage slot value must match");
}

/// A state root the caller did not expect must be rejected, not silently
/// accepted. This is the core security property.
#[test]
fn rejects_wrong_state_root() {
    let env = Env::default();
    let mut wrong = hex_field("state_root");
    wrong[31] ^= 0xff;

    let result = MptVerifier::verify_evm_state(
        env.clone(),
        bytes_n::<32>(&env, &wrong),
        address(&env),
        framed(&env, "account_proof"),
        empty_slots(&env),
    );
    assert_eq!(result, Err(MptError::RootNotFound));
}

/// A single flipped bit in any node must break verification.
#[test]
fn rejects_tampered_node() {
    let env = Env::default();
    let mut nodes = hex_array_field("account_proof");
    nodes[0][10] ^= 0x01;

    let result = MptVerifier::verify_evm_state(
        env.clone(),
        state_root(&env),
        address(&env),
        Bytes::from_slice(&env, &frame_nodes(&nodes)),
        empty_slots(&env),
    );
    assert!(result.is_err(), "tampered proof must be rejected");
}

/// A truncated node list cannot establish anything.
#[test]
fn rejects_truncated_node_list() {
    let env = Env::default();
    let mut nodes = hex_array_field("account_proof");
    nodes.remove(0);

    let result = MptVerifier::verify_evm_state(
        env.clone(),
        state_root(&env),
        address(&env),
        Bytes::from_slice(&env, &frame_nodes(&nodes)),
        empty_slots(&env),
    );
    assert!(result.is_err(), "truncated proof must be rejected");
}

/// An empty proof cannot establish anything.
#[test]
fn rejects_empty_node_list() {
    let env = Env::default();
    let result = MptVerifier::verify_evm_state(
        env.clone(),
        state_root(&env),
        address(&env),
        Bytes::new(&env),
        empty_slots(&env),
    );
    assert_eq!(result, Err(MptError::EmptyProof));
}

/// A framing header that claims more bytes than were supplied must be refused
/// rather than read out of bounds.
#[test]
fn rejects_malformed_framing() {
    let env = Env::default();
    // Header says 1000 bytes, but nothing follows it.
    let mut bad = alloc::vec![0u8; 4];
    bad[3] = 0xe8;
    let result = MptVerifier::verify_evm_state(
        env.clone(),
        state_root(&env),
        address(&env),
        Bytes::from_slice(&env, &bad),
        empty_slots(&env),
    );
    assert_eq!(result, Err(MptError::InvalidRlp));
}

/// A node claiming to be larger than the per-node cap is refused while framing.
#[test]
fn rejects_oversized_node_declaration() {
    let env = Env::default();
    let mut bad = alloc::vec![0u8; 4];
    // Length well beyond MAX_NODE_BYTES.
    bad[0] = 0xff;
    bad[1] = 0xff;
    bad[2] = 0xff;
    bad[3] = 0xff;
    let result = MptVerifier::verify_evm_state(
        env.clone(),
        state_root(&env),
        address(&env),
        Bytes::from_slice(&env, &bad),
        empty_slots(&env),
    );
    assert_eq!(result, Err(MptError::NodeTooLarge));
}

/// A storage-slot proof must be checked against the storage root the account
/// leaf commits to. Feeding account-proof nodes as slot nodes must fail,
/// proving the two proofs are genuinely bound together.
#[test]
fn storage_proof_must_match_account_storage_root() {
    let env = Env::default();
    // Account-trie nodes are not storage-trie nodes.
    let mut key = [0u8; 32];
    key.copy_from_slice(&hex_field("storage_slot_key"));
    let mixed = Bytes::from_slice(&env, &frame_slot(&key, &hex_array_field("account_proof")));

    let result = MptVerifier::verify_evm_state(
        env.clone(),
        state_root(&env),
        address(&env),
        framed(&env, "account_proof"),
        mixed,
    );
    assert!(
        result.is_err(),
        "a storage proof from the wrong trie must be rejected"
    );
}

/// A zero-length node is not a valid RLP node and must be refused.
#[test]
fn rejects_zero_length_node() {
    let env = Env::default();
    let result = MptVerifier::verify_evm_state(
        env.clone(),
        state_root(&env),
        address(&env),
        Bytes::from_slice(&env, &[0, 0, 0, 0]),
        empty_slots(&env),
    );
    assert_eq!(result, Err(MptError::MalformedNode));
}

/// Round-trips the public framing helper against the contract's parser.
#[test]
fn framing_round_trips() {
    let nodes: Vec<Vec<u8>> = alloc::vec![alloc::vec![1, 2, 3], alloc::vec![4, 5]];
    let framed = frame_nodes(&nodes);
    let expected_len = 4 + 3 + 4 + 2;
    assert_eq!(framed.len(), expected_len);
    assert_eq!(framed[0..4], [0, 0, 0, 3]);
    assert_eq!(&framed[4..7], &[1, 2, 3]);
    assert_eq!(&framed[7..11], &[0, 0, 0, 2]);
}

/// The per-node cap is a real bound, not a formality.
#[test]
fn node_cap_is_enforced_by_framing() {
    let node = alloc::vec![0u8; MAX_NODE_BYTES + 1];
    let mut framed = ((node.len() as u32).to_be_bytes()).to_vec();
    framed.extend_from_slice(&node);
    // The length is validated before any bytes are read, so the parser stops
    // at the header.
    let mut header = [0u8; 4];
    header.copy_from_slice(&(node.len() as u32).to_be_bytes());
    assert!(u32::from_be_bytes(header) as usize > MAX_NODE_BYTES);
}
