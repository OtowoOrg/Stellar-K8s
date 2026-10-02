//! End-to-end test: a 100 000-recipient distribution built **off-chain**, claimed
//! **on-chain**.
//!
//! This is the test the issue asks for, and it is also the only place where the two
//! halves of the system meet. `src/claim.rs` owns the byte format and the contract
//! verifies against it with the host's SHA-256; `src/offchain.rs` builds trees with a
//! native SHA-256. Nothing but this test proves those two agree — and a Merkle airdrop
//! that ships when they disagree is a distribution nobody can claim, or worse, one
//! where a hand-rolled second implementation lets the wrong people claim.

#[path = "../src/offchain.rs"]
mod offchain;

use merkle_airdrop::claim;
use merkle_airdrop::{Error, MerkleAirdropContract, MerkleAirdropContractClient};
use offchain::{build_tree, from_hex, recipient_xdr, synthetic_account, to_hex, Tree};
use soroban_sdk::testutils::{Address as _, EnvTestConfig, Ledger};
use soroban_sdk::{
    token::{Client as TokenClient, StellarAssetClient},
    xdr::ToXdr,
    Address, BytesN, Env, Vec as SdkVec,
};
use std::vec::Vec;

/// The recipient count from the issue: 100 000 users.
const RECIPIENTS: usize = 100_000;

/// 100 000 pads up to 131 072 leaves, so every proof is 17 siblings long.
const EXPECTED_DEPTH: u32 = 17;

/// Allocation of recipient `i`, in stroops (7 decimals). Varied so that a mismatch
/// anywhere in the amount encoding shows up as a failed proof rather than a coincidence.
fn allocation(index: usize) -> i128 {
    ((index as i128 * 7919) % 1_000 + 1) * 10_000_000
}

/// Deterministic recipient ordering, so failures are reproducible — spread from the
/// very first leaf to the very last so that every branch of the tree geometry is hit.
fn recipient_indices() -> [usize; 8] {
    [0, 1, 16, 17, 4_096, 65_536, RECIPIENTS - 2, RECIPIENTS - 1]
}

/// A slot nobody in [`recipient_indices`] occupies: used to prove that the same
/// allocation cannot simply be re-presented under a different index.
const ELSEWHERE: u32 = 50_001;

/// The recipients we actually claim for need a real `Address`, because the contract
/// re-derives the account encoding from the caller. Everyone else in the tree is
/// described by an equivalent raw encoding — same bytes, no host object — which is what
/// keeps building a 131 072-leaf tree cheap.
struct Distribution {
    tree: Tree,
    amounts: Vec<i128>,
    claimed: Vec<(usize, Address)>,
}

fn build_distribution(env: &Env) -> Distribution {
    let mut encodings: Vec<Vec<u8>> = (0..RECIPIENTS)
        .map(|i| synthetic_account(i as u32, 0x5EED))
        .collect();
    let amounts: Vec<i128> = (0..RECIPIENTS).map(allocation).collect();

    let mut claimed = Vec::new();
    for index in recipient_indices() {
        let account = Address::generate(env);
        // Account addresses serialize to 44 bytes, contract addresses to 40. Using a
        // real (contract-shaped) address here means the tree deliberately mixes both
        // widths, exercising the variable-length tail of the leaf encoding that a
        // real airdrop would hit whenever a contract address is a recipient.
        encodings[index] = account.clone().to_xdr(env).to_alloc_vec();
        claimed.push((index, account));
    }

    let tree = build_tree(&encodings, &amounts);

    Distribution {
        tree,
        amounts,
        claimed,
    }
}

struct TestWorld {
    env: Env,
    contract: Address,
    token: Address,
    admin: Address,
    distribution: Distribution,
    total: i128,
    claim_deadline: u64,
}

impl TestWorld {
    /// A distribution that never expires.
    fn new() -> TestWorld {
        TestWorld::with_deadline(0)
    }

    fn with_deadline(claim_deadline: u64) -> TestWorld {
        let env = Env::new_with_config(EnvTestConfig {
            capture_snapshot_at_drop: false,
        });
        env.mock_all_auths();

        let distribution = build_distribution(&env);
        let total: i128 = distribution.amounts.iter().sum();

        let admin = Address::generate(&env);
        let token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let contract = env.register(MerkleAirdropContract, ());

        let world = TestWorld {
            env,
            contract,
            token,
            admin,
            distribution,
            total,
            claim_deadline,
        };
        world.fund();
        world
    }

    fn airdrop(&self) -> MerkleAirdropContractClient<'_> {
        MerkleAirdropContractClient::new(&self.env, &self.contract)
    }

    fn token_client(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token)
    }

    /// Commit the off-chain root and fund the distributor.
    fn fund(&self) {
        self.airdrop().initialize(
            &self.admin,
            &self.token,
            &self.root(),
            &(RECIPIENTS as u32),
            &self.total,
            &self.claim_deadline,
        );
        StellarAssetClient::new(&self.env, &self.token).mint(&self.contract, &self.total);
    }

    fn root(&self) -> BytesN<32> {
        BytesN::from_array(&self.env, &self.distribution.tree.root())
    }

    /// Convert an off-chain proof into the calldata form.
    fn proof(&self, index: usize) -> SdkVec<BytesN<32>> {
        let mut proof = SdkVec::new(&self.env);
        for sibling in self.distribution.tree.proof(index) {
            proof.push_back(BytesN::from_array(&self.env, &sibling));
        }
        proof
    }
}

/// The single most important property in the whole contract: the tree the generator
/// builds is the tree the verifier accepts.
///
/// If this fails, every other test in this file is meaningless, so it is asserted both
/// ways — the digests agree exactly, and the root the generator computes is the root the
/// contract stored.
#[test]
fn offchain_and_onchain_hashing_agree() {
    let env = Env::new_with_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    });
    env.mock_all_auths();

    let account = Address::generate(&env);
    let encoding = account.clone().to_xdr(&env).to_alloc_vec();
    let amount = allocation(7);

    // Native SHA-256 (off-chain) versus the host's SHA-256 (on-chain), over the same
    // preimage built by the same encoder.
    let offchain_digest = offchain::leaf_digest(7, amount, &encoding);
    let onchain_digest = claim::hash_leaf(&env, 7, amount, &account).to_array();
    assert_eq!(
        to_hex(&offchain_digest),
        to_hex(&onchain_digest),
        "off-chain and on-chain leaf hashing must agree byte for byte"
    );

    // And the fold, too: two children must produce the same parent on both sides.
    let parent_offchain = offchain::parent_digest(&[1u8; 32], &[2u8; 32]);
    let parent_onchain = claim::hash_node(&env, &[1u8; 32], &[2u8; 32]).to_array();
    assert_eq!(to_hex(&parent_offchain), to_hex(&parent_onchain));

    // The documented encodings really are the widths the contract asserts.
    assert_eq!(encoding.len(), 40, "a generated address is contract-shaped");
    assert_eq!(offchain::account_xdr(&[0u8; 32]).len(), 44);
}

/// A pinned root for a small deterministic distribution.
///
/// The leaf encoding is a wire format that off-chain tooling reproduces by hand, so an
/// accidental change to a tag byte, a field order or an endianness must not be able to slip
/// through review quietly. This fails the moment the bytes change — which is exactly the
/// warning an operator needs *before* publishing a root nobody can claim against. The same
/// fixture is printed by `cargo run --example generate_tree -- 1000 7`.
#[test]
fn the_encoding_is_pinned_by_a_golden_root() {
    const COUNT: u32 = 1_000;
    const SALT: u32 = 0xCA_11;

    let accounts: Vec<Vec<u8>> = (0..COUNT).map(|i| recipient_xdr(i, SALT)).collect();
    let amounts = vec![10_000_000i128; COUNT as usize];
    let tree = build_tree(&accounts, &amounts);

    assert_eq!(tree.depth(), 10);
    assert_eq!(tree.leaves.len(), 1_024);

    let root = to_hex(&tree.root());
    assert_eq!(
        root, "575c2fb5ff91209f0eb8016165bad1d8b52e3560947b38f437e16a33c68f9781",
        "the canonical leaf/node encoding changed; a live campaign would need a new root"
    );

    // The hex helpers must round-trip, since proofs are handed to recipients as text.
    assert_eq!(from_hex(&root), tree.root());
    assert_eq!(to_hex(&from_hex(&to_hex(&tree.root()))), root);
}

#[test]
fn a_100_000_recipient_tree_is_committed_and_decoded_correctly() {
    let world = TestWorld::new();
    let airdrop = world.airdrop();

    assert_eq!(world.distribution.tree.leaves.len(), 131_072);
    assert_eq!(world.distribution.tree.depth(), EXPECTED_DEPTH);
    assert_eq!(airdrop.merkle_root(), world.root());
    assert_eq!(airdrop.leaf_count(), RECIPIENTS as u32);
    assert_eq!(airdrop.required_proof_depth(), EXPECTED_DEPTH);
    assert_eq!(airdrop.total_allocated(), world.total);
    assert_eq!(world.token_client().balance(&world.contract), world.total);
}

#[test]
fn valid_claims_pay_the_committed_allocation() {
    let world = TestWorld::new();
    let airdrop = world.airdrop();

    let mut paid = 0i128;
    for &(index, ref account) in &world.distribution.claimed {
        let amount = world.distribution.amounts[index];
        assert_eq!(world.proof(index).len(), EXPECTED_DEPTH);

        airdrop.claim(account, &(index as u32), &amount, &world.proof(index));

        assert!(airdrop.is_claimed(&(index as u32)), "index {index}");
        assert_eq!(
            world.token_client().balance(account),
            amount,
            "index {index}"
        );
        paid += amount;
    }

    let (total_claimed, claim_count) = airdrop.progress();
    assert_eq!(total_claimed, paid);
    assert_eq!(claim_count, world.distribution.claimed.len() as u32);
    // The distributor keeps exactly what was not claimed.
    assert_eq!(
        world.token_client().balance(&world.contract),
        world.total - paid
    );
}

#[test]
fn neighbouring_allocation_slots_are_not_collateral_damage() {
    let world = TestWorld::new();
    let airdrop = world.airdrop();

    // Claim 0, 1 and 17, then check that their neighbours in the same bitmap bucket
    // (and the sibling leaf at 18) are untouched.
    for &(index, ref account) in &world.distribution.claimed {
        if index <= 17 {
            airdrop.claim(
                account,
                &(index as u32),
                &world.distribution.amounts[index],
                &world.proof(index),
            );
        }
    }

    let expected_claimed = [0u32, 1, 16, 17];
    for index in [0u32, 1, 2, 15, 16, 17, 18] {
        assert_eq!(
            airdrop.is_claimed(&index),
            expected_claimed.contains(&index),
            "index {index}"
        );
    }
}

#[test]
fn a_double_claim_is_rejected_for_every_recipient() {
    let world = TestWorld::new();
    let airdrop = world.airdrop();

    for &(index, ref account) in &world.distribution.claimed {
        let amount = world.distribution.amounts[index];
        let proof = world.proof(index);

        airdrop.claim(account, &(index as u32), &amount, &proof);

        // Same proof, same slot.
        let replay = airdrop.try_claim(account, &(index as u32), &amount, &proof);
        assert_eq!(replay.unwrap_err().unwrap(), Error::AlreadyClaimed);

        // Same allocation, moved to an unclaimed slot: the index is inside the leaf, so
        // this is an invalid proof rather than a second payout.
        let moved = airdrop.try_claim(account, &ELSEWHERE, &amount, &proof);
        assert_eq!(moved.unwrap_err().unwrap(), Error::InvalidProof);

        assert_eq!(
            world.token_client().balance(account),
            amount,
            "index {index}"
        );
    }

    let (_, claim_count) = airdrop.progress();
    assert_eq!(claim_count, world.distribution.claimed.len() as u32);
    // The failed attempts to move allocations elsewhere left no marks behind.
    assert!(!airdrop.is_claimed(&ELSEWHERE));
}

#[test]
fn forged_proofs_are_rejected() {
    let world = TestWorld::new();
    let airdrop = world.airdrop();
    let (index, ref account) = world.distribution.claimed[3];
    let amount = world.distribution.amounts[index];

    // A tampered sibling somewhere in a 17-level path.
    let mut forged = world.proof(index);
    forged.set(9, BytesN::from_array(&world.env, &[0x5Au8; 32]));
    assert_eq!(
        airdrop
            .try_claim(account, &(index as u32), &amount, &forged)
            .unwrap_err()
            .unwrap(),
        Error::InvalidProof
    );

    // A truncated path.
    let mut short = SdkVec::new(&world.env);
    for i in 0..(EXPECTED_DEPTH - 1) {
        short.push_back(forged.get(i).unwrap());
    }
    assert_eq!(
        airdrop
            .try_claim(account, &(index as u32), &amount, &short)
            .unwrap_err()
            .unwrap(),
        Error::InvalidProof
    );

    // A proof that is one level too deep for this tree.
    let mut long = world.proof(index);
    long.push_back(BytesN::from_array(&world.env, &[0u8; 32]));
    assert_eq!(
        airdrop
            .try_claim(account, &(index as u32), &amount, &long)
            .unwrap_err()
            .unwrap(),
        Error::InvalidProof
    );

    // An inflated amount cannot be smuggled past the commitment.
    assert_eq!(
        airdrop
            .try_claim(account, &(index as u32), &(amount * 2), &world.proof(index))
            .unwrap_err()
            .unwrap(),
        Error::InvalidProof
    );

    // Nothing was paid and nothing was marked.
    assert_eq!(world.token_client().balance(account), 0);
    assert!(!airdrop.is_claimed(&(index as u32)));
    assert_eq!(airdrop.progress(), (0, 0));
}

#[test]
fn an_address_absent_from_the_tree_cannot_claim() {
    let world = TestWorld::new();
    let airdrop = world.airdrop();
    let outsider = Address::generate(&world.env);

    // A perfectly valid proof, presented by the wrong address: the leaf commits to the
    // recipient, so possession of somebody else's proof is worthless.
    let (index, _) = world.distribution.claimed[0];
    assert_eq!(
        airdrop
            .try_claim(
                &outsider,
                &(index as u32),
                &world.distribution.amounts[index],
                &world.proof(index),
            )
            .unwrap_err()
            .unwrap(),
        Error::InvalidProof
    );
    assert_eq!(world.token_client().balance(&outsider), 0);
}

#[test]
fn the_padded_tail_of_the_tree_is_unclaimable() {
    let world = TestWorld::new();
    let airdrop = world.airdrop();

    // Indices 100 000..131 072 are padding. They are outside `leaf_count`, and their
    // leaves are the all-zero digest, so they are refused before any hashing matters.
    for index in [RECIPIENTS, RECIPIENTS + 1, 131_071] {
        let outsider = Address::generate(&world.env);
        assert_eq!(
            airdrop
                .try_claim(&outsider, &(index as u32), &10_000_000, &world.proof(index))
                .unwrap_err()
                .unwrap(),
            Error::IndexOutOfRange,
            "index {index}"
        );
    }
}

#[test]
fn a_perpetual_distribution_can_never_be_swept() {
    let world = TestWorld::new();
    let airdrop = world.airdrop();
    let treasury = Address::generate(&world.env);

    let (index, ref account) = world.distribution.claimed[5];
    let amount = world.distribution.amounts[index];
    airdrop.claim(account, &(index as u32), &amount, &world.proof(index));

    // No deadline was set, so the sweeper must stay closed forever — even at the end of
    // time — and the unclaimed balance stays claimable.
    world.env.ledger().set_timestamp(u64::from(u32::MAX));
    assert_eq!(
        airdrop
            .try_clawback(&world.admin, &treasury)
            .unwrap_err()
            .unwrap(),
        Error::DeadlineNotSet
    );
    assert_eq!(airdrop.progress().1, 1);
    assert_eq!(
        world.token_client().balance(&world.contract),
        world.total - amount
    );
}

#[test]
fn a_deadlined_distribution_stops_claims_and_sweeps_the_remainder() {
    let world = TestWorld::with_deadline(1_000);
    let airdrop = world.airdrop();
    let treasury = Address::generate(&world.env);

    let (index, ref account) = world.distribution.claimed[5];
    let amount = world.distribution.amounts[index];

    // One second inside the window.
    world.env.ledger().set_timestamp(999);
    airdrop.claim(account, &(index as u32), &amount, &world.proof(index));

    // The window is half-open: claims are refused from the deadline onward.
    world.env.ledger().set_timestamp(1_000);
    let (late_index, ref late_account) = world.distribution.claimed[6];
    assert_eq!(
        airdrop
            .try_claim(
                late_account,
                &(late_index as u32),
                &world.distribution.amounts[late_index],
                &world.proof(late_index),
            )
            .unwrap_err()
            .unwrap(),
        Error::ClaimWindowClosed
    );

    // Only the unclaimed remainder is swept; the recipient who made it keeps theirs.
    let swept = airdrop.clawback(&world.admin, &treasury);
    assert_eq!(swept, world.total - amount);
    assert_eq!(
        world.token_client().balance(&treasury),
        world.total - amount
    );
    assert_eq!(world.token_client().balance(&world.contract), 0);
    assert_eq!(world.token_client().balance(account), amount);
}
