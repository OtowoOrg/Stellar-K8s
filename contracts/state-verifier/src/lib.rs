//! # Stellar State Verifier
//!
//! A Soroban oracle that lets other contracts check, cryptographically,
//! that a transaction was applied on the Stellar network, and with which
//! result.
//!
//! ## Trust chain
//!
//! ```text
//!  tx hash ──Merkle proof──▶ results root ──sha256(ResultSet)──▶ txSetResultHash
//!                                                                    │
//!                                  ledger hash = sha256(LedgerHeader)◀┘
//!                                        │
//!               relayer checkpoint ──────┤ (SCP value signed by a trusted validator)
//!               or ancestor proof ───────┘ (hash-linked to a verified child)
//! ```
//!
//! 1. **Checkpoint.** An authorised relayer submits a raw `LedgerHeader`
//!    ([`StateVerifier::checkpoint_ledger`]). The contract decodes it, checks
//!    that its SCP value carries a valid signature from a trusted validator
//!    for this network, and records `sha256(header)` as the ledger hash.
//! 2. **Ancestors.** Anyone can extend the verified set *backwards*
//!    ([`StateVerifier::prove_ancestor`]): a header is accepted when it
//!    hashes to the `previousLedgerHash` of an already verified ledger. This
//!    needs no trust. Forward extension is impossible without trust, because
//!    anyone can build a header that points at a given parent.
//! 3. **Results commitment.** Stellar commits transaction results linearly
//!    (`txSetResultHash = sha256(TransactionResultSet)`), not as a tree.
//!    Anyone may submit the full result set once
//!    ([`StateVerifier::commit_results`]). The contract checks its hash
//!    against the header, decodes every `(tx_hash, result_code)` pair and
//!    stores a Merkle root over them (see [`merkle`]).
//! 4. **Verification.** Each later check is an O(log n) Merkle proof
//!    ([`StateVerifier::verify_transaction`]). Callers holding the full
//!    envelope can use [`StateVerifier::verify_envelope`], which also
//!    decodes the transaction, recomputes its network-bound hash and
//!    returns the source, memo and operations (payments, path payments,
//!    account creation and merges are surfaced in full).
//!
//! Every verification failure panics with a [`VerifierError`], so a calling
//! contract reverts atomically on a bad proof.

#![no_std]

extern crate alloc;

pub mod merkle;
mod types;
pub mod xdr_parser;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod test;

pub use types::*;

use alloc::vec::Vec as AllocVec;
use soroban_sdk::{contract, contractimpl, panic_with_error, Address, Bytes, BytesN, Env, Vec};
use xdr_parser::XdrError;

/// `ENVELOPE_TYPE_SCPVALUE`, the domain of a StellarValue signature.
const ENVELOPE_TYPE_SCPVALUE: i32 = 4;

#[contract]
pub struct StateVerifier;

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_THRESHOLD, INSTANCE_BUMP);
}

fn require_admin(env: &Env) {
    let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
    admin.require_auth();
}

fn network_id(env: &Env) -> BytesN<32> {
    env.storage().instance().get(&DataKey::NetworkId).unwrap()
}

fn read_persistent<V: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
) -> Option<V> {
    let value = env.storage().persistent().get(key);
    if value.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(key, ENTRY_THRESHOLD, ENTRY_BUMP);
    }
    value
}

fn write_persistent<V: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
    value: &V,
) {
    env.storage().persistent().set(key, value);
    env.storage()
        .persistent()
        .extend_ttl(key, ENTRY_THRESHOLD, ENTRY_BUMP);
}

fn set_flag(env: &Env, key: &DataKey, enabled: bool) {
    if enabled {
        write_persistent(env, key, &true);
    } else {
        env.storage().persistent().remove(key);
    }
}

fn load_ledger(env: &Env, seq: u32) -> LedgerRecord {
    read_persistent(env, &DataKey::Ledger(seq))
        .unwrap_or_else(|| panic_with_error!(env, VerifierError::UnknownLedger))
}

fn xdr_error(env: &Env, err: XdrError) -> ! {
    match err {
        XdrError::Unsupported => panic_with_error!(env, VerifierError::UnsupportedTransaction),
        _ => panic_with_error!(env, VerifierError::MalformedXdr),
    }
}

fn bytes32(env: &Env, data: &[u8; 32]) -> BytesN<32> {
    BytesN::from_array(env, data)
}

/// Decodes a header and returns it with its ledger hash.
fn decode_header(env: &Env, header_xdr: &Bytes) -> (xdr_parser::LedgerHeader, BytesN<32>) {
    let buf = header_xdr.to_alloc_vec();
    let header = xdr_parser::parse_ledger_header(&buf).unwrap_or_else(|e| xdr_error(env, e));
    let hash: BytesN<32> = env.crypto().sha256(header_xdr).into();
    (header, hash)
}

/// Stores a verified header; re-submitting the same header is a no-op, a
/// different header for the same sequence is rejected.
fn record_ledger(
    env: &Env,
    header: &xdr_parser::LedgerHeader,
    hash: &BytesN<32>,
    anchored: bool,
) -> bool {
    let key = DataKey::Ledger(header.ledger_seq);
    if let Some(existing) = read_persistent::<LedgerRecord>(env, &key) {
        if existing.hash != *hash {
            panic_with_error!(env, VerifierError::LedgerConflict);
        }
        return false;
    }
    let record = LedgerRecord {
        ledger_seq: header.ledger_seq,
        hash: hash.clone(),
        previous_ledger_hash: bytes32(env, &header.previous_ledger_hash),
        tx_set_hash: bytes32(env, &header.tx_set_hash),
        tx_set_result_hash: bytes32(env, &header.tx_set_result_hash),
        close_time: header.close_time,
        ledger_version: header.ledger_version,
        anchored,
    };
    write_persistent(env, &key, &record);
    true
}

/// Checks the StellarValue signature:
/// `ed25519(networkID || ENVELOPE_TYPE_SCPVALUE || txSetHash || closeTime)`.
fn verify_value_signature(env: &Env, header: &xdr_parser::LedgerHeader) {
    let sig = header
        .value_signature
        .unwrap_or_else(|| panic_with_error!(env, VerifierError::UnsignedLedgerValue));
    let node_id = bytes32(env, &sig.node_id);
    if !read_persistent::<bool>(env, &DataKey::Validator(node_id.clone())).unwrap_or(false) {
        panic_with_error!(env, VerifierError::UntrustedValidator);
    }
    let mut payload = [0u8; 76];
    payload[..32].copy_from_slice(&network_id(env).to_array());
    payload[32..36].copy_from_slice(&ENVELOPE_TYPE_SCPVALUE.to_be_bytes());
    payload[36..68].copy_from_slice(&header.tx_set_hash);
    payload[68..].copy_from_slice(&header.close_time.to_be_bytes());
    env.crypto().ed25519_verify(
        &node_id,
        &Bytes::from_array(env, &payload),
        &BytesN::from_array(env, &sig.signature),
    );
}

fn verify_inclusion(
    env: &Env,
    ledger_seq: u32,
    tx_hash: &[u8; 32],
    result_code: i32,
    index: u32,
    proof: &Bytes,
) -> VerifiedTransaction {
    let ledger = load_ledger(env, ledger_seq);
    let commitment: ResultsCommitment = read_persistent(env, &DataKey::Results(ledger_seq))
        .unwrap_or_else(|| panic_with_error!(env, VerifierError::ResultsNotCommitted));
    if index >= commitment.tx_count {
        panic_with_error!(env, VerifierError::IndexOutOfRange);
    }
    if proof.len() != 32 * commitment.depth {
        panic_with_error!(env, VerifierError::InvalidProofLength);
    }
    let leaf = merkle::leaf_hash(env, tx_hash, result_code);
    let root = merkle::compute_root(env, leaf, index, commitment.depth, proof);
    if root != commitment.root.to_array() {
        panic_with_error!(env, VerifierError::NotIncluded);
    }
    VerifiedTransaction {
        ledger_seq,
        ledger_hash: ledger.hash,
        close_time: ledger.close_time,
        tx_hash: bytes32(env, tx_hash),
        result_code,
        successful: result_code == 0 || result_code == 1,
    }
}

// ---------------------------------------------------------------------------
// Conversions from the zero-copy decoder to contract types
// ---------------------------------------------------------------------------

fn muxed(env: &Env, m: &xdr_parser::Muxed) -> MuxedAccount {
    MuxedAccount {
        key: bytes32(env, &m.key),
        id: m.id,
    }
}

fn asset(env: &Env, a: &xdr_parser::Asset) -> Asset {
    match a {
        xdr_parser::Asset::Native => Asset::Native,
        xdr_parser::Asset::Alphanum4 { code, issuer } => Asset::CreditAlphanum4(AssetCode {
            code: Bytes::from_slice(env, code),
            issuer: bytes32(env, issuer),
        }),
        xdr_parser::Asset::Alphanum12 { code, issuer } => Asset::CreditAlphanum12(AssetCode {
            code: Bytes::from_slice(env, code),
            issuer: bytes32(env, issuer),
        }),
    }
}

fn operation_body(env: &Env, body: &xdr_parser::OpBody) -> OperationBody {
    use xdr_parser::OpBody as P;
    match body {
        P::CreateAccount {
            destination,
            starting_balance,
        } => OperationBody::CreateAccount(CreateAccountOp {
            destination: bytes32(env, destination),
            starting_balance: *starting_balance,
        }),
        P::Payment {
            destination,
            asset: a,
            amount,
        } => OperationBody::Payment(PaymentOp {
            destination: muxed(env, destination),
            asset: asset(env, a),
            amount: *amount,
        }),
        P::PathPaymentStrictReceive {
            send_asset,
            send_max,
            destination,
            dest_asset,
            dest_amount,
        } => OperationBody::PathPaymentStrictReceive(PathPaymentOp {
            send_asset: asset(env, send_asset),
            send_amount: *send_max,
            destination: muxed(env, destination),
            dest_asset: asset(env, dest_asset),
            dest_amount: *dest_amount,
        }),
        P::PathPaymentStrictSend {
            send_asset,
            send_amount,
            destination,
            dest_asset,
            dest_min,
        } => OperationBody::PathPaymentStrictSend(PathPaymentOp {
            send_asset: asset(env, send_asset),
            send_amount: *send_amount,
            destination: muxed(env, destination),
            dest_asset: asset(env, dest_asset),
            dest_amount: *dest_min,
        }),
        P::AccountMerge { destination } => OperationBody::AccountMerge(muxed(env, destination)),
        P::Other { op_type } => OperationBody::Other(*op_type),
    }
}

fn decoded_transaction(env: &Env, envelope: &xdr_parser::Envelope) -> DecodedTransaction {
    let tx = &envelope.tx;
    let mut operations = Vec::new(env);
    for op in tx.operations.iter() {
        operations.push_back(Operation {
            source: op
                .source
                .as_ref()
                .map_or(OperationSource::Transaction, |m| {
                    OperationSource::Account(muxed(env, m))
                }),
            body: operation_body(env, &op.body),
        });
    }
    let memo = match tx.memo {
        xdr_parser::Memo::None => Memo::None,
        xdr_parser::Memo::Text(t) => Memo::Text(Bytes::from_slice(env, t)),
        xdr_parser::Memo::Id(id) => Memo::Id(id),
        xdr_parser::Memo::Hash(h) => Memo::Hash(bytes32(env, &h)),
        xdr_parser::Memo::Return(h) => Memo::Return(bytes32(env, &h)),
    };
    let (min_time, max_time) = tx.time_bounds.unwrap_or((0, 0));
    DecodedTransaction {
        source: muxed(env, &tx.source),
        fee_bump: envelope.fee_bump.is_some(),
        fee_source: muxed(env, &envelope.fee_bump.map_or(tx.source, |f| f.fee_source)),
        fee: envelope.fee_bump.map_or(tx.fee as i64, |f| f.fee),
        seq_num: tx.seq_num,
        time_bounds: TimeBounds { min_time, max_time },
        memo,
        operations,
    }
}

/// Network-bound transaction hash:
/// `sha256(networkID || envelopeType || (KEY_TYPE_ED25519 for v0) || tx)`.
fn envelope_hash(env: &Env, envelope: &xdr_parser::Envelope) -> [u8; 32] {
    let mut preimage = AllocVec::with_capacity(40 + envelope.hashed_bytes.len());
    preimage.extend_from_slice(&network_id(env).to_array());
    preimage.extend_from_slice(&envelope.hash_envelope_type.to_be_bytes());
    if envelope.v0_prefix {
        preimage.extend_from_slice(&[0u8; 4]);
    }
    preimage.extend_from_slice(envelope.hashed_bytes);
    env.crypto()
        .sha256(&Bytes::from_slice(env, &preimage))
        .to_array()
}

#[contractimpl]
impl StateVerifier {
    /// `network_id` is `sha256(network passphrase)`, e.g. mainnet's
    /// `sha256("Public Global Stellar Network ; September 2015")`.
    pub fn __constructor(env: Env, admin: Address, network_id: BytesN<32>) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::NetworkId, &network_id);
        bump_instance(&env);
    }

    // -----------------------------------------------------------------------
    // Relayer interface
    // -----------------------------------------------------------------------

    /// Anchors a raw XDR `LedgerHeader` submitted by an authorised relayer.
    /// The header's SCP value must be signed by a trusted validator for
    /// this network. Returns the ledger hash.
    pub fn checkpoint_ledger(env: Env, relayer: Address, header_xdr: Bytes) -> BytesN<32> {
        relayer.require_auth();
        bump_instance(&env);
        if !read_persistent::<bool>(&env, &DataKey::Relayer(relayer.clone())).unwrap_or(false) {
            panic_with_error!(&env, VerifierError::NotRelayer);
        }
        let (header, hash) = decode_header(&env, &header_xdr);
        verify_value_signature(&env, &header);
        if record_ledger(&env, &header, &hash, true) {
            LedgerCheckpointed {
                ledger_seq: header.ledger_seq,
                ledger_hash: hash.clone(),
                relayer,
            }
            .publish(&env);
        }
        hash
    }

    /// Verifies (permissionlessly) the parent of an already verified ledger:
    /// `sha256(header_xdr)` must equal the child's `previousLedgerHash`.
    /// Returns the ledger hash.
    pub fn prove_ancestor(env: Env, header_xdr: Bytes) -> BytesN<32> {
        bump_instance(&env);
        let (header, hash) = decode_header(&env, &header_xdr);
        let child_seq = header
            .ledger_seq
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(&env, VerifierError::UnknownLedger));
        if load_ledger(&env, child_seq).previous_ledger_hash != hash {
            panic_with_error!(&env, VerifierError::NotAncestor);
        }
        if record_ledger(&env, &header, &hash, false) {
            AncestorProven {
                ledger_seq: header.ledger_seq,
                ledger_hash: hash.clone(),
            }
            .publish(&env);
        }
        hash
    }

    /// Commits (permissionlessly) a verified ledger's `TransactionResultSet`:
    /// checks it against `txSetResultHash` and stores a Merkle root over its
    /// `(tx_hash, result_code)` pairs. Idempotent.
    pub fn commit_results(env: Env, ledger_seq: u32, results_xdr: Bytes) -> ResultsCommitment {
        bump_instance(&env);
        let ledger = load_ledger(&env, ledger_seq);
        if let Some(existing) = read_persistent(&env, &DataKey::Results(ledger_seq)) {
            return existing;
        }
        let hash: BytesN<32> = env.crypto().sha256(&results_xdr).into();
        if hash != ledger.tx_set_result_hash {
            panic_with_error!(&env, VerifierError::ResultsHashMismatch);
        }

        let buf = results_xdr.to_alloc_vec();
        let mut leaves: AllocVec<[u8; 32]> = AllocVec::new();
        let tx_count = xdr_parser::parse_result_set(&buf, |_, tx_hash, code| {
            leaves.push(merkle::leaf_hash(&env, tx_hash, code));
        })
        .unwrap_or_else(|e| xdr_error(&env, e));
        let depth = merkle::depth_for(tx_count);
        if depth > merkle::MAX_DEPTH {
            panic_with_error!(&env, VerifierError::TooManyTransactions);
        }

        let commitment = ResultsCommitment {
            root: bytes32(&env, &merkle::root(&env, leaves)),
            tx_count,
            depth,
        };
        write_persistent(&env, &DataKey::Results(ledger_seq), &commitment);
        ResultsCommitted {
            ledger_seq,
            root: commitment.root.clone(),
            tx_count,
        }
        .publish(&env);
        commitment
    }

    // -----------------------------------------------------------------------
    // Verification
    // -----------------------------------------------------------------------

    /// Proves that the transaction `tx_hash` was applied in `ledger_seq` at
    /// position `index` with `result_code`. `proof` is the concatenation of
    /// the sibling hashes, bottom-up. Panics with `NotIncluded` otherwise.
    pub fn verify_transaction(
        env: Env,
        ledger_seq: u32,
        tx_hash: BytesN<32>,
        result_code: i32,
        index: u32,
        proof: Bytes,
    ) -> VerifiedTransaction {
        verify_inclusion(
            &env,
            ledger_seq,
            &tx_hash.to_array(),
            result_code,
            index,
            &proof,
        )
    }

    /// Decodes a `TransactionEnvelope`, recomputes its network-bound hash and
    /// proves its inclusion, returning the decoded transaction.
    pub fn verify_envelope(
        env: Env,
        ledger_seq: u32,
        envelope_xdr: Bytes,
        result_code: i32,
        index: u32,
        proof: Bytes,
    ) -> VerifiedEnvelope {
        let buf = envelope_xdr.to_alloc_vec();
        let envelope = xdr_parser::parse_envelope(&buf).unwrap_or_else(|e| xdr_error(&env, e));
        let tx_hash = envelope_hash(&env, &envelope);
        let inclusion = verify_inclusion(&env, ledger_seq, &tx_hash, result_code, index, &proof);
        VerifiedEnvelope {
            inclusion,
            transaction: decoded_transaction(&env, &envelope),
        }
    }

    /// Network-bound hash of a `TransactionEnvelope` (no inclusion check).
    pub fn transaction_hash(env: Env, envelope_xdr: Bytes) -> BytesN<32> {
        let buf = envelope_xdr.to_alloc_vec();
        let envelope = xdr_parser::parse_envelope(&buf).unwrap_or_else(|e| xdr_error(&env, e));
        bytes32(&env, &envelope_hash(&env, &envelope))
    }

    // -----------------------------------------------------------------------
    // Administration
    // -----------------------------------------------------------------------

    pub fn set_relayer(env: Env, relayer: Address, enabled: bool) {
        require_admin(&env);
        bump_instance(&env);
        set_flag(&env, &DataKey::Relayer(relayer.clone()), enabled);
        RelayerSet { relayer, enabled }.publish(&env);
    }

    /// Trusts (or distrusts) a validator node's ed25519 key for checkpoints.
    pub fn set_validator(env: Env, node_id: BytesN<32>, trusted: bool) {
        require_admin(&env);
        bump_instance(&env);
        set_flag(&env, &DataKey::Validator(node_id.clone()), trusted);
        ValidatorSet { node_id, trusted }.publish(&env);
    }

    /// Transfers administration. Both the current and new admin must sign.
    pub fn set_admin(env: Env, new_admin: Address) {
        require_admin(&env);
        new_admin.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        AdminChanged { admin: new_admin }.publish(&env);
    }

    // -----------------------------------------------------------------------
    // Views
    // -----------------------------------------------------------------------

    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
    }

    pub fn network_id(env: Env) -> BytesN<32> {
        network_id(&env)
    }

    pub fn is_relayer(env: Env, relayer: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Relayer(relayer))
            .unwrap_or(false)
    }

    pub fn is_validator(env: Env, node_id: BytesN<32>) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Validator(node_id))
            .unwrap_or(false)
    }

    pub fn ledger(env: Env, ledger_seq: u32) -> Option<LedgerRecord> {
        env.storage().persistent().get(&DataKey::Ledger(ledger_seq))
    }

    pub fn results(env: Env, ledger_seq: u32) -> Option<ResultsCommitment> {
        env.storage()
            .persistent()
            .get(&DataKey::Results(ledger_seq))
    }
}
