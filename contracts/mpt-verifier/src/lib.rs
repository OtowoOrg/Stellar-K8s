//! # mpt-verifier
//!
//! A Soroban-native Merkle Patricia Trie validator for trustless
//! EVM-to-Stellar bridge deposits.
//!
//! ## Why this exists
//!
//! A bridge that moves assets from an EVM network to Stellar has to convince a
//! Stellar contract that a deposit really happened. Trusting a relayer to
//! "assert it saw the transfer" replaces cryptography with an assumption. This
//! contract instead takes the RLP-encoded Merkle Patricia Trie nodes Ethereum
//! itself produces and checks, on-chain, that they hash from a known state root
//! to a leaf holding the expected account or storage value.
//!
//! ## What callers supply
//!
//! The caller provides three things, all of which are untrusted:
//!
//! * the **state root** it expects (normally the `stateRoot` of a finalised
//!   block header, itself verified by a light client or checkpoint),
//! * the **account address** whose state is being claimed,
//! * the raw **RLP node list** from `eth_getProof`.
//!
//! Nothing is trusted implicitly: every node is re-hashed, the path is walked
//! to a leaf, and the leaf's path must match the requested key exactly.
//!
//! ## Denial-of-service hardening
//!
//! Proof bytes come from an attacker, so [`trie::Limits`] bounds the walk on
//! three axes — path depth, node count, and total encoded bytes — and every
//! node is decoded under bounded [`rlp::Limits`]. Traversal is iterative, so a
//! deeply nested proof cannot overflow the Wasm stack. Together these mean a
//! hostile proof is rejected rather than being allowed to exhaust the Soroban
//! budget.
//!
//! ## Example
//!
//! The verifier is used through the contract, or directly as a library:
//!
//! ```rust
//! use mpt_verifier::trie::{keccak256, verify_proof, Limits, TrieOutcome};
//!
//! // The state trie is keyed by keccak256(address).
//! let mut address = [0u8; 20];
//! address[19] = 0x01;
//! let key = keccak256(&address);
//!
//! // `proof` is the node list from eth_getProof and `state_root` the root the
//! // caller expects. Nothing about either is trusted.
//! let result = verify_proof(&[], &[0u8; 32], &key, &Limits::default());
//! assert!(result.is_err(), "an empty proof establishes nothing");
//! ```
//!
//! The verification logic is exercised end-to-end against a real Ethereum
//! mainnet state proof in `tests/mainnet_state_proof.rs`.

// The cdylib target is what ships to Soroban. Building it off-WASM (as `cargo
// test` does) has no allocator or panic handler available, so it is opt-in.
#![cfg_attr(feature = "contract-wasm", no_std)]

extern crate alloc;

pub mod account;
pub mod rlp;
pub mod trie;

use alloc::vec::Vec;

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Bytes, BytesN, Env};

use account::{decode_account, decode_storage_value};
use trie::{keccak256, verify_proof, Limits, TrieError, TrieOutcome};

/// Failures surfaced to the caller. Discriminants are stable so callers can
/// branch on them.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MptError {
    /// The proof contained no nodes.
    EmptyProof = 1,
    /// A single node exceeded the size budget.
    NodeTooLarge = 2,
    /// The whole proof exceeded the aggregate size budget.
    ProofTooLarge = 3,
    /// The proof contained more nodes than permitted.
    TooManyNodes = 4,
    /// Traversal exceeded the depth budget.
    DepthExceeded = 5,
    /// A node or value was not well-formed RLP.
    InvalidRlp = 6,
    /// A node had an invalid trie shape.
    MalformedNode = 7,
    /// The root node was absent from the proof.
    RootNotFound = 8,
    /// A referenced child node was absent from the proof.
    ChildNotFound = 9,
    /// A hex-prefix path was malformed.
    InvalidPathEncoding = 10,
    /// The proof resolved to a different key (valid non-inclusion).
    KeyMismatch = 11,
    /// The state root was not 32 bytes.
    InvalidStateRoot = 12,
    /// The account address was not 20 bytes.
    InvalidAddress = 13,
    /// The address is not in the trie, so the account does not exist.
    AccountNotFound = 14,
    /// A storage slot was requested but is not present in the trie.
    StorageSlotNotFound = 15,
    /// An account value was not a well-formed account.
    MalformedAccount = 16,
}

impl From<TrieError> for MptError {
    fn from(e: TrieError) -> Self {
        match e {
            TrieError::EmptyProof => MptError::EmptyProof,
            TrieError::NodeTooLarge => MptError::NodeTooLarge,
            TrieError::ProofTooLarge => MptError::ProofTooLarge,
            TrieError::TooManyNodes => MptError::TooManyNodes,
            TrieError::DepthExceeded => MptError::DepthExceeded,
            TrieError::Rlp(_) => MptError::InvalidRlp,
            TrieError::MalformedNode => MptError::MalformedNode,
            TrieError::RootNotFound => MptError::RootNotFound,
            TrieError::ChildNotFound => MptError::ChildNotFound,
            TrieError::InvalidPathEncoding => MptError::InvalidPathEncoding,
            TrieError::KeyMismatch => MptError::KeyMismatch,
            TrieError::EmptyTrie => MptError::AccountNotFound,
        }
    }
}

impl From<account::DecodeError> for MptError {
    fn from(e: account::DecodeError) -> Self {
        match e {
            account::DecodeError::Rlp(_) => MptError::InvalidRlp,
            account::DecodeError::MalformedAccount => MptError::MalformedAccount,
            account::DecodeError::InvalidHashLength => MptError::MalformedAccount,
            account::DecodeError::ScalarTooLarge => MptError::MalformedAccount,
        }
    }
}

/// Maximum encoded size of a single proof node accepted at the ABI boundary.
///
/// Proof nodes are variable length, but the Soroban ABI cannot carry a
/// variable-length collection in a contract type. Each node is therefore
/// supplied as a length-prefixed blob inside one `Bytes` argument, and this
/// bound is enforced while framing: a node larger than this is rejected
/// outright rather than silently truncated.
pub const MAX_NODE_BYTES: usize = 8 * 1024;

/// The verified state of an EVM account and any requested storage slots.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedEvmState {
    /// True when the account exists in the trie under the given state root.
    pub account_exists: bool,
    /// The account's nonce, or zero for an absent account.
    pub nonce: u64,
    /// The account balance in wei, big-endian and zero-padded to 32 bytes.
    pub balance: BytesN<32>,
    /// The account's storage trie root, which any slot proof must match.
    pub storage_root: BytesN<32>,
    /// The account's code hash; the empty hash for an externally owned account.
    pub code_hash: BytesN<32>,
    /// Number of storage slots resolved, matching the request order.
    pub storage_value_count: u32,
    /// Verified slot values, each big-endian and zero-padded to 32 bytes,
    /// concatenated in request order. A slot proved absent is 32 zero bytes.
    ///
    /// The ABI cannot carry a variable-length list in a contract type, so the
    /// values are returned as a single blob; the count above says how many
    /// 32-byte entries it holds.
    pub storage_values: Bytes,
}

/// The contract, exposing a single verification entry point.
#[contract]
pub struct MptVerifier;

#[contractimpl]
impl MptVerifier {
    /// Verify EVM state against a known state root.
    ///
    /// `framed_nodes` carries the `eth_getProof` node list framed as
    /// `[len: u32-be][bytes]...`, because the ABI cannot pass a
    /// variable-length list directly. The verified result is returned as a
    /// host vector via the out-parameters below so that a caller of varying slot
    /// count can be served from one invocation.
    ///
    /// Everything supplied here is attacker-controlled and is validated
    /// entirely on-chain; nothing about the proof is assumed.
    pub fn verify_evm_state(
        env: Env,
        state_root: BytesN<32>,
        account_address: BytesN<20>,
        framed_nodes: Bytes,
        framed_slots: Bytes,
    ) -> Result<VerifiedEvmState, MptError> {
        let limits = Limits::default();
        let rlp_limits = limits.rlp();

        let root = state_root.to_array();
        let address = account_address.to_array();
        // The state trie is keyed by keccak256(address).
        let key = keccak256(&address);
        let nodes = unframe_nodes(&framed_nodes)?;

        // A proof that lands on a different key, or on an empty slot, is a
        // valid Ethereum non-inclusion proof rather than a hard failure. The
        // caller asked about a specific account, so absence is reported as
        // such instead of being mistaken for presence.
        let outcome = verify_proof(&nodes, &root, &key, &limits)?;
        let account_value = match outcome {
            TrieOutcome::Present(value) => value,
            TrieOutcome::Absent => {
                return Ok(VerifiedEvmState {
                    account_exists: false,
                    nonce: 0,
                    balance: BytesN::from_array(&env, &[0u8; 32]),
                    storage_root: BytesN::from_array(&env, &trie::EMPTY_ROOT),
                    code_hash: BytesN::from_array(&env, &trie::EMPTY_CODE_HASH),
                    storage_value_count: 0,
                    storage_values: Bytes::new(&env),
                })
            }
        };

        let decoded = decode_account(&account_value, &rlp_limits)?;

        // Resolve every requested storage slot against the storage root the
        // account leaf itself commits to. That is what binds the two proofs
        // together: a slot proof is worthless unless the account owning the
        // storage root is itself proven to sit under the expected state root.
        // Slot values accumulate into a flat blob of 32-byte entries, which
        // the ABI permits; `storage_value_count` records how many are present.
        let mut storage_values: Vec<u8> = Vec::new();
        let slots = unframe_slots(&framed_slots)?;
        let slot_count = slots.len() as u32;
        for (slot_key, slot_nodes) in slots {
            let slot_hash = keccak256(&slot_key);
            match verify_proof(&slot_nodes, &decoded.storage_root, &slot_hash, &limits)? {
                TrieOutcome::Present(value) => {
                    let raw = decode_storage_value(&value, &rlp_limits)?;
                    let padded =
                        rlp::scalar_to_padded(&raw, 32).map_err(|_| MptError::MalformedAccount)?;
                    storage_values.extend_from_slice(&padded);
                }
                // A slot proved absent reads as zero.
                TrieOutcome::Absent => storage_values.extend_from_slice(&[0u8; 32]),
            }
        }

        // Left-pad the big-endian balance to the ABI's fixed 32-byte width.
        let mut balance = [0u8; 32];
        let raw_balance =
            rlp::scalar_to_padded(&decoded.balance, 32).map_err(|_| MptError::MalformedAccount)?;
        balance.copy_from_slice(&raw_balance);

        Ok(VerifiedEvmState {
            account_exists: true,
            nonce: decoded.nonce,
            balance: BytesN::from_array(&env, &balance),
            storage_root: BytesN::from_array(&env, &decoded.storage_root),
            code_hash: BytesN::from_array(&env, &decoded.code_hash),
            storage_value_count: slot_count,
            storage_values: Bytes::from_slice(&env, &storage_values),
        })
    }
}

/// Splits a length-prefixed node blob into individual RLP nodes.
///
/// Framing is `[len: u32 big-endian][len bytes]` repeated. Every length is
/// validated before it is used, so a hostile framing cannot cause an
/// out-of-bounds read or an unbounded allocation; the per-node size and the
/// aggregate node/byte budgets are then applied by the traversal itself.
fn unframe_nodes(framed: &Bytes) -> Result<Vec<Vec<u8>>, MptError> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    let total = framed.len() as usize;

    while offset < total {
        // A trailing partial header is malformed, not an empty list.
        if offset + 4 > total {
            return Err(MptError::InvalidRlp);
        }
        let mut len_bytes = [0u8; 4];
        for (i, slot) in len_bytes.iter_mut().enumerate() {
            *slot = framed.get((offset + i) as u32).unwrap_or(0);
        }
        let len = u32::from_be_bytes(len_bytes) as usize;
        offset += 4;

        if len == 0 {
            return Err(MptError::MalformedNode);
        }
        if len > MAX_NODE_BYTES {
            return Err(MptError::NodeTooLarge);
        }
        if offset + len > total {
            return Err(MptError::InvalidRlp);
        }

        let mut node = alloc::vec![0u8; len];
        for (i, slot) in node.iter_mut().enumerate() {
            *slot = framed.get((offset + i) as u32).unwrap_or(0);
        }
        out.push(node);
        offset += len;
    }

    Ok(out)
}

/// One parsed storage-slot request: the 32-byte mapping slot key and the RLP
/// nodes proving that slot's presence or absence.
type ParsedSlot = ([u8; 32], Vec<Vec<u8>>);

/// Splits the storage-slot request blob into `(slot_key, nodes)` pairs.
///
/// The layout is, per slot, a 32-byte key followed by its own length-prefixed
/// node list framed exactly like [`unframe_nodes`]. A trailing partial slot is
/// rejected rather than ignored, so a malformed request can never be silently
/// under-verified.
fn unframe_slots(framed: &Bytes) -> Result<Vec<ParsedSlot>, MptError> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    let total = framed.len() as usize;

    while offset < total {
        if offset + 32 > total {
            return Err(MptError::InvalidStateRoot);
        }
        let mut key = [0u8; 32];
        for (i, slot) in key.iter_mut().enumerate() {
            *slot = framed.get((offset + i) as u32).unwrap_or(0);
        }
        offset += 32;

        // Each slot's node list is itself length-prefixed, so read nodes until
        // the framing's end. Nested framing is unambiguous because every node
        // declares its own length.
        let mut nodes = Vec::new();
        loop {
            if offset == total {
                break;
            }
            if offset + 4 > total {
                return Err(MptError::InvalidRlp);
            }
            let mut len_bytes = [0u8; 4];
            for (i, slot) in len_bytes.iter_mut().enumerate() {
                *slot = framed.get((offset + i) as u32).unwrap_or(0);
            }
            let len = u32::from_be_bytes(len_bytes) as usize;
            offset += 4;
            if len == 0 {
                return Err(MptError::MalformedNode);
            }
            if len > MAX_NODE_BYTES {
                return Err(MptError::NodeTooLarge);
            }
            if offset + len > total {
                return Err(MptError::InvalidRlp);
            }
            let mut node = alloc::vec![0u8; len];
            for (i, slot) in node.iter_mut().enumerate() {
                *slot = framed.get((offset + i) as u32).unwrap_or(0);
            }
            nodes.push(node);
            offset += len;
        }
        out.push((key, nodes));
    }

    Ok(out)
}

/// Frames one storage-slot request, matching [`unframe_slots`].
pub fn frame_slot(slot_key: &[u8; 32], nodes: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(slot_key);
    out.extend_from_slice(&frame_nodes(nodes));
    out
}

/// Frames RLP nodes for submission as a single blob: the inverse of
/// [`unframe_nodes`]. Exposed so callers and tests build arguments the same way
/// the contract expects.
pub fn frame_nodes(nodes: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for node in nodes {
        out.extend_from_slice(&(node.len() as u32).to_be_bytes());
        out.extend_from_slice(node);
    }
    out
}

#[cfg(test)]
mod tests;
