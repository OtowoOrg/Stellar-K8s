//! Gas profile of the compiled contract against real mainnet data.
//!
//! Runs the release WASM (`make build`) inside the Soroban VM so that VM
//! instantiation and every WASM instruction are metered, then checks each
//! entry point against the network's per-transaction limits.
//!
//! Print the table with `cargo test --test gas_profile -- --nocapture`.

use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env};
use state_verifier::{merkle, xdr_parser};

/// Shared with the unit tests; not every constant is used here.
#[allow(dead_code)]
mod fx {
    include!("../src/fixtures.rs");
}

mod contract {
    soroban_sdk::contractimport!(file = "target/wasm32v1-none/release/state_verifier.wasm");
}

/// Mainnet per-transaction limits (Soroban network settings).
const TX_MAX_INSTRUCTIONS: i64 = 100_000_000;
const TX_MAX_MEMORY_BYTES: i64 = 41_943_040;
const TX_MAX_SIZE_BYTES: usize = 132_096;

/// Largest result set the worst-case test submits: the transaction size
/// limit minus headroom for the rest of the invocation envelope.
const WORST_CASE_RESULTS_BYTES: usize = 128 * 1024;

const MAINNET_PASSPHRASE: &[u8] = b"Public Global Stellar Network ; September 2015";

/// Regression ceilings for the hot verification paths. These sit far below
/// the network limit so that a calling contract keeps most of its budget.
const VERIFY_TRANSACTION_CEILING: i64 = 2_000_000;
const VERIFY_ENVELOPE_CEILING: i64 = 4_000_000;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

struct Profile {
    rows: Vec<(&'static str, i64, i64)>,
}

impl Profile {
    fn record(&mut self, env: &Env, name: &'static str) -> i64 {
        let r = env.cost_estimate().resources();
        assert!(
            r.instructions <= TX_MAX_INSTRUCTIONS,
            "{name}: {} instructions exceeds the network limit",
            r.instructions
        );
        assert!(
            r.mem_bytes <= TX_MAX_MEMORY_BYTES,
            "{name}: {} bytes exceeds the network memory limit",
            r.mem_bytes
        );
        self.rows.push((name, r.instructions, r.mem_bytes));
        r.instructions
    }

    fn print(&self) {
        println!(
            "\n{:<22} {:>14} {:>9} {:>14}",
            "entry point", "instructions", "% limit", "memory bytes"
        );
        for (name, insns, mem) in &self.rows {
            let pct = *insns as f64 * 100.0 / TX_MAX_INSTRUCTIONS as f64;
            println!("{name:<22} {insns:>14} {pct:>8.2}% {mem:>14}");
        }
    }
}

#[test]
fn mainnet_ledger_verification_fits_network_limits() {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);
    let network_id: BytesN<32> = env
        .crypto()
        .sha256(&Bytes::from_slice(&env, MAINNET_PASSPHRASE))
        .into();
    let id = env.register(contract::WASM, (admin, network_id));
    let client = contract::Client::new(&env, &id);

    let header = hex(fx::HEADER_XDR);
    let node = xdr_parser::parse_ledger_header(&header)
        .unwrap()
        .value_signature
        .unwrap()
        .node_id;
    client.set_relayer(&relayer, &true);
    client.set_validator(&BytesN::from_array(&env, &node), &true);

    let results = hex(fx::RESULTS_XDR);
    assert!(
        results.len() < TX_MAX_SIZE_BYTES,
        "result set exceeds tx size limit"
    );

    let mut leaves = Vec::new();
    xdr_parser::parse_result_set(&results, |_, h, c| {
        leaves.push(merkle::leaf_hash(&env, h, c))
    })
    .unwrap();
    let proof = |i: u32| Bytes::from_slice(&env, &merkle::proof(&env, &leaves, i as usize));
    let payment_proof = proof(fx::PAYMENT_INDEX);
    let bump_proof = proof(fx::FEE_BUMP_INDEX);

    let mut p = Profile { rows: Vec::new() };

    client.checkpoint_ledger(&relayer, &Bytes::from_slice(&env, &header));
    p.record(&env, "checkpoint_ledger");

    client.prove_ancestor(&Bytes::from_slice(&env, &hex(fx::PARENT_HEADER_XDR)));
    p.record(&env, "prove_ancestor");

    let commitment = client.commit_results(&fx::LEDGER_SEQ, &Bytes::from_slice(&env, &results));
    p.record(&env, "commit_results");
    assert_eq!(commitment.tx_count, fx::TX_COUNT);

    let v = client.verify_transaction(
        &fx::LEDGER_SEQ,
        &BytesN::from_array(&env, &hex(fx::PAYMENT_TX_HASH).try_into().unwrap()),
        &fx::PAYMENT_RESULT_CODE,
        &fx::PAYMENT_INDEX,
        &payment_proof,
    );
    assert!(v.successful);
    let verify_tx = p.record(&env, "verify_transaction");

    let v = client.verify_envelope(
        &fx::LEDGER_SEQ,
        &Bytes::from_slice(&env, &hex(fx::PAYMENT_ENVELOPE_XDR)),
        &fx::PAYMENT_RESULT_CODE,
        &fx::PAYMENT_INDEX,
        &payment_proof,
    );
    assert!(v.inclusion.successful);
    let verify_payment = p.record(&env, "verify_envelope (pay)");

    let v = client.verify_envelope(
        &fx::LEDGER_SEQ,
        &Bytes::from_slice(&env, &hex(fx::FEE_BUMP_ENVELOPE_XDR)),
        &fx::FEE_BUMP_RESULT_CODE,
        &fx::FEE_BUMP_INDEX,
        &bump_proof,
    );
    assert!(v.transaction.fee_bump);
    let verify_bump = p.record(&env, "verify_envelope (bump)");

    p.print();
    assert!(
        verify_tx <= VERIFY_TRANSACTION_CEILING,
        "verify_transaction regressed: {verify_tx}"
    );
    assert!(
        verify_payment <= VERIFY_ENVELOPE_CEILING,
        "verify_envelope regressed: {verify_payment}"
    );
    assert!(
        verify_bump <= VERIFY_ENVELOPE_CEILING,
        "verify_envelope regressed: {verify_bump}"
    );
}

/// Worst case: a result set as large as a transaction can carry, committed
/// against a header signed by a test validator.
#[test]
fn maximum_size_result_set_fits_network_limits() {
    let env = Env::default();
    env.mock_all_auths();
    env.cost_estimate().budget().reset_unlimited();

    // ~128 KB of real mainnet result pairs, cycled.
    let fixture = hex(fx::RESULTS_XDR);
    let pairs = xdr_parser::split_result_set(&fixture).unwrap();
    let mut body = Vec::new();
    let mut count = 0u32;
    for pair in pairs.iter().cycle() {
        if 4 + body.len() + pair.len() > WORST_CASE_RESULTS_BYTES {
            break;
        }
        body.extend_from_slice(pair);
        count += 1;
    }
    let mut results = count.to_be_bytes().to_vec();
    results.extend_from_slice(&body);
    assert!(results.len() < TX_MAX_SIZE_BYTES);

    // Re-sign the fixture header with a test validator, committing to the
    // synthetic result set.
    let signer = SigningKey::from_bytes(&[7u8; 32]);
    let network_id: [u8; 32] = env
        .crypto()
        .sha256(&Bytes::from_slice(&env, MAINNET_PASSPHRASE))
        .to_array();
    let mut header = hex(fx::HEADER_XDR);
    assert_eq!(
        &header[76..80],
        &[0, 0, 0, 0],
        "fixture must have no upgrades"
    );
    header[88..120].copy_from_slice(signer.verifying_key().as_bytes());
    let mut payload = network_id.to_vec();
    payload.extend_from_slice(&4i32.to_be_bytes());
    payload.extend_from_slice(&header[36..76]); // txSetHash || closeTime
    header[124..188].copy_from_slice(&signer.sign(&payload).to_bytes());
    let results_hash = env
        .crypto()
        .sha256(&Bytes::from_slice(&env, &results))
        .to_array();
    header[188..220].copy_from_slice(&results_hash);

    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);
    let id = env.register(
        contract::WASM,
        (admin, BytesN::from_array(&env, &network_id)),
    );
    let client = contract::Client::new(&env, &id);
    client.set_relayer(&relayer, &true);
    client.set_validator(
        &BytesN::from_array(&env, signer.verifying_key().as_bytes()),
        &true,
    );
    client.checkpoint_ledger(&relayer, &Bytes::from_slice(&env, &header));

    let mut p = Profile { rows: Vec::new() };
    let commitment = client.commit_results(&fx::LEDGER_SEQ, &Bytes::from_slice(&env, &results));
    assert_eq!(commitment.tx_count, count);
    p.record(&env, "commit_results (max)");

    // Proof depth grows with the set, so verification is profiled too.
    let mut leaves = Vec::new();
    let mut last = ([0u8; 32], 0i32);
    xdr_parser::parse_result_set(&results, |_, h, c| {
        leaves.push(merkle::leaf_hash(&env, h, c));
        last = (*h, c);
    })
    .unwrap();
    let index = count - 1;
    let proof = Bytes::from_slice(&env, &merkle::proof(&env, &leaves, index as usize));
    client.verify_transaction(
        &fx::LEDGER_SEQ,
        &BytesN::from_array(&env, &last.0),
        &last.1,
        &index,
        &proof,
    );
    let verify = p.record(&env, "verify_transaction (max)");
    p.print();
    println!(
        "({count} transactions, {} bytes, depth {})",
        results.len(),
        commitment.depth
    );
    assert!(
        verify <= VERIFY_TRANSACTION_CEILING,
        "verify_transaction regressed: {verify}"
    );
}
