//! Instruction-cost benchmark for `MerkleAirdropContract::claim`.
//!
//! The issue requires benchmarks measuring instruction usage for proofs up to 20 levels
//! deep, because "highly optimised verification" is the whole point of a pull-based
//! distributor: the distributor never pays per recipient, but every *claimant* does, and a
//! verification that grows faster than the tree does — or that quietly exceeds the
//! network's instruction ceiling — turns a "free" airdrop into an unclaimable one.
//!
//! # What is measured
//!
//! A real `claim()` at every depth from 0 (single leaf, empty proof) to
//! [`MAX_PROOF_DEPTH`] = 20 (2^20 = 1 048 576 padded leaves), using a proof extracted from
//! a real off-chain tree, paid out by a real Stellar Asset Contract. Nothing is stubbed,
//! so the numbers include the storage reads and writes, the token transfer and the event —
//! not just the hashing.
//!
//! Three things make the numbers trustworthy, and all three were arrived at by measurement
//! rather than assumption:
//!
//! * **The fixture is built off-chain and is environment-independent.** Recipients are
//!   derived arithmetically (see `recipient_payload`), so a depth's tree and proof are
//!   hashed once and then replayed verbatim in a fresh `Env`.
//! * **Each measurement gets a fresh `Env`, and the cheapest repeat wins.** The test host
//!   caches host objects across invocations, so measuring 21 depths inside one `Env` mixes
//!   cache state into the result and yields numbers that are not even monotonic in depth.
//! * **The meters are read before any other call.** They describe the last top-level
//!   invocation, so a stray `is_claimed` afterwards would replace a claim's figures with a
//!   trivial view's — which, silently, makes every depth look identical.
//!
//! # Reading the table
//!
//! * **cpu/level** is `(cpu(d) − cpu(0)) / d`. Everything except the number of hashes is
//!   identical at every depth (same storage slots, same token, same event), so the
//!   difference isolates the cost of one additional Merkle level. A flat column is the
//!   evidence that verification is linear in depth — that is, `O(log n)` in recipients.
//! * **invocation instrs** is the host's separate invocation-level meter. It models the
//!   bookkeeping of a *Wasm* invocation, which a test `Env` does not perform (contracts run
//!   as native Rust here), so it is reported for completeness rather than asserted on.
//!
//! Run with `cargo bench`, or as a test with `cargo test --bench claim_bench`. The binary
//! asserts, so it fails the build rather than printing a regression.

use merkle_airdrop::claim::{MAX_LEAF_COUNT, MAX_PROOF_DEPTH, NODE_LEN};
use merkle_airdrop::{MerkleAirdropContract, MerkleAirdropContractClient};
use soroban_sdk::address_payload::AddressPayload;
use soroban_sdk::testutils::{Address as _, EnvTestConfig};
use soroban_sdk::{
    token::{Client as TokenClient, StellarAssetClient},
    Address, BytesN, Env, Vec as SdkVec,
};

#[path = "../src/offchain.rs"]
mod offchain;

use offchain::{build_tree, contract_xdr, recipient_payload, recipient_xdr};

/// One whole token per recipient, in stroops.
const ALLOCATION: i128 = 10_000_000;

/// Salt keeping these fixtures distinct from the integration test's.
const SALT: u32 = 0xB0_1CE;

/// The instruction ceiling a single invocation may consume on Stellar mainnet.
const MAINNET_INSTRUCTION_LIMIT: u64 = 400_000_000;

/// Identical runs per depth; the cheapest is kept so one-off noise cannot inflate a row.
const REPEATS: usize = 5;

/// A distribution of a given depth, built entirely off-chain and therefore reproducible in
/// any `Env`.
struct Fixture {
    depth: u32,
    leaf_count: u32,
    root: [u8; 32],
    proof: Vec<[u8; 32]>,
    /// Contract-id payload of the recipient at leaf 0 — the one that claims.
    recipient: [u8; 32],
}

impl Fixture {
    fn build(depth: u32) -> Fixture {
        let leaf_count = 1u32 << depth;
        assert!(
            leaf_count <= MAX_LEAF_COUNT,
            "depth {depth} exceeds the supported distribution size"
        );

        let recipient = recipient_payload(0, SALT);
        let mut accounts: Vec<Vec<u8>> = (0..leaf_count).map(|i| recipient_xdr(i, SALT)).collect();
        accounts[0] = contract_xdr(&recipient);
        let amounts = vec![ALLOCATION; leaf_count as usize];

        let tree = build_tree(&accounts, &amounts);
        assert_eq!(tree.depth(), depth);

        let proof = tree.proof(0);
        assert_eq!(proof.len() as u32, depth);

        Fixture {
            depth,
            leaf_count,
            root: tree.root(),
            proof,
            recipient,
        }
    }
}

#[derive(Clone, Copy)]
struct Measurement {
    cpu_instructions: u64,
    mem_bytes: u64,
    invocation_instructions: i64,
    fee_stroops: i64,
}

fn main() {
    let mut rows: Vec<(Fixture, Measurement)> = Vec::new();

    for depth in 0..=MAX_PROOF_DEPTH {
        let fixture = Fixture::build(depth);
        let measurement = (0..REPEATS)
            .map(|_| measure(&fixture))
            .min_by_key(|m| m.cpu_instructions)
            .expect("REPEATS > 0");
        rows.push((fixture, measurement));
    }

    report(&rows);
    assert_scaling(&rows);
}

/// Run one complete claim against a fresh `Env` and report what it cost.
fn measure(fixture: &Fixture) -> Measurement {
    let env = fresh_env();

    let admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract = env.register(MerkleAirdropContract, ());
    let client = MerkleAirdropContractClient::new(&env, &contract);

    client.initialize(
        &admin,
        &token,
        &BytesN::from_array(&env, &fixture.root),
        &fixture.leaf_count,
        &ALLOCATION,
        &0,
    );
    StellarAssetClient::new(&env, &token).mint(&contract, &ALLOCATION);

    // The claimant is the payout recipient itself, rebuilt from its 32-byte payload — the
    // tree's leaf 0 is `contract_xdr(recipient)`, so the proof only verifies if the
    // contract reconstructs exactly those bytes from this `Address`.
    let claimant = AddressPayload::ContractIdHash(BytesN::from_array(&env, &fixture.recipient))
        .to_address(&env);

    let mut proof = SdkVec::new(&env);
    for sibling in &fixture.proof {
        proof.push_back(BytesN::from_array(&env, sibling));
    }

    // The measured invocation. Resource-limit enforcement is left at its default
    // (`mainnet()`), so a depth that could not be paid for on-chain would abort here.
    client.claim(&claimant, &0, &ALLOCATION, &proof);

    // Both meters describe the **last** top-level invocation, and they are reset at the
    // start of the next one. So they have to be read before anything else is called — even
    // a view like `is_claimed` would replace the claim's figures with its own.
    let budget = env.cost_estimate().budget();
    let cpu_instructions = budget.cpu_instruction_cost();
    let mem_bytes = budget.memory_bytes_cost();
    let invocation_instructions = env.cost_estimate().resources().instructions;
    let fee_stroops = env.cost_estimate().fee().total;

    assert!(
        client.is_claimed(&0),
        "the measured claim must have succeeded"
    );
    assert_eq!(
        TokenClient::new(&env, &token).balance(&claimant),
        ALLOCATION,
        "the measured claim must have been paid"
    );

    Measurement {
        cpu_instructions,
        mem_bytes,
        invocation_instructions,
        fee_stroops,
    }
}

fn fresh_env() -> Env {
    let env = Env::new_with_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    });
    env.mock_all_auths();
    env
}

fn report(rows: &[(Fixture, Measurement)]) {
    println!(
        "\nMerkle airdrop claim cost by proof depth \
         (1 recipient = 1 token with 7 decimals, one real claim per row)\n"
    );
    println!(
        "{:>5}  {:>6}  {:>11}  {:>12}  {:>10}  {:>10}  {:>10}",
        "depth", "proof", "leaves", "cpu instrs", "cpu/level", "mem bytes", "invocation"
    );
    println!("{}", "-".repeat(78));

    let base = rows[0].1;
    for (fixture, m) in rows {
        let per_level = if fixture.depth == 0 {
            "n/a".to_string()
        } else {
            ((m.cpu_instructions - base.cpu_instructions) / u64::from(fixture.depth)).to_string()
        };
        println!(
            "{:>5}  {:>6}  {:>11}  {:>12}  {:>10}  {:>10}  {:>10}",
            fixture.depth,
            fixture.proof.len(),
            1u32 << fixture.depth,
            m.cpu_instructions,
            per_level,
            m.mem_bytes,
            m.invocation_instructions,
        );
    }

    let deepest = &rows[rows.len() - 1];
    let d = deepest.0.depth;
    let m = deepest.1;
    let marginal = (m.cpu_instructions - base.cpu_instructions) / u64::from(d);

    println!("\nheadline — CPU instructions charged to one claim:");
    println!(
        "  depth  0, 1 leaf, empty proof .......... {}",
        base.cpu_instructions
    );
    println!(
        "  depth 17, 100 000 users (131 072 leaves)  {}",
        rows[17].1.cpu_instructions
    );
    println!(
        "  depth 20, 1 048 576 padded leaves ....... {}",
        m.cpu_instructions
    );
    println!(
        "  marginal cost of one proof level ........ {marginal} instructions \
         (a {NODE_LEN}-byte preimage hashed once)"
    );
    println!(
        "  share of the 400M mainnet ceiling ....... {:.4}%",
        m.cpu_instructions as f64 / MAINNET_INSTRUCTION_LIMIT as f64 * 100.0
    );
    println!(
        "  modelled memory at depth {d} .............. {} bytes",
        m.mem_bytes
    );
    println!(
        "  partial fee estimate .................... {:.7} XLM \
         (the SDK models neither Wasm execution nor transaction size)",
        deepest.1.fee_stroops as f64 / 10_000_000.0
    );
}

/// Fail the build if verification ever stops being linear in depth, or stops fitting
/// inside a mainnet invocation with room to spare.
fn assert_scaling(rows: &[(Fixture, Measurement)]) {
    let base = rows[0].1;

    // Strictly increasing: one more sibling must cost more, or work is being skipped.
    for window in rows.windows(2) {
        assert!(
            window[1].1.cpu_instructions > window[0].1.cpu_instructions,
            "instruction cost must grow with proof depth (depth {} -> {})",
            window[0].0.depth,
            window[1].0.depth,
        );
    }

    // Linear: the marginal cost of a level must not drift as the tree grows. This is the
    // property that makes the worst case predictable, and it is what would break if
    // verification ever became quadratic — for instance by rehashing a growing prefix, or
    // by validating the whole proof before folding it.
    let per_level: Vec<u64> = rows
        .iter()
        .skip(1)
        .map(|(f, m)| (m.cpu_instructions - base.cpu_instructions) / u64::from(f.depth))
        .collect();
    let min = *per_level.iter().min().unwrap();
    let max = *per_level.iter().max().unwrap();
    assert!(min > 0, "a proof level must cost something");
    assert!(
        max - min <= min / 10,
        "cost per proof level must be flat across all depths, saw {min}..{max}"
    );

    // The deepest supported distribution must fit comfortably inside a mainnet invocation.
    let (fixture, deepest) = &rows[rows.len() - 1];
    assert_eq!(fixture.depth, MAX_PROOF_DEPTH);
    assert!(
        deepest.cpu_instructions < MAINNET_INSTRUCTION_LIMIT / 4,
        "a depth-{} claim must fit in a mainnet invocation, used {} of {}",
        fixture.depth,
        deepest.cpu_instructions,
        MAINNET_INSTRUCTION_LIMIT,
    );

    // Absolute budget: hashing 21 nodes must stay a rounding error next to everything else
    // a claim does, so a large airdrop never becomes too expensive for its recipients.
    assert!(
        deepest.cpu_instructions < 1_000_000,
        "a depth-{} claim should stay under 1M charged instructions, used {}",
        fixture.depth,
        deepest.cpu_instructions,
    );
}
