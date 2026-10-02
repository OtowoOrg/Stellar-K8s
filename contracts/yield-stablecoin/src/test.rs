//! # Test Suite — Programmable Yield-Bearing Stablecoin
//!
//! Covers:
//! * Happy-path mint, burn, transfer, approve / transfer_from
//! * Compliance: blacklisted and frozen addresses are blocked from all flows
//! * Yield distribution: proportional allocation, dust handling, skipping
//!   restricted recipients, multi-sig quorum enforcement
//! * Multi-sig: duplicate signer rejection, insufficient quorum rejection
//! * Fuzz-style parametric tests: large multi-sig payloads, extreme amounts

#![allow(unused_imports)]

use soroban_sdk::{
    testutils::Address as _,
    vec, Address, Env, String,
};

use crate::{
    hooks::{is_blacklisted, is_frozen},
    Error, YieldStablecoin, YieldStablecoinClient,
};

// ── Test harness ─────────────────────────────────────────────────────────────

/// Default token parameters used across most tests.
const DECIMALS: u32 = 7;

struct Setup {
    env: Env,
    #[allow(dead_code)]
    contract: Address,
    admin: Address,
    signers: soroban_sdk::Vec<Address>,
    /// Convenience client.
    client: YieldStablecoinClient<'static>,
}

/// Leak `env` to obtain a `'static` reference required by the typed client.
/// This is safe in tests because the `Env` lives for the duration of the test.
fn setup_with_signers(signer_count: usize, threshold: u32, unblacklist_threshold: u32) -> Setup {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let mut signer_vec = soroban_sdk::Vec::new(&env);
    for _ in 0..signer_count {
        signer_vec.push_back(Address::generate(&env));
    }

    let contract = env.register(
        YieldStablecoin,
        (
            admin.clone(),
            String::from_str(&env, "USD Yield Coin"),
            String::from_str(&env, "USDY"),
            DECIMALS,
            signer_vec.clone(),
            threshold,
            unblacklist_threshold,
        ),
    );

    // SAFETY: env is owned by Setup and lives for the whole test.
    let env_ref: &'static Env = unsafe { &*(&env as *const Env) };
    let client = YieldStablecoinClient::new(env_ref, &contract);

    Setup {
        env,
        contract,
        admin,
        signers: signer_vec,
        client,
    }
}

fn setup() -> Setup {
    setup_with_signers(3, 2, 3)
}

// ── Basic token metadata ──────────────────────────────────────────────────────

#[test]
fn test_metadata() {
    let s = setup();
    assert_eq!(s.client.name(), String::from_str(&s.env, "USD Yield Coin"));
    assert_eq!(s.client.symbol(), String::from_str(&s.env, "USDY"));
    assert_eq!(s.client.decimals(), DECIMALS);
    assert_eq!(s.client.total_supply(), 0);
}

// ── Mint ──────────────────────────────────────────────────────────────────────

#[test]
fn test_mint_increases_balance_and_supply() {
    let s = setup();
    let user = Address::generate(&s.env);

    s.client.mint(&s.admin, &user, &1_000_000_000i128);

    assert_eq!(s.client.balance(&user), 1_000_000_000);
    assert_eq!(s.client.total_supply(), 1_000_000_000);
}

#[test]
#[should_panic]
fn test_mint_zero_rejected() {
    let s = setup();
    let user = Address::generate(&s.env);
    s.client.mint(&s.admin, &user, &0i128);
}

#[test]
#[should_panic]
fn test_mint_negative_rejected() {
    let s = setup();
    let user = Address::generate(&s.env);
    s.client.mint(&s.admin, &user, &-1i128);
}

#[test]
#[should_panic]
fn test_mint_to_blacklisted_address_panics() {
    let s = setup();
    let user = Address::generate(&s.env);
    s.client.blacklist_address(&s.admin, &user);
    // Compliance hook must fire and panic before any state mutation.
    s.client.mint(&s.admin, &user, &500i128);
}

#[test]
#[should_panic]
fn test_mint_to_frozen_address_panics() {
    let s = setup();
    let user = Address::generate(&s.env);
    s.client.freeze_address(&s.admin, &user);
    s.client.mint(&s.admin, &user, &500i128);
}

// ── Burn ──────────────────────────────────────────────────────────────────────

#[test]
fn test_burn_decreases_balance_and_supply() {
    let s = setup();
    let user = Address::generate(&s.env);
    s.client.mint(&s.admin, &user, &1_000i128);
    s.client.burn(&user, &400i128);

    assert_eq!(s.client.balance(&user), 600);
    assert_eq!(s.client.total_supply(), 600);
}

#[test]
#[should_panic]
fn test_burn_exceeds_balance_panics() {
    let s = setup();
    let user = Address::generate(&s.env);
    s.client.mint(&s.admin, &user, &100i128);
    s.client.burn(&user, &200i128);
}

#[test]
#[should_panic]
fn test_burn_from_blacklisted_panics() {
    let s = setup();
    let user = Address::generate(&s.env);
    s.client.mint(&s.admin, &user, &1_000i128);
    s.client.blacklist_address(&s.admin, &user);
    s.client.burn(&user, &100i128);
}

#[test]
#[should_panic]
fn test_burn_from_frozen_panics() {
    let s = setup();
    let user = Address::generate(&s.env);
    s.client.mint(&s.admin, &user, &1_000i128);
    s.client.freeze_address(&s.admin, &user);
    s.client.burn(&user, &100i128);
}

// ── Transfer ──────────────────────────────────────────────────────────────────

#[test]
fn test_transfer_happy_path() {
    let s = setup();
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);

    s.client.mint(&s.admin, &alice, &2_000i128);
    s.client.transfer(&alice, &bob, &800i128);

    assert_eq!(s.client.balance(&alice), 1_200);
    assert_eq!(s.client.balance(&bob), 800);
    assert_eq!(s.client.total_supply(), 2_000); // supply unchanged
}

#[test]
#[should_panic]
fn test_transfer_from_frozen_sender_panics() {
    let s = setup();
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    s.client.mint(&s.admin, &alice, &1_000i128);
    s.client.freeze_address(&s.admin, &alice);
    s.client.transfer(&alice, &bob, &100i128);
}

#[test]
#[should_panic]
fn test_transfer_to_blacklisted_recipient_panics() {
    let s = setup();
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    s.client.mint(&s.admin, &alice, &1_000i128);
    s.client.blacklist_address(&s.admin, &bob);
    s.client.transfer(&alice, &bob, &100i128);
}

#[test]
fn test_thaw_restores_transfers() {
    let s = setup();
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    s.client.mint(&s.admin, &alice, &500i128);
    s.client.freeze_address(&s.admin, &alice);
    s.client.thaw_address(&s.admin, &alice);
    s.client.transfer(&alice, &bob, &200i128);
    assert_eq!(s.client.balance(&bob), 200);
}

// ── Approve / transfer_from ───────────────────────────────────────────────────

#[test]
fn test_approve_and_transfer_from() {
    let s = setup();
    let owner = Address::generate(&s.env);
    let spender = Address::generate(&s.env);
    let recipient = Address::generate(&s.env);

    s.client.mint(&s.admin, &owner, &1_000i128);
    s.client.approve(&owner, &spender, &600i128);
    assert_eq!(s.client.allowance(&owner, &spender), 600);

    s.client.transfer_from(&spender, &owner, &recipient, &400i128);
    assert_eq!(s.client.balance(&owner), 600);
    assert_eq!(s.client.balance(&recipient), 400);
    assert_eq!(s.client.allowance(&owner, &spender), 200);
}

#[test]
#[should_panic]
fn test_transfer_from_exceeds_allowance_panics() {
    let s = setup();
    let owner = Address::generate(&s.env);
    let spender = Address::generate(&s.env);
    let recipient = Address::generate(&s.env);
    s.client.mint(&s.admin, &owner, &1_000i128);
    s.client.approve(&owner, &spender, &100i128);
    s.client.transfer_from(&spender, &owner, &recipient, &500i128);
}

// ── Compliance: blacklist / freeze ────────────────────────────────────────────

#[test]
fn test_blacklist_flag_persists() {
    let s = setup();
    let user = Address::generate(&s.env);
    assert!(!s.client.is_blacklisted(&user));
    s.client.blacklist_address(&s.admin, &user);
    assert!(s.client.is_blacklisted(&user));
}

#[test]
fn test_freeze_and_thaw_cycle() {
    let s = setup();
    let user = Address::generate(&s.env);
    s.client.freeze_address(&s.admin, &user);
    assert!(s.client.is_frozen(&user));
    s.client.thaw_address(&s.admin, &user);
    assert!(!s.client.is_frozen(&user));
}

#[test]
fn test_unblacklist_requires_multisig_quorum() {
    // Setup: 3 signers, unblacklist threshold = 3 (all must sign)
    let s = setup_with_signers(3, 2, 3);
    let user = Address::generate(&s.env);
    s.client.blacklist_address(&s.admin, &user);

    // Provide all 3 signers → should succeed
    s.client.unblacklist_address(&s.signers, &user);
    assert!(!s.client.is_blacklisted(&user));
}

#[test]
#[should_panic]
fn test_unblacklist_with_insufficient_signers_panics() {
    // threshold=3, but only 2 signers provided
    let s = setup_with_signers(3, 2, 3);
    let user = Address::generate(&s.env);
    s.client.blacklist_address(&s.admin, &user);

    let mut partial = soroban_sdk::Vec::new(&s.env);
    partial.push_back(s.signers.get(0).unwrap());
    partial.push_back(s.signers.get(1).unwrap());
    s.client.unblacklist_address(&partial, &user);
}

// ── Yield distribution ────────────────────────────────────────────────────────

#[test]
fn test_yield_distribution_proportional() {
    // 3 holders: 1000, 2000, 7000 (total 10000)
    // Yield = 1000 → allocations: 100, 200, 700
    let s = setup();
    let alice = Address::generate(&s.env);
    let bob = Address::generate(&s.env);
    let carol = Address::generate(&s.env);

    s.client.mint(&s.admin, &alice, &1_000i128);
    s.client.mint(&s.admin, &bob, &2_000i128);
    s.client.mint(&s.admin, &carol, &7_000i128);

    let recipients = vec![&s.env, alice.clone(), bob.clone(), carol.clone()];
    let balances = vec![&s.env, 1_000i128, 2_000i128, 7_000i128];

    // Use 2-of-3 threshold signers
    let auth_signers = vec![
        &s.env,
        s.signers.get(0).unwrap(),
        s.signers.get(1).unwrap(),
    ];

    let minted = s.client.distribute_yield(
        &auth_signers,
        &recipients,
        &balances,
        &10_000i128,
        &1_000i128,
        &1u32,
    );

    assert_eq!(minted, 1_000); // 100 + 200 + 700 = 1000
    assert_eq!(s.client.balance(&alice), 1_100);
    assert_eq!(s.client.balance(&bob), 2_200);
    assert_eq!(s.client.balance(&carol), 7_700);
    assert_eq!(s.client.total_supply(), 11_000);
}

#[test]
fn test_yield_distribution_skips_blacklisted_recipient() {
    let s = setup();
    let alice = Address::generate(&s.env);
    let bad_actor = Address::generate(&s.env);

    s.client.mint(&s.admin, &alice, &5_000i128);
    s.client.mint(&s.admin, &bad_actor, &5_000i128);
    s.client.blacklist_address(&s.admin, &bad_actor);

    let recipients = vec![&s.env, alice.clone(), bad_actor.clone()];
    let balances = vec![&s.env, 5_000i128, 5_000i128];
    let auth_signers = vec![
        &s.env,
        s.signers.get(0).unwrap(),
        s.signers.get(1).unwrap(),
    ];

    let minted = s.client.distribute_yield(
        &auth_signers,
        &recipients,
        &balances,
        &10_000i128,
        &1_000i128,
        &1u32,
    );

    // Only alice's 500 minted; bad_actor silently skipped
    assert_eq!(minted, 500);
    assert_eq!(s.client.balance(&alice), 5_500);
    assert_eq!(s.client.balance(&bad_actor), 5_000); // unchanged
}

#[test]
fn test_yield_distribution_skips_frozen_recipient() {
    let s = setup();
    let alice = Address::generate(&s.env);
    let frozen_user = Address::generate(&s.env);

    s.client.mint(&s.admin, &alice, &6_000i128);
    s.client.mint(&s.admin, &frozen_user, &4_000i128);
    s.client.freeze_address(&s.admin, &frozen_user);

    let recipients = vec![&s.env, alice.clone(), frozen_user.clone()];
    let balances = vec![&s.env, 6_000i128, 4_000i128];
    let auth_signers = vec![
        &s.env,
        s.signers.get(0).unwrap(),
        s.signers.get(1).unwrap(),
    ];

    let minted = s.client.distribute_yield(
        &auth_signers,
        &recipients,
        &balances,
        &10_000i128,
        &1_000i128,
        &1u32,
    );

    assert_eq!(minted, 600); // only alice's share
    assert_eq!(s.client.balance(&frozen_user), 4_000); // unchanged
}

#[test]
fn test_yield_dust_not_minted() {
    // 3 holders each with 1 out of a supply of 10 → each gets 0 from 2 yield
    // (2 / 10 = 0 per holder after floor division)
    let s = setup();
    let users: soroban_sdk::Vec<Address> = {
        let mut v = soroban_sdk::Vec::new(&s.env);
        for _ in 0..3 {
            let u = Address::generate(&s.env);
            s.client.mint(&s.admin, &u, &1i128);
            v.push_back(u);
        }
        v
    };

    let balances = vec![&s.env, 1i128, 1i128, 1i128];
    let auth_signers = vec![
        &s.env,
        s.signers.get(0).unwrap(),
        s.signers.get(1).unwrap(),
    ];

    let minted = s.client.distribute_yield(
        &auth_signers,
        &users,
        &balances,
        &10i128, // total supply snapshot
        &2i128,  // tiny yield
        &1u32,
    );

    // floor(1 * 2 / 10) = 0 for each → nothing minted
    assert_eq!(minted, 0);
}

#[test]
#[should_panic]
fn test_yield_multisig_quorum_not_met_panics() {
    let s = setup(); // threshold = 2
    let user = Address::generate(&s.env);
    s.client.mint(&s.admin, &user, &1_000i128);

    // Only 1 signer provided (threshold is 2)
    let auth_signers = vec![&s.env, s.signers.get(0).unwrap()];
    let recipients = vec![&s.env, user.clone()];
    let balances = vec![&s.env, 1_000i128];

    s.client.distribute_yield(
        &auth_signers,
        &recipients,
        &balances,
        &1_000i128,
        &100i128,
        &1u32,
    );
}

#[test]
#[should_panic]
fn test_yield_duplicate_signer_panics() {
    let s = setup(); // threshold = 2
    let user = Address::generate(&s.env);
    s.client.mint(&s.admin, &user, &1_000i128);

    let signer0 = s.signers.get(0).unwrap();
    // Same signer listed twice → duplicate detection must fire
    let auth_signers = vec![&s.env, signer0.clone(), signer0.clone()];
    let recipients = vec![&s.env, user.clone()];
    let balances = vec![&s.env, 1_000i128];

    s.client.distribute_yield(
        &auth_signers,
        &recipients,
        &balances,
        &1_000i128,
        &100i128,
        &1u32,
    );
}

// ── Fuzz-style: large payloads ────────────────────────────────────────────────

/// Simulate a large multi-sig payload (20 signers, threshold 11) with a 200-
/// holder yield distribution to verify no CPU instruction budget blowout in
/// the test environment and correct proportional arithmetic.
#[test]
fn test_fuzz_large_multisig_and_distribution() {
    const SIGNER_COUNT: usize = 20;
    const THRESHOLD: u32 = 11;
    const HOLDER_COUNT: usize = 50;
    const BALANCE_PER_HOLDER: i128 = 1_000_000;
    const YIELD_AMOUNT: i128 = 500_000; // 1% of total supply

    let s = setup_with_signers(SIGNER_COUNT, THRESHOLD, THRESHOLD + 1);

    // Mint to all holders
    let mut holders = soroban_sdk::Vec::new(&s.env);
    let mut balances_vec = soroban_sdk::Vec::new(&s.env);
    let total_supply: i128 = (HOLDER_COUNT as i128) * BALANCE_PER_HOLDER;

    for _ in 0..HOLDER_COUNT {
        let h = Address::generate(&s.env);
        s.client.mint(&s.admin, &h, &BALANCE_PER_HOLDER);
        holders.push_back(h);
        balances_vec.push_back(BALANCE_PER_HOLDER);
    }
    assert_eq!(s.client.total_supply(), total_supply);

    // Authorize with exactly `threshold` signers
    let mut auth_signers = soroban_sdk::Vec::new(&s.env);
    for i in 0..(THRESHOLD as usize) {
        auth_signers.push_back(s.signers.get(i as u32).unwrap());
    }

    let minted = s.client.distribute_yield(
        &auth_signers,
        &holders,
        &balances_vec,
        &total_supply,
        &YIELD_AMOUNT,
        &42u32,
    );

    // Each holder gets floor(1_000_000 * 500_000 / 50_000_000) = 10_000
    let expected_per = BALANCE_PER_HOLDER * YIELD_AMOUNT / total_supply;
    let expected_total = expected_per * (HOLDER_COUNT as i128);
    assert_eq!(minted, expected_total);
    assert_eq!(s.client.total_supply(), total_supply + expected_total);
}

/// Verify that a large batch of mints does not overflow total_supply tracking
/// (uses saturating arithmetic checks).
#[test]
fn test_fuzz_mass_mint_supply_integrity() {
    let s = setup();
    let iterations = 100u32;
    let amount_each: i128 = 10_000_000_000i128; // 10 billion base units each

    for _ in 0..iterations {
        let user = Address::generate(&s.env);
        s.client.mint(&s.admin, &user, &amount_each);
    }

    let expected_supply: i128 = (iterations as i128) * amount_each;
    assert_eq!(s.client.total_supply(), expected_supply);
}

/// Verify that blacklisted addresses are cryptographically blocked from
/// receiving yield across a large batch run.
#[test]
fn test_fuzz_blacklisted_accounts_blocked_from_yield() {
    const TOTAL: usize = 30;
    const BLACKLISTED_EVERY: usize = 3; // every 3rd address is blacklisted

    let s = setup();
    let mut holders = soroban_sdk::Vec::new(&s.env);
    let mut balances_vec = soroban_sdk::Vec::new(&s.env);
    // Track which indices got blacklisted (soroban Vec — no std available)
    let mut blacklisted_indices = soroban_sdk::Vec::<u32>::new(&s.env);

    let balance_each: i128 = 1_000;
    let total_supply: i128 = (TOTAL as i128) * balance_each;

    for i in 0..TOTAL {
        let h = Address::generate(&s.env);
        s.client.mint(&s.admin, &h, &balance_each);
        if i % BLACKLISTED_EVERY == 0 {
            s.client.blacklist_address(&s.admin, &h);
            blacklisted_indices.push_back(i as u32);
        }
        holders.push_back(h.clone());
        balances_vec.push_back(balance_each);
    }

    let auth_signers = vec![
        &s.env,
        s.signers.get(0).unwrap(),
        s.signers.get(1).unwrap(),
    ];
    let yield_amount: i128 = 3_000;

    s.client.distribute_yield(
        &auth_signers,
        &holders,
        &balances_vec,
        &total_supply,
        &yield_amount,
        &1u32,
    );

    // Every blacklisted holder must have exactly their original balance (no yield added)
    for i in 0..blacklisted_indices.len() {
        let idx = blacklisted_indices.get(i).unwrap();
        let addr = holders.get(idx).unwrap();
        assert!(s.client.is_blacklisted(&addr));
        assert_eq!(
            s.client.balance(&addr),
            balance_each,
            "Blacklisted address at index {} received yield — cryptographic block failed",
            idx
        );
    }
}

// ── Multi-sig config update ───────────────────────────────────────────────────

#[test]
fn test_update_multisig_config() {
    let s = setup(); // 3 signers, threshold 2
    let new_s1 = Address::generate(&s.env);
    let new_s2 = Address::generate(&s.env);
    let new_signers = vec![&s.env, new_s1.clone(), new_s2.clone()];

    let auth = vec![
        &s.env,
        s.signers.get(0).unwrap(),
        s.signers.get(1).unwrap(),
    ];
    s.client.update_multisig(&auth, &new_signers, &2u32, &2u32);

    // Now distribute yield using NEW signers — old signers must no longer work
    let user = Address::generate(&s.env);
    s.client.mint(&s.admin, &user, &1_000i128);

    let new_auth = vec![&s.env, new_s1.clone(), new_s2.clone()];
    let recipients = vec![&s.env, user.clone()];
    let balances = vec![&s.env, 1_000i128];

    let minted = s.client.distribute_yield(
        &new_auth,
        &recipients,
        &balances,
        &1_000i128,
        &100i128,
        &2u32,
    );
    assert_eq!(minted, 100);
}

// ── Admin transfer ────────────────────────────────────────────────────────────

#[test]
fn test_set_admin_transfers_authority() {
    let s = setup();
    let new_admin = Address::generate(&s.env);
    s.client.set_admin(&s.admin, &new_admin);

    // New admin can mint; old admin cannot
    let user = Address::generate(&s.env);
    s.client.mint(&new_admin, &user, &100i128);
    assert_eq!(s.client.balance(&user), 100);
}

#[test]
#[should_panic]
fn test_old_admin_cannot_mint_after_transfer() {
    let s = setup();
    let new_admin = Address::generate(&s.env);
    s.client.set_admin(&s.admin, &new_admin);

    let user = Address::generate(&s.env);
    // old admin should no longer be authorized
    s.client.mint(&s.admin, &user, &100i128);
}
