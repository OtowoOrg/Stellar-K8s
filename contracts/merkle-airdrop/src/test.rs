#![cfg(test)]

//! Contract behaviour tests.
//!
//! These build small trees with the contract's *own* hashing so they can exercise the
//! state machine (claims, replay guard, proof rejection, admin surface, claim window)
//! without depending on the std off-chain generator. The generator's agreement with the
//! on-chain verifier at real scale is proven separately in `tests/merkle_airdrop.rs`,
//! which pushes a 100 000-recipient tree through these same entrypoints.

use crate::claim;
use crate::{Claimed, Error, MerkleAirdropContract, MerkleAirdropContractClient};
use soroban_sdk::{
    testutils::{Address as _, EnvTestConfig, Events as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, BytesN, Env, Event as _, InvokeError, Vec as SdkVec,
};
use std::vec;
use std::vec::Vec;

/// One whole token, with 7 decimal places.
const UNIT: i128 = 10_000_000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A deployment of the contract plus a Stellar Asset Contract to distribute.
struct World {
    env: Env,
    contract: Address,
    token: Address,
    admin: Address,
}

fn world() -> World {
    // Snapshot capture is off: these tests assert state directly, and a 100 000-leaf
    // fixture would otherwise write megabytes of JSON into the working tree on every
    // `cargo test` run.
    let env = Env::new_with_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    });
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let contract = env.register(MerkleAirdropContract, ());

    World {
        env,
        contract,
        token,
        admin,
    }
}

impl World {
    fn airdrop(&self) -> MerkleAirdropContractClient<'_> {
        MerkleAirdropContractClient::new(&self.env, &self.contract)
    }

    fn asset(&self) -> StellarAssetClient<'_> {
        StellarAssetClient::new(&self.env, &self.token)
    }

    fn token_client(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token)
    }
}

/// A small balanced Merkle tree, built with the contract's own hashers.
struct Tree {
    levels: Vec<Vec<BytesN<32>>>,
}

impl Tree {
    fn build(env: &Env, allocations: &[(Address, i128)]) -> Tree {
        let padded = claim::padded_leaf_count(allocations.len() as u32) as usize;

        let mut leaves: Vec<BytesN<32>> = Vec::with_capacity(padded);
        for (index, (account, amount)) in allocations.iter().enumerate() {
            leaves.push(claim::hash_leaf(env, index as u32, *amount, account));
        }
        // Padding mirrors the off-chain generator: the all-zero digest.
        for _ in allocations.len()..padded {
            leaves.push(BytesN::from_array(env, &claim::PAD_LEAF));
        }

        let mut levels = Vec::new();
        levels.push(leaves);
        while levels[levels.len() - 1].len() > 1 {
            let current = levels[levels.len() - 1].clone();
            let mut next = Vec::new();
            for pair in current.chunks(2) {
                next.push(claim::hash_node(
                    env,
                    &pair[0].to_array(),
                    &pair[1].to_array(),
                ));
            }
            levels.push(next);
        }

        Tree { levels }
    }

    fn root(&self) -> BytesN<32> {
        self.levels[self.levels.len() - 1][0].clone()
    }

    fn depth(&self) -> u32 {
        (self.levels.len() - 1) as u32
    }

    fn proof(&self, env: &Env, index: usize) -> SdkVec<BytesN<32>> {
        let mut proof = SdkVec::new(env);
        let mut idx = index;
        for level in &self.levels[..self.levels.len() - 1] {
            proof.push_back(level[idx ^ 1].clone());
            idx >>= 1;
        }
        proof
    }
}

/// Assert that a client call failed with the expected contract error.
///
/// The generated `try_*` methods return `Result<Result<T, C>, Result<E, InvokeError>>`,
/// where the middle layer is where a `#[contracterror]` discriminant lands. Telling
/// that layer apart from a host abort is what makes these assertions meaningful rather
/// than just "something went wrong" — a malformed argument or a panicking host function
/// would otherwise pass as "the claim was rejected".
fn assert_error<T: core::fmt::Debug, C: core::fmt::Debug>(
    result: Result<Result<T, C>, Result<Error, InvokeError>>,
    expected: Error,
) {
    match result {
        Ok(Ok(value)) => panic!("expected {expected:?}, but the call succeeded with {value:?}"),
        Ok(Err(undecodable)) => panic!(
            "expected {expected:?}, but the return value could not be decoded: {undecodable:?}"
        ),
        Err(Ok(actual)) => assert_eq!(actual, expected, "wrong contract error"),
        Err(Err(host)) => panic!("expected {expected:?}, but the host raised {host:?}"),
    }
}

/// Total of a set of allocations.
fn total_of(allocations: &[(Address, i128)]) -> i128 {
    allocations.iter().map(|(_, amount)| *amount).sum()
}

/// Deploy a distribution over `allocations`, fund it, and return the tree.
fn deployed(world: &World, allocations: &[(Address, i128)], deadline: u64) -> Tree {
    let tree = Tree::build(&world.env, allocations);
    world.airdrop().initialize(
        &world.admin,
        &world.token,
        &tree.root(),
        &(allocations.len() as u32),
        &total_of(allocations),
        &deadline,
    );
    world.asset().mint(&world.contract, &total_of(allocations));
    tree
}

/// Four recipients with distinct allocations.
fn four_recipients(env: &Env) -> Vec<(Address, i128)> {
    vec![
        (Address::generate(env), 4 * UNIT),
        (Address::generate(env), 3 * UNIT),
        (Address::generate(env), 2 * UNIT),
        (Address::generate(env), UNIT),
    ]
}

/// Five recipients — used to model a corrected tree.
fn five_recipients(env: &Env) -> Vec<(Address, i128)> {
    vec![
        (Address::generate(env), 5 * UNIT),
        (Address::generate(env), 4 * UNIT),
        (Address::generate(env), 3 * UNIT),
        (Address::generate(env), 2 * UNIT),
        (Address::generate(env), UNIT),
    ]
}

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

#[test]
fn initialize_records_the_distribution() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);
    let airdrop = world.airdrop();

    assert_eq!(airdrop.merkle_root(), tree.root());
    assert_eq!(airdrop.leaf_count(), 4);
    assert_eq!(airdrop.total_allocated(), 10 * UNIT);
    assert_eq!(airdrop.claim_deadline(), 0);
    assert_eq!(airdrop.required_proof_depth(), 2);
    assert_eq!(airdrop.progress(), (0, 0));
    assert!(!airdrop.is_paused());
    assert_eq!(airdrop.admin(), world.admin);
    assert_eq!(airdrop.token(), world.token);
}

#[test]
fn initialize_cannot_be_called_twice() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);

    assert_error(
        world.airdrop().try_initialize(
            &world.admin,
            &world.token,
            &tree.root(),
            &4,
            &(10 * UNIT),
            &0,
        ),
        Error::AlreadyInitialized,
    );
}

#[test]
fn initialize_rejects_a_zero_root() {
    let world = world();
    let zero = BytesN::from_array(&world.env, &[0u8; 32]);

    assert_error(
        world
            .airdrop()
            .try_initialize(&world.admin, &world.token, &zero, &4, &(10 * UNIT), &0),
        Error::InvalidMerkleRoot,
    );
}

#[test]
fn initialize_rejects_impossible_leaf_counts() {
    let world = world();
    let root = BytesN::from_array(&world.env, &[9u8; 32]);

    assert_error(
        world
            .airdrop()
            .try_initialize(&world.admin, &world.token, &root, &0, &UNIT, &0),
        Error::InvalidLeafCount,
    );
    assert_error(
        world.airdrop().try_initialize(
            &world.admin,
            &world.token,
            &root,
            &(claim::MAX_LEAF_COUNT + 1),
            &UNIT,
            &0,
        ),
        Error::InvalidLeafCount,
    );
}

#[test]
fn claim_before_initialize_is_not_initialized() {
    let world = world();
    let claimant = Address::generate(&world.env);
    let proof = SdkVec::new(&world.env);

    assert_error(
        world.airdrop().try_claim(&claimant, &0, &UNIT, &proof),
        Error::NotInitialized,
    );
    assert_error(
        world.airdrop().try_set_merkle_root(
            &world.admin,
            &BytesN::from_array(&world.env, &[1u8; 32]),
            &1,
            &UNIT,
        ),
        Error::NotInitialized,
    );
}

// ---------------------------------------------------------------------------
// Successful claims
// ---------------------------------------------------------------------------

#[test]
fn every_allocation_can_be_claimed_exactly_once() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);
    let airdrop = world.airdrop();

    assert_eq!(tree.depth(), 2);

    for (index, (account, amount)) in allocations.iter().enumerate() {
        airdrop.claim(
            account,
            &(index as u32),
            amount,
            &tree.proof(&world.env, index),
        );

        assert!(airdrop.is_claimed(&(index as u32)));
        assert_eq!(world.token_client().balance(account), *amount);
    }

    assert_eq!(airdrop.progress(), (10 * UNIT, 4));
    // The distributor holds nothing extra: payouts are exactly the commitments.
    assert_eq!(world.token_client().balance(&world.contract), 0);
}

#[test]
fn a_claim_emits_the_typed_claim_event() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);

    world.airdrop().claim(
        &allocations[0].0,
        &0,
        &allocations[0].1,
        &tree.proof(&world.env, 0),
    );

    // The event is part of the contract's interface, not an afterthought: an indexer
    // rebuilds the paid-out set from these alone, so pin its exact encoding.
    let expected = Claimed {
        claimant: allocations[0].0.clone(),
        index: 0,
        amount: allocations[0].1,
    }
    .to_xdr(&world.env, &world.contract);

    // The Stellar Asset Contract publishes its own `transfer` event during the payout,
    // so the claim event is asserted as the final one rather than the only one.
    let emitted = world.env.events().all();
    assert_eq!(emitted.events().last(), Some(&expected));
}

#[test]
fn a_claimant_without_the_matching_allocation_cannot_be_paid() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);

    // A genuine proof for recipient 0, presented by somebody else: the leaf commits to
    // the address, so the contract refuses even though the proof itself is valid.
    let impostor = Address::generate(&world.env);
    assert_error(
        world
            .airdrop()
            .try_claim(&impostor, &0, &allocations[0].1, &tree.proof(&world.env, 0)),
        Error::InvalidProof,
    );
    assert_eq!(world.token_client().balance(&impostor), 0);
}

// ---------------------------------------------------------------------------
// Replay / double-claim protection
// ---------------------------------------------------------------------------

#[test]
fn a_second_claim_for_the_same_allocation_is_rejected() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);
    let airdrop = world.airdrop();
    let proof = tree.proof(&world.env, 0);
    let (account, amount) = &allocations[0];

    airdrop.claim(account, &0, amount, &proof);
    assert_eq!(world.token_client().balance(account), *amount);

    // Replay with the identical proof.
    assert_error(
        airdrop.try_claim(account, &0, amount, &proof),
        Error::AlreadyClaimed,
    );
    // And the guard is on the *slot*, not on the presentation: no other proof body
    // helps, because the replay check runs before the proof is even parsed.
    assert_error(
        airdrop.try_claim(account, &0, amount, &SdkVec::new(&world.env)),
        Error::AlreadyClaimed,
    );

    assert_eq!(world.token_client().balance(account), *amount);
    assert_eq!(airdrop.progress(), (*amount, 1));
}

#[test]
fn an_allocation_cannot_be_replayed_under_a_different_index() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);
    let airdrop = world.airdrop();
    let (account, amount) = &allocations[0];

    airdrop.claim(account, &0, amount, &tree.proof(&world.env, 0));

    // The index is hashed into the leaf, so moving the same allocation to a *different*
    // slot produces a different leaf and the proof no longer reconstructs the root.
    assert_error(
        airdrop.try_claim(account, &1, amount, &tree.proof(&world.env, 0)),
        Error::InvalidProof,
    );
    assert_eq!(world.token_client().balance(account), *amount);
}

#[test]
fn the_same_allocation_can_only_be_paid_once_across_many_attempts() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);
    let airdrop = world.airdrop();
    let (account, amount) = &allocations[2];

    airdrop.claim(account, &2, amount, &tree.proof(&world.env, 2));

    for _ in 0..8 {
        let _ = airdrop.try_claim(account, &2, amount, &tree.proof(&world.env, 2));
    }

    assert_eq!(world.token_client().balance(account), *amount);
    assert_eq!(airdrop.progress().1, 1);
}

// ---------------------------------------------------------------------------
// Proof rejection
// ---------------------------------------------------------------------------

#[test]
fn a_tampered_sibling_is_rejected() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);

    let mut proof = tree.proof(&world.env, 0);
    proof.set(0, BytesN::from_array(&world.env, &[0xABu8; 32]));

    assert_error(
        world
            .airdrop()
            .try_claim(&allocations[0].0, &0, &allocations[0].1, &proof),
        Error::InvalidProof,
    );
}

#[test]
fn a_truncated_proof_is_rejected() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let _ = deployed(&world, &allocations, 0);

    assert_error(
        world.airdrop().try_claim(
            &allocations[0].0,
            &0,
            &allocations[0].1,
            &SdkVec::new(&world.env),
        ),
        Error::InvalidProof,
    );
}

#[test]
fn an_inflated_amount_is_rejected() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);

    // The amount is part of the commitment, so a claimant cannot round their payout up.
    assert_error(
        world.airdrop().try_claim(
            &allocations[0].0,
            &0,
            &(allocations[0].1 + UNIT),
            &tree.proof(&world.env, 0),
        ),
        Error::InvalidProof,
    );
    // Nor can they claim a *different* recipient's larger allocation.
    assert_error(
        world.airdrop().try_claim(
            &allocations[0].0,
            &0,
            &allocations[1].1,
            &tree.proof(&world.env, 0),
        ),
        Error::InvalidProof,
    );
}

#[test]
fn an_index_past_the_declared_range_is_rejected() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);

    // `leaf_count` is 4, so index 4 is out of range even for a well-formed proof.
    let mut proof = tree.proof(&world.env, 0);
    proof.push_back(BytesN::from_array(&world.env, &[0u8; 32]));

    assert_error(
        world
            .airdrop()
            .try_claim(&allocations[0].0, &4, &allocations[0].1, &proof),
        Error::IndexOutOfRange,
    );
}

#[test]
fn an_over_deep_proof_is_rejected_before_any_hashing() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let _ = deployed(&world, &allocations, 0);

    let mut too_deep = SdkVec::new(&world.env);
    for _ in 0..=claim::MAX_PROOF_DEPTH {
        too_deep.push_back(BytesN::from_array(&world.env, &[0x11u8; 32]));
    }
    assert_error(
        world
            .airdrop()
            .try_claim(&allocations[0].0, &0, &allocations[0].1, &too_deep),
        Error::ProofTooDeep,
    );

    // Exactly at the cap is structurally allowed (it simply fails to verify), so the
    // bound is inclusive and the benchmark's worst case is reachable.
    let mut at_cap = SdkVec::new(&world.env);
    for _ in 0..claim::MAX_PROOF_DEPTH {
        at_cap.push_back(BytesN::from_array(&world.env, &[0x11u8; 32]));
    }
    assert_error(
        world
            .airdrop()
            .try_claim(&allocations[0].0, &0, &allocations[0].1, &at_cap),
        Error::InvalidProof,
    );
}

#[test]
fn a_non_positive_amount_is_rejected() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);

    assert_error(
        world
            .airdrop()
            .try_claim(&allocations[0].0, &0, &0, &tree.proof(&world.env, 0)),
        Error::InvalidAmount,
    );
}

#[test]
fn a_padded_slot_cannot_be_claimed() {
    let world = world();
    // Three real allocations pad to four leaves; the fourth is the all-zero digest.
    let mut allocations = four_recipients(&world.env);
    allocations.truncate(3);

    let tree = Tree::build(&world.env, &allocations);
    assert_eq!(tree.depth(), 2);
    world.airdrop().initialize(
        &world.admin,
        &world.token,
        &tree.root(),
        &3,
        &total_of(&allocations),
        &0,
    );
    world.asset().mint(&world.contract, &total_of(&allocations));

    // Index 3 is padding, and it is also past `leaf_count`, so it is doubly unavailable.
    let claimant = Address::generate(&world.env);
    assert_error(
        world
            .airdrop()
            .try_claim(&claimant, &3, &UNIT, &tree.proof(&world.env, 3)),
        Error::IndexOutOfRange,
    );

    // Even without the range check the padded leaf is unreachable: the all-zero digest
    // is not the image of any SHA-256 under this encoding, so every candidate address
    // is rejected. Index 2 is real, so this isolates the padding rather than the range.
    assert_error(
        world
            .airdrop()
            .try_claim(&claimant, &2, &UNIT, &tree.proof(&world.env, 2)),
        Error::InvalidProof,
    );

    // The three real recipients are unaffected.
    world.airdrop().claim(
        &allocations[1].0,
        &1,
        &allocations[1].1,
        &tree.proof(&world.env, 1),
    );
    assert_eq!(
        world.token_client().balance(&allocations[1].0),
        allocations[1].1
    );
}

// ---------------------------------------------------------------------------
// Claim bitmap
// ---------------------------------------------------------------------------

#[test]
fn claim_flags_do_not_leak_between_neighbouring_indices() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);
    let airdrop = world.airdrop();

    airdrop.claim(
        &allocations[2].0,
        &2,
        &allocations[2].1,
        &tree.proof(&world.env, 2),
    );

    // Indices 0..=3 share one 128-bit bucket; only bit 2 may be set.
    for index in 0..4u32 {
        assert_eq!(airdrop.is_claimed(&index), index == 2, "index {index}");
    }
}

#[test]
fn claim_flags_cross_bucket_boundaries_correctly() {
    let world = world();
    // 130 allocations pad to 256 leaves, so index 127 stays in the first 128-bit bucket
    // while 128 and 129 land in the second.
    let allocations: Vec<(Address, i128)> = (0..130)
        .map(|i| (Address::generate(&world.env), (i as i128 + 1) * UNIT))
        .collect();

    let tree = Tree::build(&world.env, &allocations);
    assert_eq!(tree.depth(), 8);
    world.airdrop().initialize(
        &world.admin,
        &world.token,
        &tree.root(),
        &(allocations.len() as u32),
        &total_of(&allocations),
        &0,
    );
    world.asset().mint(&world.contract, &total_of(&allocations));

    let airdrop = world.airdrop();
    for index in [127usize, 128, 129] {
        airdrop.claim(
            &allocations[index].0,
            &(index as u32),
            &allocations[index].1,
            &tree.proof(&world.env, index),
        );
    }

    for index in [0u32, 126, 127, 128, 129] {
        assert_eq!(airdrop.is_claimed(&index), index >= 127, "index {index}");
    }
    assert_eq!(airdrop.progress().1, 3);
}

// ---------------------------------------------------------------------------
// Root rotation
// ---------------------------------------------------------------------------

#[test]
fn the_root_can_be_rotated_until_the_first_claim() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = Tree::build(&world.env, &allocations);
    let airdrop = world.airdrop();

    airdrop.initialize(
        &world.admin,
        &world.token,
        &tree.root(),
        &4,
        &total_of(&allocations),
        &0,
    );
    world.asset().mint(&world.contract, &total_of(&allocations));

    // A corrected tree — say the generator dropped a recipient.
    let corrected = five_recipients(&world.env);
    let corrected_tree = Tree::build(&world.env, &corrected);
    airdrop.set_merkle_root(
        &world.admin,
        &corrected_tree.root(),
        &(corrected.len() as u32),
        &total_of(&corrected),
    );

    assert_eq!(airdrop.merkle_root(), corrected_tree.root());
    assert_eq!(airdrop.leaf_count(), 5);
    assert_eq!(airdrop.required_proof_depth(), 3);

    // The corrected tree is the one that now pays out.
    airdrop.claim(
        &corrected[4].0,
        &4,
        &corrected[4].1,
        &corrected_tree.proof(&world.env, 4),
    );
    assert_eq!(
        world.token_client().balance(&corrected[4].0),
        corrected[4].1
    );
}

#[test]
fn the_root_is_frozen_once_a_claim_has_landed() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);
    let airdrop = world.airdrop();

    airdrop.claim(
        &allocations[0].0,
        &0,
        &allocations[0].1,
        &tree.proof(&world.env, 0),
    );

    // Rotating now would reassign slots whose flags are already burned, stranding
    // allocations the original root had granted.
    let replacement = Tree::build(&world.env, &allocations);
    assert_error(
        airdrop.try_set_merkle_root(&world.admin, &replacement.root(), &4, &(10 * UNIT)),
        Error::RootFinalized,
    );
    assert_eq!(airdrop.merkle_root(), tree.root());
}

#[test]
fn only_the_admin_can_rotate_the_root() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);
    let intruder = Address::generate(&world.env);

    assert_error(
        world
            .airdrop()
            .try_set_merkle_root(&intruder, &tree.root(), &4, &(10 * UNIT)),
        Error::Unauthorized,
    );
}

// ---------------------------------------------------------------------------
// Pause
// ---------------------------------------------------------------------------

#[test]
fn only_the_admin_can_pause() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let _ = deployed(&world, &allocations, 0);
    let intruder = Address::generate(&world.env);

    assert_error(
        world.airdrop().try_set_paused(&intruder, &true),
        Error::Unauthorized,
    );
    assert!(!world.airdrop().is_paused());
}

#[test]
fn pausing_halts_new_claims_and_resuming_restarts_them() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 0);
    let airdrop = world.airdrop();

    airdrop.claim(
        &allocations[0].0,
        &0,
        &allocations[0].1,
        &tree.proof(&world.env, 0),
    );

    airdrop.set_paused(&world.admin, &true);
    assert!(airdrop.is_paused());

    assert_error(
        airdrop.try_claim(
            &allocations[1].0,
            &1,
            &allocations[1].1,
            &tree.proof(&world.env, 1),
        ),
        Error::ContractPaused,
    );

    airdrop.set_paused(&world.admin, &false);
    airdrop.claim(
        &allocations[1].0,
        &1,
        &allocations[1].1,
        &tree.proof(&world.env, 1),
    );
    assert_eq!(
        world.token_client().balance(&allocations[1].0),
        allocations[1].1
    );
}

// ---------------------------------------------------------------------------
// Claim window and clawback
// ---------------------------------------------------------------------------

#[test]
fn claims_stop_at_the_deadline() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 1_000);
    let airdrop = world.airdrop();

    world.env.ledger().set_timestamp(999);
    airdrop.claim(
        &allocations[0].0,
        &0,
        &allocations[0].1,
        &tree.proof(&world.env, 0),
    );

    world.env.ledger().set_timestamp(1_000);
    assert_error(
        airdrop.try_claim(
            &allocations[1].0,
            &1,
            &allocations[1].1,
            &tree.proof(&world.env, 1),
        ),
        Error::ClaimWindowClosed,
    );
}

#[test]
fn clawback_needs_a_deadline_and_waits_for_it() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let _ = deployed(&world, &allocations, 1_000);
    let airdrop = world.airdrop();
    let treasury = Address::generate(&world.env);

    world.env.ledger().set_timestamp(999);
    assert_error(
        airdrop.try_clawback(&world.admin, &treasury),
        Error::ClaimWindowStillOpen,
    );

    world.env.ledger().set_timestamp(1_000);
    // Nobody claimed, so the whole balance is swept.
    let swept = airdrop.clawback(&world.admin, &treasury);
    assert_eq!(swept, 10 * UNIT);
    assert_eq!(world.token_client().balance(&treasury), 10 * UNIT);
    assert_eq!(world.token_client().balance(&world.contract), 0);

    // Nothing left for a second sweep.
    assert_error(
        airdrop.try_clawback(&world.admin, &treasury),
        Error::EmptyAirdropBalance,
    );
}

#[test]
fn clawback_leaves_claimed_allocations_alone() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let tree = deployed(&world, &allocations, 1_000);
    let airdrop = world.airdrop();
    let treasury = Address::generate(&world.env);

    airdrop.claim(
        &allocations[0].0,
        &0,
        &allocations[0].1,
        &tree.proof(&world.env, 0),
    );

    world.env.ledger().set_timestamp(1_000);
    let swept = airdrop.clawback(&world.admin, &treasury);
    assert_eq!(swept, 6 * UNIT, "the paid-out allocation is not swept");

    assert_eq!(world.token_client().balance(&allocations[0].0), 4 * UNIT);
    assert_eq!(world.token_client().balance(&treasury), 6 * UNIT);
}

#[test]
fn a_distribution_without_a_deadline_can_never_be_clawed_back() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let _ = deployed(&world, &allocations, 0);
    let treasury = Address::generate(&world.env);

    // Even far in the future the sweeper stays closed, so a perpetual airdrop can never
    // be drained out from under its recipients.
    world.env.ledger().set_timestamp(u64::from(u32::MAX));
    assert_error(
        world.airdrop().try_clawback(&world.admin, &treasury),
        Error::DeadlineNotSet,
    );
}

#[test]
fn only_the_admin_can_claw_back() {
    let world = world();
    let allocations = four_recipients(&world.env);
    let _ = deployed(&world, &allocations, 1_000);
    let intruder = Address::generate(&world.env);

    world.env.ledger().set_timestamp(2_000);
    assert_error(
        world.airdrop().try_clawback(&intruder, &intruder),
        Error::Unauthorized,
    );
}
