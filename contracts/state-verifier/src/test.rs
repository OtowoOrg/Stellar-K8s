extern crate std;

use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env};
use std::vec::Vec;

use crate::fixtures as fx;
use crate::merkle;
use crate::xdr_parser::{self, OpBody, XdrError};
use crate::{Asset, OperationBody, ResultsCommitment, StateVerifier, StateVerifierClient};

const MAINNET_PASSPHRASE: &str = "Public Global Stellar Network ; September 2015";

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex32(s: &str) -> [u8; 32] {
    hex(s).try_into().unwrap()
}

fn bytes(env: &Env, s: &str) -> Bytes {
    Bytes::from_slice(env, &hex(s))
}

fn mainnet_network_id(env: &Env) -> BytesN<32> {
    env.crypto()
        .sha256(&Bytes::from_slice(env, MAINNET_PASSPHRASE.as_bytes()))
        .into()
}

fn header_node_id() -> [u8; 32] {
    xdr_parser::parse_ledger_header(&hex(fx::HEADER_XDR))
        .unwrap()
        .value_signature
        .unwrap()
        .node_id
}

/// Leaves of the fixture ledger, decoded with the crate's own parser.
fn fixture_leaves(env: &Env) -> Vec<[u8; 32]> {
    let mut leaves = Vec::new();
    xdr_parser::parse_result_set(&hex(fx::RESULTS_XDR), |_, h, c| {
        leaves.push(merkle::leaf_hash(env, h, c))
    })
    .unwrap();
    leaves
}

fn proof_for(env: &Env, index: u32) -> Bytes {
    Bytes::from_slice(
        env,
        &merkle::proof(env, &fixture_leaves(env), index as usize),
    )
}

struct Setup<'a> {
    env: Env,
    admin: Address,
    relayer: Address,
    client: StateVerifierClient<'a>,
}

impl<'a> Setup<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_unlimited();
        let admin = Address::generate(&env);
        let relayer = Address::generate(&env);
        let id = env.register(StateVerifier, (admin.clone(), mainnet_network_id(&env)));
        let client = StateVerifierClient::new(&env, &id);
        client.set_relayer(&relayer, &true);
        client.set_validator(&BytesN::from_array(&env, &header_node_id()), &true);
        Setup {
            env,
            admin,
            relayer,
            client,
        }
    }

    fn checkpoint(&self) -> BytesN<32> {
        self.client
            .checkpoint_ledger(&self.relayer, &bytes(&self.env, fx::HEADER_XDR))
    }

    fn committed(&self) -> ResultsCommitment {
        self.checkpoint();
        self.client
            .commit_results(&fx::LEDGER_SEQ, &bytes(&self.env, fx::RESULTS_XDR))
    }
}

// ---------------------------------------------------------------------------
// XDR decoding of real mainnet data
// ---------------------------------------------------------------------------

#[test]
fn decodes_mainnet_ledger_header() {
    let header = xdr_parser::parse_ledger_header(&hex(fx::HEADER_XDR)).unwrap();
    assert_eq!(header.ledger_seq, fx::LEDGER_SEQ);
    assert!(header.ledger_version >= 23);
    assert!(header.value_signature.is_some());
    let parent = xdr_parser::parse_ledger_header(&hex(fx::PARENT_HEADER_XDR)).unwrap();
    assert_eq!(parent.ledger_seq, fx::LEDGER_SEQ - 1);
}

#[test]
fn decodes_every_mainnet_result_pair() {
    let mut codes = Vec::new();
    let n = xdr_parser::parse_result_set(&hex(fx::RESULTS_XDR), |i, _, c| {
        assert_eq!(i as usize, codes.len());
        codes.push(c)
    })
    .unwrap();
    assert_eq!(n, fx::TX_COUNT);
    assert_eq!(codes[fx::PAYMENT_INDEX as usize], fx::PAYMENT_RESULT_CODE);
    assert_eq!(codes[fx::FEE_BUMP_INDEX as usize], fx::FEE_BUMP_RESULT_CODE);
}

#[test]
fn decodes_mainnet_payment_and_fee_bump_envelopes() {
    let payment_xdr = hex(fx::PAYMENT_ENVELOPE_XDR);
    let payment = xdr_parser::parse_envelope(&payment_xdr).unwrap();
    assert!(payment.fee_bump.is_none());
    assert!(!payment.tx.operations.is_empty());
    assert!(payment
        .tx
        .operations
        .iter()
        .all(|op| matches!(op.body, OpBody::Payment { .. })));

    let bump_xdr = hex(fx::FEE_BUMP_ENVELOPE_XDR);
    let bump = xdr_parser::parse_envelope(&bump_xdr).unwrap();
    assert!(bump.fee_bump.is_some());
    assert!(!bump.tx.operations.is_empty());
}

#[test]
fn rejects_truncated_and_padded_inputs() {
    let header = hex(fx::HEADER_XDR);
    assert_eq!(
        xdr_parser::parse_ledger_header(&header[..header.len() - 1]),
        Err(XdrError::Truncated)
    );
    let mut padded = header.clone();
    padded.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(
        xdr_parser::parse_ledger_header(&padded),
        Err(XdrError::TrailingBytes)
    );

    let results = hex(fx::RESULTS_XDR);
    assert!(xdr_parser::parse_result_set(&results[..results.len() - 4], |_, _, _| {}).is_err());
    let envelope = hex(fx::PAYMENT_ENVELOPE_XDR);
    assert!(xdr_parser::parse_envelope(&envelope[..envelope.len() - 1]).is_err());
}

// ---------------------------------------------------------------------------
// Synthetic envelopes
// ---------------------------------------------------------------------------

/// Tiny XDR writer for hand-built envelopes.
#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn i32(mut self, v: i32) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u64(mut self, v: u64) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn raw(mut self, b: &[u8]) -> Self {
        self.0.extend_from_slice(b);
        self
    }
    fn opaque(self, b: &[u8]) -> Self {
        let pad = (4 - b.len() % 4) % 4;
        self.i32(b.len() as i32).raw(b).raw(&[0u8; 3][..pad])
    }
}

/// v1 envelope: muxed source, memo text, one native payment, no signatures.
fn synthetic_envelope(op: W, ext: i32) -> Vec<u8> {
    W::default()
        .i32(2) // ENVELOPE_TYPE_TX
        .i32(0x100) // KEY_TYPE_MUXED_ED25519
        .u64(42)
        .raw(&[7u8; 32])
        .i32(100) // fee
        .u64(9) // seqNum
        .i32(1) // PRECOND_TIME
        .u64(10)
        .u64(20)
        .i32(1) // MEMO_TEXT
        .opaque(b"bridge-deposit")
        .i32(1) // one operation
        .i32(0) // no op source
        .raw(&op.0)
        .i32(ext)
        .i32(0) // no signatures
        .0
}

fn native_payment() -> W {
    W::default()
        .i32(1) // PAYMENT
        .i32(0)
        .raw(&[9u8; 32])
        .i32(0) // native
        .u64(5_000_000)
}

#[test]
fn decodes_muxed_source_memo_and_time_bounds() {
    let xdr = synthetic_envelope(native_payment(), 0);
    let env = xdr_parser::parse_envelope(&xdr).unwrap();
    assert_eq!(env.tx.source.id, Some(42));
    assert_eq!(env.tx.time_bounds, Some((10, 20)));
    assert_eq!(env.tx.memo, xdr_parser::Memo::Text(b"bridge-deposit"));
    assert_eq!(
        env.tx.operations[0].body,
        OpBody::Payment {
            destination: xdr_parser::Muxed {
                key: [9u8; 32],
                id: None
            },
            asset: xdr_parser::Asset::Native,
            amount: 5_000_000,
        }
    );
    // Hashed bytes are the Transaction: everything but type and signatures.
    assert_eq!(env.hashed_bytes, &xdr[4..xdr.len() - 4]);
}

#[test]
fn soroban_transactions_are_unsupported_by_the_envelope_decoder() {
    let invoke = W::default().i32(24);
    assert_eq!(
        xdr_parser::parse_envelope(&synthetic_envelope(invoke, 0)),
        Err(XdrError::Unsupported)
    );
    assert_eq!(
        xdr_parser::parse_envelope(&synthetic_envelope(native_payment(), 1)),
        Err(XdrError::Unsupported)
    );
}

#[test]
fn rejects_unknown_discriminants_and_nonzero_padding() {
    assert_eq!(
        xdr_parser::parse_envelope(&synthetic_envelope(W::default().i32(99), 0)),
        Err(XdrError::Invalid)
    );
    let mut xdr = synthetic_envelope(native_payment(), 0);
    // Memo text "bridge-deposit" is 14 bytes -> 2 padding bytes follow it.
    let pad_at = 4 + 4 + 8 + 32 + 4 + 8 + 4 + 16 + 4 + 4 + 14;
    xdr[pad_at] = 1;
    assert_eq!(xdr_parser::parse_envelope(&xdr), Err(XdrError::Invalid));
}

#[test]
fn v0_envelope_hashes_like_its_v1_equivalent() {
    let s = Setup::new();
    let v1 = synthetic_envelope(native_payment(), 0);
    // Rebuild as v0: plain ed25519 source (no muxing), then identical fields.
    let mut v0 = W::default().i32(0).raw(&[7u8; 32]).0;
    v0.extend_from_slice(&v1[4 + 4 + 8 + 32..]);
    let mut v1_plain = W::default().i32(2).i32(0).raw(&[7u8; 32]).0;
    v1_plain.extend_from_slice(&v1[4 + 4 + 8 + 32..]);

    let h0 = s.client.transaction_hash(&Bytes::from_slice(&s.env, &v0));
    let h1 = s
        .client
        .transaction_hash(&Bytes::from_slice(&s.env, &v1_plain));
    assert_eq!(h0, h1);
}

// ---------------------------------------------------------------------------
// Merkle commitment
// ---------------------------------------------------------------------------

#[test]
fn merkle_depths() {
    assert_eq!(
        [0, 1, 2, 3, 4, 5, 8, 9, 172].map(merkle::depth_for),
        [0, 0, 1, 2, 2, 3, 3, 4, 8]
    );
}

#[test]
fn merkle_proofs_verify_for_every_leaf_and_size() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    for n in [1usize, 2, 3, 5, 8, 9, 17] {
        let leaves: Vec<[u8; 32]> = (0..n)
            .map(|i| merkle::leaf_hash(&env, &[i as u8; 32], i as i32))
            .collect();
        let root = merkle::root(&env, leaves.clone().into_iter().collect());
        let depth = merkle::depth_for(n as u32);
        for (i, leaf) in leaves.iter().enumerate() {
            let proof = Bytes::from_slice(&env, &merkle::proof(&env, &leaves, i));
            assert_eq!(
                merkle::compute_root(&env, *leaf, i as u32, depth, &proof),
                root
            );
            // The same leaf at a different position must not verify.
            if n > 1 {
                let other = ((i + 1) % n) as u32;
                assert_ne!(
                    merkle::compute_root(&env, *leaf, other, depth, &proof),
                    root
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Relayer checkpoints
// ---------------------------------------------------------------------------

#[test]
fn checkpoints_mainnet_header_with_valid_validator_signature() {
    let s = Setup::new();
    let hash = s.checkpoint();
    assert_eq!(hash.to_array(), hex32(fx::LEDGER_HASH));

    let record = s.client.ledger(&fx::LEDGER_SEQ).unwrap();
    assert_eq!(record.hash, hash);
    assert!(record.anchored);
    let parent_hash = s.env.crypto().sha256(&bytes(&s.env, fx::PARENT_HEADER_XDR));
    assert_eq!(
        record.previous_ledger_hash.to_array(),
        parent_hash.to_array()
    );

    // Re-submitting the same header is a no-op.
    assert_eq!(s.checkpoint(), hash);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn checkpoint_requires_relayer() {
    let s = Setup::new();
    let outsider = Address::generate(&s.env);
    s.client
        .checkpoint_ledger(&outsider, &bytes(&s.env, fx::HEADER_XDR));
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn checkpoint_requires_trusted_validator() {
    let s = Setup::new();
    s.client
        .set_validator(&BytesN::from_array(&s.env, &header_node_id()), &false);
    s.checkpoint();
}

#[test]
#[should_panic]
fn checkpoint_rejects_tampered_signed_value() {
    let s = Setup::new();
    let mut header = hex(fx::HEADER_XDR);
    header[4 + 32 + 32 + 7] ^= 1; // flip a closeTime bit
    s.client
        .checkpoint_ledger(&s.relayer, &Bytes::from_slice(&s.env, &header));
}

#[test]
#[should_panic]
fn checkpoint_rejects_header_from_another_network() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let testnet: BytesN<32> = env
        .crypto()
        .sha256(&Bytes::from_slice(
            &env,
            b"Test SDF Network ; September 2015",
        ))
        .into();
    let client = StateVerifierClient::new(&env, &env.register(StateVerifier, (admin, testnet)));
    let relayer = Address::generate(&env);
    client.set_relayer(&relayer, &true);
    client.set_validator(&BytesN::from_array(&env, &header_node_id()), &true);
    client.checkpoint_ledger(&relayer, &bytes(&env, fx::HEADER_XDR));
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn checkpoint_rejects_malformed_header() {
    let s = Setup::new();
    s.client
        .checkpoint_ledger(&s.relayer, &Bytes::from_slice(&s.env, &[0u8; 12]));
}

// ---------------------------------------------------------------------------
// Ancestor proofs
// ---------------------------------------------------------------------------

#[test]
fn proves_parent_ledger_without_trust() {
    let s = Setup::new();
    s.checkpoint();
    let parent_hash = s
        .client
        .prove_ancestor(&bytes(&s.env, fx::PARENT_HEADER_XDR));
    let record = s.client.ledger(&(fx::LEDGER_SEQ - 1)).unwrap();
    assert_eq!(record.hash, parent_hash);
    assert!(!record.anchored);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn ancestor_requires_verified_child() {
    let s = Setup::new();
    s.client
        .prove_ancestor(&bytes(&s.env, fx::PARENT_HEADER_XDR));
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn ancestor_must_hash_to_previous_ledger_hash() {
    let s = Setup::new();
    s.checkpoint();
    let mut parent = hex(fx::PARENT_HEADER_XDR);
    let n = parent.len();
    parent[n - 20] ^= 1; // tamper with a skip-list byte
    s.client.prove_ancestor(&Bytes::from_slice(&s.env, &parent));
}

// ---------------------------------------------------------------------------
// Results commitment
// ---------------------------------------------------------------------------

#[test]
fn commits_results_matching_independent_root() {
    let s = Setup::new();
    let c = s.committed();
    assert_eq!(c.tx_count, fx::TX_COUNT);
    assert_eq!(c.depth, fx::RESULTS_DEPTH);
    // Root computed by scripts/fetch_fixture.py, independently of the contract.
    assert_eq!(c.root.to_array(), hex32(fx::RESULTS_ROOT));
    assert_eq!(s.client.results(&fx::LEDGER_SEQ), Some(c.clone()));
    // Idempotent.
    assert_eq!(
        s.client
            .commit_results(&fx::LEDGER_SEQ, &bytes(&s.env, fx::RESULTS_XDR)),
        c
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn results_must_match_header_commitment() {
    let s = Setup::new();
    s.checkpoint();
    let mut results = hex(fx::RESULTS_XDR);
    results[10] ^= 1;
    s.client
        .commit_results(&fx::LEDGER_SEQ, &Bytes::from_slice(&s.env, &results));
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn results_require_verified_ledger() {
    let s = Setup::new();
    s.client
        .commit_results(&fx::LEDGER_SEQ, &bytes(&s.env, fx::RESULTS_XDR));
}

// ---------------------------------------------------------------------------
// Inclusion verification
// ---------------------------------------------------------------------------

#[test]
fn every_mainnet_transaction_verifies() {
    let s = Setup::new();
    s.committed();
    let leaves = fixture_leaves(&s.env);
    let mut pairs = Vec::new();
    xdr_parser::parse_result_set(&hex(fx::RESULTS_XDR), |_, h, c| pairs.push((*h, c))).unwrap();
    for (i, (hash, code)) in pairs.iter().enumerate() {
        let proof = Bytes::from_slice(&s.env, &merkle::proof(&s.env, &leaves, i));
        let v = s.client.verify_transaction(
            &fx::LEDGER_SEQ,
            &BytesN::from_array(&s.env, hash),
            code,
            &(i as u32),
            &proof,
        );
        assert_eq!(v.result_code, *code);
        assert_eq!(v.successful, *code == 0 || *code == 1);
        assert_eq!(v.ledger_hash.to_array(), hex32(fx::LEDGER_HASH));
    }
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn forged_result_code_is_rejected() {
    let s = Setup::new();
    s.committed();
    // Claim a failed transaction succeeded.
    let failed = {
        let mut found = None;
        xdr_parser::parse_result_set(&hex(fx::RESULTS_XDR), |i, h, c| {
            if c == -1 && found.is_none() {
                found = Some((i, *h));
            }
        })
        .unwrap();
        found.unwrap()
    };
    s.client.verify_transaction(
        &fx::LEDGER_SEQ,
        &BytesN::from_array(&s.env, &failed.1),
        &0,
        &failed.0,
        &proof_for(&s.env, failed.0),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn unknown_transaction_is_rejected() {
    let s = Setup::new();
    s.committed();
    s.client.verify_transaction(
        &fx::LEDGER_SEQ,
        &BytesN::from_array(&s.env, &[0xAB; 32]),
        &0,
        &fx::PAYMENT_INDEX,
        &proof_for(&s.env, fx::PAYMENT_INDEX),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn index_beyond_transaction_count_is_rejected() {
    let s = Setup::new();
    s.committed();
    s.client.verify_transaction(
        &fx::LEDGER_SEQ,
        &BytesN::from_array(&s.env, &hex32(fx::PAYMENT_TX_HASH)),
        &0,
        &fx::TX_COUNT,
        &proof_for(&s.env, 0),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn proof_of_wrong_length_is_rejected() {
    let s = Setup::new();
    s.committed();
    s.client.verify_transaction(
        &fx::LEDGER_SEQ,
        &BytesN::from_array(&s.env, &hex32(fx::PAYMENT_TX_HASH)),
        &fx::PAYMENT_RESULT_CODE,
        &fx::PAYMENT_INDEX,
        &Bytes::from_slice(&s.env, &[0u8; 32]),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn verification_requires_committed_results() {
    let s = Setup::new();
    s.checkpoint();
    s.client.verify_transaction(
        &fx::LEDGER_SEQ,
        &BytesN::from_array(&s.env, &hex32(fx::PAYMENT_TX_HASH)),
        &0,
        &fx::PAYMENT_INDEX,
        &proof_for(&s.env, fx::PAYMENT_INDEX),
    );
}

#[test]
fn verifies_and_decodes_mainnet_payment_envelope() {
    let s = Setup::new();
    s.committed();
    let v = s.client.verify_envelope(
        &fx::LEDGER_SEQ,
        &bytes(&s.env, fx::PAYMENT_ENVELOPE_XDR),
        &fx::PAYMENT_RESULT_CODE,
        &fx::PAYMENT_INDEX,
        &proof_for(&s.env, fx::PAYMENT_INDEX),
    );
    assert_eq!(v.inclusion.tx_hash.to_array(), hex32(fx::PAYMENT_TX_HASH));
    assert!(v.inclusion.successful);
    assert!(!v.transaction.fee_bump);
    assert_eq!(v.transaction.fee_source, v.transaction.source);
    assert!(!v.transaction.operations.is_empty());
    for op in v.transaction.operations.iter() {
        let OperationBody::Payment(p) = op.body else {
            panic!("expected payment");
        };
        assert!(p.amount > 0);
        assert!(matches!(
            p.asset,
            Asset::Native | Asset::CreditAlphanum4(_) | Asset::CreditAlphanum12(_)
        ));
    }
}

#[test]
fn verifies_and_decodes_mainnet_fee_bump_envelope() {
    let s = Setup::new();
    s.committed();
    let v = s.client.verify_envelope(
        &fx::LEDGER_SEQ,
        &bytes(&s.env, fx::FEE_BUMP_ENVELOPE_XDR),
        &fx::FEE_BUMP_RESULT_CODE,
        &fx::FEE_BUMP_INDEX,
        &proof_for(&s.env, fx::FEE_BUMP_INDEX),
    );
    assert_eq!(v.inclusion.tx_hash.to_array(), hex32(fx::FEE_BUMP_TX_HASH));
    assert!(v.transaction.fee_bump);
    assert_ne!(v.transaction.fee_source, v.transaction.source);
    assert!(v.transaction.fee > 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn envelope_cannot_borrow_another_transactions_proof() {
    let s = Setup::new();
    s.committed();
    s.client.verify_envelope(
        &fx::LEDGER_SEQ,
        &bytes(&s.env, fx::PAYMENT_ENVELOPE_XDR),
        &fx::FEE_BUMP_RESULT_CODE,
        &fx::FEE_BUMP_INDEX,
        &proof_for(&s.env, fx::FEE_BUMP_INDEX),
    );
}

// ---------------------------------------------------------------------------
// Administration
// ---------------------------------------------------------------------------

#[test]
fn admin_functions_require_admin_auth() {
    let s = Setup::new();
    let who = Address::generate(&s.env);
    s.client.set_relayer(&who, &true);
    assert_eq!(s.env.auths()[0].0, s.admin);
    assert!(s.client.is_relayer(&who));
    s.client.set_relayer(&who, &false);
    assert!(!s.client.is_relayer(&who));

    let next = Address::generate(&s.env);
    s.client.set_admin(&next);
    assert_eq!(s.client.admin(), next);
}

#[test]
fn admin_functions_reject_unauthorised_callers() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let client = StateVerifierClient::new(
        &env,
        &env.register(StateVerifier, (admin, mainnet_network_id(&env))),
    );
    let who = Address::generate(&env);
    assert!(client.try_set_relayer(&who, &true).is_err());
    assert!(client
        .try_set_validator(&BytesN::from_array(&env, &[1; 32]), &true)
        .is_err());
    assert!(client.try_set_admin(&who).is_err());
}
