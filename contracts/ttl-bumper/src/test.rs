//! Integration tests for the TTL Auto-Bump Maintenance Contract.
//!
//! These tests use the Soroban test environment to:
//!   1. Simulate contract registration and key aging.
//!   2. Verify batch TTL extension operates correctly.
//!   3. Confirm keeper bounty payouts are accurate and balance-safe.
//!   4. Validate security: admin-only functions, deregistration auth, etc.
//!
//! Note: `env.deployer().extend_ttl()` requires the target contract to be
//! deployed in the ledger.  Tests therefore register real contracts (typically
//! additional TtlBumperContract instances) as the "target" entries.

#![cfg(test)]

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec,
};

use crate::{
    registry::{DEFAULT_EXTENSION_LEDGERS, DEFAULT_THRESHOLD_LEDGERS, MAX_REGISTRY_SIZE},
    TtlBumperContract, TtlBumperContractClient, MAX_BATCH_SIZE,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deploy the bumper contract and return (env, contract_address, client).
fn setup() -> (Env, Address, TtlBumperContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(TtlBumperContract, ());
    let client = TtlBumperContractClient::new(&env, &contract_id);
    (env, contract_id, client)
}

/// Deploy the bumper + create an admin address + initialize.
fn setup_initialized() -> (Env, Address, TtlBumperContractClient<'static>, Address) {
    let (env, contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin, &0i128);
    (env, contract_id, client, admin)
}

/// Create a mock XLM token (Stellar Asset Contract) in the test environment.
fn create_token<'a>(
    env: &'a Env,
    admin: &Address,
) -> (Address, TokenClient<'a>, StellarAssetClient<'a>) {
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());
    let token_addr = token_id.address();
    let token = TokenClient::new(env, &token_addr);
    let token_admin = StellarAssetClient::new(env, &token_addr);
    (token_addr, token, token_admin)
}

/// Register N *actually deployed* contracts (fresh TtlBumperContract instances)
/// and return their (address, entry_id) pairs.
///
/// Using deployed contracts ensures `env.deployer().extend_ttl()` succeeds
/// because the contract instance exists in the ledger.
fn register_n_deployed_contracts(
    env: &Env,
    client: &TtlBumperContractClient,
    owner: &Address,
    n: u32,
) -> Vec<(Address, u32)> {
    let mut results = Vec::new(env);
    for _ in 0..n {
        // Deploy a real contract so the instance exists in the ledger.
        let target = env.register(TtlBumperContract, ());
        let id = client.register(
            &target,
            &DEFAULT_THRESHOLD_LEDGERS,
            &DEFAULT_EXTENSION_LEDGERS,
            owner,
        );
        results.push_back((target, id));
    }
    results
}

// ===========================================================================
// Lifecycle tests
// ===========================================================================

#[test]
fn test_initialize_sets_admin() {
    let (_env, _contract_id, client, admin) = setup_initialized();
    assert_eq!(client.view_admin(), admin);
}

#[test]
fn test_initialize_defaults() {
    let (_, _, client, _) = setup_initialized();
    assert_eq!(client.view_bounty_balance(), 0i128);
    assert_eq!(client.view_bounty_per_key(), 0i128);
    assert_eq!(client.view_active_count(), 0u32);
}

#[test]
#[should_panic(expected = "AlreadyInit")]
fn test_initialize_twice_panics() {
    let (_env, _, client, admin) = setup_initialized();
    client.initialize(&admin, &0i128);
}

#[test]
fn test_initialize_with_bounty_per_key() {
    let (env, _, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin, &1_000_000i128);
    assert_eq!(client.view_bounty_per_key(), 1_000_000i128);
}

// ===========================================================================
// Registry tests
// ===========================================================================

#[test]
fn test_register_single_entry() {
    let (env, _, client, _admin) = setup_initialized();
    let owner = Address::generate(&env);
    // Use a deployed contract as target.
    let target = env.register(TtlBumperContract, ());

    let id = client.register(&target, &500u32, &1000u32, &owner);
    assert_eq!(id, 1u32);
    assert_eq!(client.view_active_count(), 1u32);
}

#[test]
fn test_register_uses_defaults_when_zero() {
    let (env, _, client, _admin) = setup_initialized();
    let owner = Address::generate(&env);
    let target = env.register(TtlBumperContract, ());

    let id = client.register(&target, &0u32, &0u32, &owner);
    let entry = client.view_entry(&id);
    assert_eq!(entry.threshold_ledgers, DEFAULT_THRESHOLD_LEDGERS);
    assert_eq!(entry.extension_ledgers, DEFAULT_EXTENSION_LEDGERS);
}

#[test]
fn test_register_multiple_entries() {
    let (env, _, client, _admin) = setup_initialized();
    let owner = Address::generate(&env);

    let pairs = register_n_deployed_contracts(&env, &client, &owner, 5);
    assert_eq!(client.view_active_count(), 5u32);
    for (idx, (_addr, id)) in pairs.iter().enumerate() {
        assert_eq!(id, (idx as u32) + 1);
    }
}

#[test]
fn test_deregister_by_owner() {
    let (env, _, client, _admin) = setup_initialized();
    let owner = Address::generate(&env);
    let target = env.register(TtlBumperContract, ());

    let id = client.register(&target, &500u32, &1000u32, &owner);
    assert_eq!(client.view_active_count(), 1u32);

    let removed = client.deregister(&id, &owner);
    assert!(removed);
    assert_eq!(client.view_active_count(), 0u32);
}

#[test]
fn test_deregister_by_admin() {
    let (env, _, client, admin) = setup_initialized();
    let owner = Address::generate(&env);
    let target = env.register(TtlBumperContract, ());

    let id = client.register(&target, &500u32, &1000u32, &owner);
    let removed = client.deregister(&id, &admin);
    assert!(removed);
    assert_eq!(client.view_active_count(), 0u32);
}

#[test]
#[should_panic]
fn test_deregister_by_non_owner_panics() {
    let (env, _, client, _admin) = setup_initialized();
    let owner = Address::generate(&env);
    let attacker = Address::generate(&env);
    let target = env.register(TtlBumperContract, ());

    let id = client.register(&target, &500u32, &1000u32, &owner);
    client.deregister(&id, &attacker);
}

#[test]
fn test_deregister_nonexistent_returns_false() {
    let (_env, _, client, admin) = setup_initialized();
    let result = client.deregister(&999u32, &admin);
    assert!(!result);
}

#[test]
fn test_view_active_entries_excludes_deregistered() {
    let (env, _, client, admin) = setup_initialized();
    let owner = Address::generate(&env);

    let id1 = client.register(&env.register(TtlBumperContract, ()), &500u32, &1000u32, &owner);
    let id2 = client.register(&env.register(TtlBumperContract, ()), &500u32, &1000u32, &owner);
    let id3 = client.register(&env.register(TtlBumperContract, ()), &500u32, &1000u32, &owner);

    client.deregister(&id2, &admin);

    let active = client.view_active_entries();
    assert_eq!(active.len(), 2);
    let active_ids: std::vec::Vec<u32> = active.iter().map(|e| e.id).collect();
    assert!(active_ids.contains(&id1));
    assert!(!active_ids.contains(&id2));
    assert!(active_ids.contains(&id3));
}

// ===========================================================================
// Batch bump tests (no bounty)
// ===========================================================================

#[test]
fn test_bump_batch_empty_panics() {
    let (env, _, client, _admin) = setup_initialized();
    let keeper = Address::generate(&env);
    let empty: Vec<u32> = Vec::new(&env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.bump_batch(&keeper, &empty, &None);
    }));
    assert!(result.is_err(), "empty batch should panic");
}

#[test]
fn test_bump_batch_skips_unknown_ids() {
    let (env, _, client, _admin) = setup_initialized();
    let keeper = Address::generate(&env);
    // IDs 100, 200 don't exist – should silently skip and return 0 bounty.
    let ids = vec![&env, 100u32, 200u32];
    let bounty = client.bump_batch(&keeper, &ids, &None);
    assert_eq!(bounty, 0u64);
}

#[test]
fn test_bump_batch_returns_zero_when_no_bounty_configured() {
    let (env, _, client, _admin) = setup_initialized();
    let owner = Address::generate(&env);
    let keeper = Address::generate(&env);

    // Use a deployed contract so extend_ttl succeeds.
    let target = env.register(TtlBumperContract, ());
    let id = client.register(&target, &500u32, &1000u32, &owner);
    let ids = vec![&env, id];
    let bounty = client.bump_batch(&keeper, &ids, &None);
    assert_eq!(bounty, 0u64);
}

#[test]
fn test_bump_batch_processes_all_active_entries() {
    let (env, _, client, _admin) = setup_initialized();
    let owner = Address::generate(&env);
    let keeper = Address::generate(&env);

    let pairs = register_n_deployed_contracts(&env, &client, &owner, 10);
    let ids: Vec<u32> = {
        let mut v = Vec::new(&env);
        for (_addr, id) in pairs.iter() {
            v.push_back(id);
        }
        v
    };

    let result = client.bump_batch(&keeper, &ids, &None);
    assert_eq!(result, 0u64);
}

// ===========================================================================
// Batch bump with bounty
// ===========================================================================

#[test]
fn test_bump_batch_pays_bounty_per_key() {
    let (env, _contract_id, client, admin) = setup_initialized();
    let owner = Address::generate(&env);
    let keeper = Address::generate(&env);

    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let pool_amount = 10_000_000i128;
    let bounty_per_key = 100_000i128;

    token_admin.mint(&admin, &pool_amount);
    client.set_bounty(&bounty_per_key);
    client.deposit_bounty(&pool_amount, &token_addr);

    // Register 3 deployed contracts.
    let id1 = client.register(&env.register(TtlBumperContract, ()), &500u32, &1000u32, &owner);
    let id2 = client.register(&env.register(TtlBumperContract, ()), &500u32, &1000u32, &owner);
    let id3 = client.register(&env.register(TtlBumperContract, ()), &500u32, &1000u32, &owner);

    let ids = vec![&env, id1, id2, id3];
    let bounty = client.bump_batch(&keeper, &ids, &Some(token_addr.clone()));

    let expected = (3 * bounty_per_key) as u64;
    assert_eq!(bounty, expected);
    assert_eq!(token.balance(&keeper), bounty_per_key * 3);

    let remaining_pool = pool_amount - (bounty_per_key * 3);
    assert_eq!(client.view_bounty_balance(), remaining_pool);
}

#[test]
fn test_bump_batch_caps_at_pool_balance() {
    let (env, _contract_id, client, admin) = setup_initialized();
    let owner = Address::generate(&env);
    let keeper = Address::generate(&env);

    let (token_addr, _token, token_admin) = create_token(&env, &admin);
    let bounty_per_key = 1_000_000i128;
    // Only enough for 2 keys but we'll submit 5.
    let pool_amount = bounty_per_key * 2;

    token_admin.mint(&admin, &pool_amount);
    client.set_bounty(&bounty_per_key);
    client.deposit_bounty(&pool_amount, &token_addr);

    let pairs = register_n_deployed_contracts(&env, &client, &owner, 5);
    let ids: Vec<u32> = {
        let mut v = Vec::new(&env);
        for (_addr, id) in pairs.iter() {
            v.push_back(id);
        }
        v
    };

    let bounty = client.bump_batch(&keeper, &ids, &Some(token_addr));

    // Only 2 keys could be paid (pool exhausted after key 2).
    assert_eq!(bounty, (bounty_per_key * 2) as u64);
    assert_eq!(client.view_bounty_balance(), 0i128);
}

#[test]
fn test_bump_batch_zero_pool_no_transfer() {
    let (env, _, client, admin) = setup_initialized();
    let owner = Address::generate(&env);
    let keeper = Address::generate(&env);

    let (token_addr, token, _) = create_token(&env, &admin);
    let bounty_per_key = 100_000i128;
    // No deposit – pool is empty.
    client.set_bounty(&bounty_per_key);

    let target = env.register(TtlBumperContract, ());
    let id = client.register(&target, &500u32, &1000u32, &owner);
    let ids = vec![&env, id];

    let bounty = client.bump_batch(&keeper, &ids, &Some(token_addr));
    assert_eq!(bounty, 0u64);
    assert_eq!(token.balance(&keeper), 0i128);
}

// ===========================================================================
// Key aging simulation
// ===========================================================================

/// Simulates key aging:
/// 1. Register a contract with a short threshold.
/// 2. Advance the ledger to simulate time passing.
/// 3. Keeper calls bump_batch; TTL is extended successfully.
#[test]
fn test_key_aging_and_recovery() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(TtlBumperContract, ());
    let client = TtlBumperContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let owner = Address::generate(&env);

    let threshold = 100u32;
    let extension = 500u32;

    client.initialize(&admin, &0i128);

    // Deploy a real contract as the target.
    let target = env.register(TtlBumperContract, ());
    let id = client.register(&target, &threshold, &extension, &owner);

    // Simulate ledger advancement – well past the 100-ledger threshold.
    env.ledger().with_mut(|info| {
        info.sequence_number = info.sequence_number.saturating_add(200);
        info.timestamp = info.timestamp.saturating_add(200 * 5);
    });

    let ids = vec![&env, id];
    let bounty = client.bump_batch(&keeper, &ids, &None);
    assert_eq!(bounty, 0u64);
    assert_eq!(client.view_active_count(), 1u32);
}

/// Multiple keys aging at different rates – batch handles all gracefully.
#[test]
fn test_mixed_expiry_batch() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(TtlBumperContract, ());
    let client = TtlBumperContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let owner = Address::generate(&env);

    client.initialize(&admin, &0i128);

    let id_short = client.register(
        &env.register(TtlBumperContract, ()),
        &50u32,
        &500u32,
        &owner,
    );
    let id_medium = client.register(
        &env.register(TtlBumperContract, ()),
        &DEFAULT_THRESHOLD_LEDGERS,
        &DEFAULT_EXTENSION_LEDGERS,
        &owner,
    );
    let id_long = client.register(
        &env.register(TtlBumperContract, ()),
        &100_000u32,
        &200_000u32,
        &owner,
    );

    env.ledger().with_mut(|info| {
        info.sequence_number = info.sequence_number.saturating_add(1_000);
    });

    let ids = vec![&env, id_short, id_medium, id_long];
    let bounty = client.bump_batch(&keeper, &ids, &None);
    assert_eq!(bounty, 0u64);
    assert_eq!(client.view_active_count(), 3u32);
}

// ===========================================================================
// Bounty management tests
// ===========================================================================

#[test]
fn test_deposit_and_withdraw_bounty() {
    let (env, _, client, admin) = setup_initialized();
    let (token_addr, token, token_admin) = create_token(&env, &admin);

    let amount = 5_000_000i128;
    token_admin.mint(&admin, &amount);
    client.deposit_bounty(&amount, &token_addr);

    assert_eq!(client.view_bounty_balance(), amount);

    let withdraw = amount / 2;
    client.withdraw_bounty(&withdraw, &token_addr, &admin);
    assert_eq!(client.view_bounty_balance(), amount - withdraw);
    assert_eq!(token.balance(&admin), withdraw);
}

#[test]
#[should_panic(expected = "InsufficientBalance")]
fn test_withdraw_exceeds_balance_panics() {
    let (env, _, client, admin) = setup_initialized();
    let (token_addr, _, token_admin) = create_token(&env, &admin);

    let amount = 1_000_000i128;
    token_admin.mint(&admin, &amount);
    client.deposit_bounty(&amount, &token_addr);

    client.withdraw_bounty(&(amount + 1), &token_addr, &admin);
}

#[test]
#[should_panic(expected = "InvalidDeposit")]
fn test_deposit_zero_panics() {
    let (env, _, client, admin) = setup_initialized();
    let (token_addr, _, _) = create_token(&env, &admin);
    client.deposit_bounty(&0i128, &token_addr);
}

#[test]
fn test_set_bounty_per_key() {
    let (_, _, client, _) = setup_initialized();
    client.set_bounty(&500_000i128);
    assert_eq!(client.view_bounty_per_key(), 500_000i128);
}

#[test]
fn test_set_bounty_to_zero_disables_payout() {
    let (_, _, client, _) = setup_initialized();
    client.set_bounty(&1_000_000i128);
    client.set_bounty(&0i128);
    assert_eq!(client.view_bounty_per_key(), 0i128);
}

// ===========================================================================
// Security / authorization tests
// ===========================================================================

#[test]
fn test_batch_too_large_panics() {
    let (env, _, client, _admin) = setup_initialized();
    let keeper = Address::generate(&env);

    let mut ids = Vec::new(&env);
    for i in 0..=MAX_BATCH_SIZE {
        ids.push_back(i);
    }

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.bump_batch(&keeper, &ids, &None);
    }));
    assert!(result.is_err(), "oversized batch should panic");
}

// ===========================================================================
// Registry capacity tests
// ===========================================================================

#[test]
fn test_registry_reaches_max_size() {
    let (env, _, client, _admin) = setup_initialized();
    let owner = Address::generate(&env);

    for _ in 0..MAX_REGISTRY_SIZE {
        client.register(
            &env.register(TtlBumperContract, ()),
            &500u32,
            &1000u32,
            &owner,
        );
    }
    assert_eq!(client.view_active_count(), MAX_REGISTRY_SIZE);
}

#[test]
#[should_panic]
fn test_registry_overflow_panics() {
    let (env, _, client, _admin) = setup_initialized();
    let owner = Address::generate(&env);

    for _ in 0..MAX_REGISTRY_SIZE {
        client.register(
            &env.register(TtlBumperContract, ()),
            &500u32,
            &1000u32,
            &owner,
        );
    }
    // One more should panic.
    client.register(
        &env.register(TtlBumperContract, ()),
        &500u32,
        &1000u32,
        &owner,
    );
}

#[test]
fn test_deregister_frees_logical_slot() {
    let (env, _, client, admin) = setup_initialized();
    let owner = Address::generate(&env);

    let mut ids = std::vec::Vec::new();
    for _ in 0..MAX_REGISTRY_SIZE {
        let id = client.register(
            &env.register(TtlBumperContract, ()),
            &500u32,
            &1000u32,
            &owner,
        );
        ids.push(id);
    }

    assert_eq!(client.view_active_count(), MAX_REGISTRY_SIZE);

    client.deregister(&ids[0], &admin);
    assert_eq!(client.view_active_count(), MAX_REGISTRY_SIZE - 1);
}

// ===========================================================================
// Keeper automated recovery (end-to-end)
// ===========================================================================

/// Full keeper bot workflow:
///   1. Admin deploys + initialises the bumper contract.
///   2. Admin sets up a bounty pool.
///   3. Multiple contract owners register their keys.
///   4. Ledger advances (keys approach expiry).
///   5. Keeper calls bump_batch to extend all near-expiry keys.
///   6. Keeper receives bounty.
///   7. State is consistent after bump.
#[test]
fn test_full_keeper_workflow() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(TtlBumperContract, ());
    let client = TtlBumperContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let owner_a = Address::generate(&env);
    let owner_b = Address::generate(&env);

    // ── Step 1: Initialise ──────────────────────────────────────────────────
    let bounty_per_key = 100_000i128;
    client.initialize(&admin, &bounty_per_key);

    // ── Step 2: Set up bounty pool ──────────────────────────────────────────
    let (token_addr, token, token_admin) = create_token(&env, &admin);
    let pool_amount = 1_000_000i128;
    token_admin.mint(&admin, &pool_amount);
    client.deposit_bounty(&pool_amount, &token_addr);
    assert_eq!(client.view_bounty_balance(), pool_amount);

    // ── Step 3: Register keys ───────────────────────────────────────────────
    let threshold = 200u32;
    let extension = 1000u32;

    let mut all_ids = Vec::new(&env);
    for _ in 0..3 {
        let id = client.register(
            &env.register(TtlBumperContract, ()),
            &threshold,
            &extension,
            &owner_a,
        );
        all_ids.push_back(id);
    }
    for _ in 0..2 {
        let id = client.register(
            &env.register(TtlBumperContract, ()),
            &threshold,
            &extension,
            &owner_b,
        );
        all_ids.push_back(id);
    }
    assert_eq!(client.view_active_count(), 5u32);

    // ── Step 4: Simulate ledger advancement ─────────────────────────────────
    env.ledger().with_mut(|info| {
        info.sequence_number = info.sequence_number.saturating_add(500);
        info.timestamp = info.timestamp.saturating_add(500 * 5);
    });

    // ── Step 5: Keeper submits bump batch ───────────────────────────────────
    assert_eq!(token.balance(&keeper), 0i128);
    let bounty_earned = client.bump_batch(&keeper, &all_ids, &Some(token_addr.clone()));

    // ── Step 6: Verify bounty payout ────────────────────────────────────────
    let expected_bounty = (bounty_per_key * 5) as u64;
    assert_eq!(bounty_earned, expected_bounty);
    assert_eq!(token.balance(&keeper), bounty_per_key * 5);
    let expected_remaining = pool_amount - bounty_per_key * 5;
    assert_eq!(client.view_bounty_balance(), expected_remaining);

    // ── Step 7: State consistency ───────────────────────────────────────────
    assert_eq!(client.view_active_count(), 5u32);
    assert_eq!(client.view_active_entries().len(), 5);

    // Owner B deregisters one of their entries.
    client.deregister(&all_ids.get(3).unwrap(), &owner_b);
    assert_eq!(client.view_active_count(), 4u32);
}

/// Keeper targets only entries it considers near expiry (off-chain filtering).
#[test]
fn test_keeper_targets_near_expiry_only() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(TtlBumperContract, ());
    let client = TtlBumperContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let keeper = Address::generate(&env);
    let owner = Address::generate(&env);

    let bounty_per_key = 50_000i128;
    client.initialize(&admin, &bounty_per_key);

    let (token_addr, token, token_admin) = create_token(&env, &admin);
    token_admin.mint(&admin, &500_000i128);
    client.deposit_bounty(&500_000i128, &token_addr);

    // Register 5 entries.
    let mut all_ids = Vec::new(&env);
    for _ in 0..5 {
        let id = client.register(
            &env.register(TtlBumperContract, ()),
            &100u32,
            &500u32,
            &owner,
        );
        all_ids.push_back(id);
    }

    // Keeper only bumps the first 2 (simulating off-chain near-expiry filter).
    let targeted_ids = vec![
        &env,
        all_ids.get(0).unwrap(),
        all_ids.get(1).unwrap(),
    ];

    let bounty = client.bump_batch(&keeper, &targeted_ids, &Some(token_addr));
    assert_eq!(bounty, (bounty_per_key * 2) as u64);
    assert_eq!(token.balance(&keeper), bounty_per_key * 2);

    // All 5 entries are still active (bumping doesn't deregister).
    assert_eq!(client.view_active_count(), 5u32);
}
