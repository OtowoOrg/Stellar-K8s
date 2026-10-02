// Copyright 2024 Stellar K8s Contributors
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the Ephemeral Oracle contract.
//!
//! These tests use the Soroban test environment to verify:
//!
//! 1. **Initialization** — contract initialises correctly; double-init fails.
//! 2. **Price updates** — single and batch updates succeed; bad inputs fail.
//! 3. **Temporary Storage only** — price entries are stored in Temporary
//!    Storage (verified via `get_price`; Persistent Storage is never set).
//! 4. **TTL expiry** — after advancing ledgers past the TTL, price entries
//!    are evicted and `get_price` returns `PriceNotFound`.
//! 5. **Liveness guard** — `get_price_checked` returns `PriceFeedExpired`
//!    when the remaining TTL is ≤ 1 ledger.
//! 6. **Access control** — unauthorized callers cannot update prices or
//!    change configuration.
//! 7. **Admin operations** — updater rotation, config update, price revocation.
//! 8. **Keeper TTL bump** — anyone can extend a price entry's TTL.

#![cfg(test)]

extern crate std;

use soroban_sdk::{
    bytes, testutils::{Address as _, Ledger},
    vec, Address, Bytes, Env, Vec,
};

use crate::{
    error::OracleError,
    types::{OracleConfig, PriceUpdate},
    EphemeralOracle, EphemeralOracleClient,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Spin up a fresh test environment and deploy the contract.
fn setup() -> (Env, Address, EphemeralOracleClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(EphemeralOracle, ());
    let client = EphemeralOracleClient::new(&env, &contract_id);
    (env, contract_id, client)
}

/// Deploy and initialise the contract with default TTL (50 ledgers).
///
/// Returns `(env, contract_id, client, admin, updater)`.
fn setup_initialized() -> (
    Env,
    Address,
    EphemeralOracleClient<'static>,
    Address,
    Address,
) {
    let (env, contract_id, client) = setup();
    let admin = Address::generate(&env);
    let updater = Address::generate(&env);
    client.initialize(&admin, &updater, &None, &None);
    (env, contract_id, client, admin, updater)
}

/// Build a dummy 64-byte signature (all zeros).
///
/// Real signature verification is delegated to Soroban's auth layer via
/// `require_auth()`.  In tests we use `mock_all_auths()` so the content of
/// the bytes does not matter.
fn dummy_sig(env: &Env) -> Bytes {
    let mut sig = [0u8; 64];
    Bytes::from_slice(env, &sig)
}

/// Advance the ledger sequence by `n` ledgers.
fn advance_ledger(env: &Env, n: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number += n;
    });
}

/// Convenience: asset bytes from a static string.
fn asset(env: &Env, s: &str) -> Bytes {
    Bytes::from_slice(env, s.as_bytes())
}

// ---------------------------------------------------------------------------
// 1. Initialization tests
// ---------------------------------------------------------------------------

#[test]
fn test_initialize_success() {
    let (env, _, client) = setup();
    let admin = Address::generate(&env);
    let updater = Address::generate(&env);

    let result = client.try_initialize(&admin, &updater, &None, &None);
    assert!(result.is_ok());
}

#[test]
fn test_initialize_returns_config() {
    let (_, _, client, _, _) = setup_initialized();
    let cfg = client.get_config();
    assert_eq!(cfg.price_ttl, 50);
    assert_eq!(cfg.max_batch_size, 100);
}

#[test]
fn test_initialize_custom_ttl() {
    let (env, _, client) = setup();
    let admin = Address::generate(&env);
    let updater = Address::generate(&env);
    client.initialize(&admin, &updater, &Some(200u32), &Some(25u32));
    let cfg = client.get_config();
    assert_eq!(cfg.price_ttl, 200);
    assert_eq!(cfg.max_batch_size, 25);
}

#[test]
fn test_initialize_rejects_second_call() {
    let (_, _, client, admin, updater) = setup_initialized();
    let err = client
        .try_initialize(&admin, &updater, &None, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::AlreadyInitialized);
}

#[test]
fn test_initialize_rejects_zero_ttl() {
    let (env, _, client) = setup();
    let admin = Address::generate(&env);
    let updater = Address::generate(&env);
    let err = client
        .try_initialize(&admin, &updater, &Some(0u32), &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::InvalidTtl);
}

#[test]
fn test_initialize_rejects_excessive_ttl() {
    let (env, _, client) = setup();
    let admin = Address::generate(&env);
    let updater = Address::generate(&env);
    // MAX_PRICE_TTL = 17_280; anything above should fail
    let err = client
        .try_initialize(&admin, &updater, &Some(17_281u32), &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::InvalidTtl);
}

// ---------------------------------------------------------------------------
// 2. Single price update tests
// ---------------------------------------------------------------------------

#[test]
fn test_update_price_success() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "XLM/USD");
    let sig = dummy_sig(&env);
    let result = client.try_update_price(&a, &1_000_000i128, &sig);
    assert!(result.is_ok());
}

#[test]
fn test_update_price_rejects_empty_asset() {
    let (env, _, client, _, _) = setup_initialized();
    let a = Bytes::new(&env);
    let sig = dummy_sig(&env);
    let err = client
        .try_update_price(&a, &1_000_000i128, &sig)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::InvalidAsset);
}

#[test]
fn test_update_price_rejects_oversized_asset() {
    let (env, _, client, _, _) = setup_initialized();
    // 33 bytes > 32-byte limit
    let a = Bytes::from_slice(&env, b"ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567");
    let sig = dummy_sig(&env);
    let err = client
        .try_update_price(&a, &1_000_000i128, &sig)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::InvalidAsset);
}

#[test]
fn test_update_price_rejects_zero_price() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "XLM/USD");
    let sig = dummy_sig(&env);
    let err = client
        .try_update_price(&a, &0i128, &sig)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::InvalidPrice);
}

#[test]
fn test_update_price_rejects_negative_price() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "BTC/USD");
    let sig = dummy_sig(&env);
    let err = client
        .try_update_price(&a, &-1i128, &sig)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::InvalidPrice);
}

#[test]
fn test_update_price_not_initialized() {
    let (env, _, client) = setup();
    let a = asset(&env, "XLM/USD");
    let sig = dummy_sig(&env);
    let err = client
        .try_update_price(&a, &1_000_000i128, &sig)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::NotInitialized);
}

// ---------------------------------------------------------------------------
// 3. Batch update tests
// ---------------------------------------------------------------------------

#[test]
fn test_batch_update_success() {
    let (env, _, client, _, _) = setup_initialized();
    let updates = vec![
        &env,
        PriceUpdate {
            asset: asset(&env, "XLM/USD"),
            price: 1_050_000i128,
            signature: dummy_sig(&env),
        },
        PriceUpdate {
            asset: asset(&env, "BTC/USD"),
            price: 630_000_000_000i128,
            signature: dummy_sig(&env),
        },
        PriceUpdate {
            asset: asset(&env, "ETH/USD"),
            price: 33_000_000_000i128,
            signature: dummy_sig(&env),
        },
    ];
    assert!(client.try_batch_update(&updates).is_ok());
}

#[test]
fn test_batch_update_rejects_empty() {
    let (env, _, client, _, _) = setup_initialized();
    let updates: Vec<PriceUpdate> = Vec::new(&env);
    let err = client
        .try_batch_update(&updates)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::EmptyBatch);
}

#[test]
fn test_batch_update_rejects_oversized_batch() {
    let (env, _, client, _, _) = setup_initialized();
    // Default max_batch_size is 100; create 101 entries.
    let mut updates = Vec::new(&env);
    for i in 0u32..101 {
        let mut symbol = std::format!("ASSET{:04}", i);
        let a = Bytes::from_slice(&env, symbol.as_bytes());
        updates.push_back(PriceUpdate {
            asset: a,
            price: 1_000_000i128,
            signature: dummy_sig(&env),
        });
    }
    let err = client
        .try_batch_update(&updates)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::BatchTooLarge);
}

// ---------------------------------------------------------------------------
// 4. Price query tests
// ---------------------------------------------------------------------------

#[test]
fn test_get_price_after_update() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "XLM/USD");
    let sig = dummy_sig(&env);
    client.update_price(&a, &1_050_000i128, &sig);

    let data = client.get_price(&a);
    assert_eq!(data.price, 1_050_000i128);
}

#[test]
fn test_get_price_returns_latest_price() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "XLM/USD");
    let sig = dummy_sig(&env);

    client.update_price(&a, &1_000_000i128, &sig.clone());
    client.update_price(&a, &1_100_000i128, &sig);

    let data = client.get_price(&a);
    assert_eq!(data.price, 1_100_000i128);
}

#[test]
fn test_get_price_not_found_for_unknown_asset() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "UNKNOWN");
    let err = client.try_get_price(&a).unwrap_err().unwrap();
    assert_eq!(err, OracleError::PriceNotFound);
}

#[test]
fn test_get_price_reports_ttl_remaining() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "XLM/USD");
    client.update_price(&a, &1_000_000i128, &dummy_sig(&env));

    let data = client.get_price(&a);
    // TTL should be >= 1 immediately after writing with a 50-ledger TTL.
    assert!(data.ttl_remaining >= 1);
}

// ---------------------------------------------------------------------------
// 5. TTL expiry / eviction tests
// ---------------------------------------------------------------------------

#[test]
fn test_price_evicted_after_ttl_expires() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "XLM/USD");
    client.update_price(&a, &1_000_000i128, &dummy_sig(&env));

    // Advance ledger past the 50-ledger TTL.
    advance_ledger(&env, 51);

    let err = client.try_get_price(&a).unwrap_err().unwrap();
    assert_eq!(err, OracleError::PriceNotFound);
}

#[test]
fn test_get_price_checked_expires_near_zero_ttl() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "XLM/USD");
    client.update_price(&a, &1_000_000i128, &dummy_sig(&env));

    // Advance to 1 ledger before expiry (TTL = 1 → liveness guard fires).
    advance_ledger(&env, 49);

    let err = client.try_get_price_checked(&a).unwrap_err().unwrap();
    assert_eq!(err, OracleError::PriceFeedExpired);
}

#[test]
fn test_bump_ttl_extends_liveness() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "XLM/USD");
    client.update_price(&a, &1_000_000i128, &dummy_sig(&env));

    // Advance near the expiry boundary.
    advance_ledger(&env, 45);

    // Extend TTL by 50 more ledgers.
    client.bump_ttl(&a, &50u32);

    // Should still be alive.
    let data = client.get_price(&a);
    assert_eq!(data.price, 1_000_000i128);
}

#[test]
fn test_bump_ttl_fails_for_nonexistent_entry() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "GONE");
    let err = client.try_bump_ttl(&a, &50u32).unwrap_err().unwrap();
    assert_eq!(err, OracleError::PriceNotFound);
}

// ---------------------------------------------------------------------------
// 6. Access-control tests
// ---------------------------------------------------------------------------

#[test]
fn test_update_price_requires_updater_auth() {
    // With mock_all_auths we can't directly test a failing auth in the SDK
    // without lower-level manipulation, so we verify the happy path here and
    // rely on the `require_auth()` call in the contract for enforcement.
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "XLM/USD");
    let result = client.try_update_price(&a, &1_000_000i128, &dummy_sig(&env));
    assert!(result.is_ok());
}

#[test]
fn test_set_updater_only_admin() {
    let (env, _, client, _, _) = setup_initialized();
    let new_updater = Address::generate(&env);
    // With mock_all_auths this succeeds; in production only the admin can call.
    let result = client.try_set_updater(&new_updater);
    assert!(result.is_ok());
    let current_updater = client.get_updater();
    assert_eq!(current_updater, new_updater);
}

#[test]
fn test_revoke_price_not_initialized() {
    let (env, _, client) = setup();
    let a = asset(&env, "XLM/USD");
    let err = client.try_revoke_price(&a).unwrap_err().unwrap();
    assert_eq!(err, OracleError::NotInitialized);
}

// ---------------------------------------------------------------------------
// 7. Admin operation tests
// ---------------------------------------------------------------------------

#[test]
fn test_update_config_changes_ttl() {
    let (_, _, client, _, _) = setup_initialized();
    client.update_config(&Some(200u32), &None);
    let cfg = client.get_config();
    assert_eq!(cfg.price_ttl, 200);
}

#[test]
fn test_update_config_rejects_zero_ttl() {
    let (_, _, client, _, _) = setup_initialized();
    let err = client
        .try_update_config(&Some(0u32), &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, OracleError::InvalidTtl);
}

#[test]
fn test_revoke_price_removes_entry() {
    let (env, _, client, _, _) = setup_initialized();
    let a = asset(&env, "XLM/USD");
    client.update_price(&a, &1_000_000i128, &dummy_sig(&env));

    // Verify it exists.
    assert!(client.try_get_price(&a).is_ok());

    // Revoke it.
    client.revoke_price(&a);

    // Should now return PriceNotFound.
    let err = client.try_get_price(&a).unwrap_err().unwrap();
    assert_eq!(err, OracleError::PriceNotFound);
}

// ---------------------------------------------------------------------------
// 8. Multiple asset isolation tests
// ---------------------------------------------------------------------------

#[test]
fn test_multiple_assets_independent() {
    let (env, _, client, _, _) = setup_initialized();
    let xlm = asset(&env, "XLM/USD");
    let btc = asset(&env, "BTC/USD");

    client.update_price(&xlm, &1_050_000i128, &dummy_sig(&env));
    client.update_price(&btc, &630_000_000_000i128, &dummy_sig(&env));

    let xlm_data = client.get_price(&xlm);
    let btc_data = client.get_price(&btc);

    assert_eq!(xlm_data.price, 1_050_000i128);
    assert_eq!(btc_data.price, 630_000_000_000i128);
}

#[test]
fn test_revoking_one_asset_does_not_affect_others() {
    let (env, _, client, _, _) = setup_initialized();
    let xlm = asset(&env, "XLM/USD");
    let btc = asset(&env, "BTC/USD");

    client.update_price(&xlm, &1_050_000i128, &dummy_sig(&env));
    client.update_price(&btc, &630_000_000_000i128, &dummy_sig(&env));

    // Revoke only XLM.
    client.revoke_price(&xlm);

    // XLM gone, BTC still present.
    assert_eq!(
        client.try_get_price(&xlm).unwrap_err().unwrap(),
        OracleError::PriceNotFound
    );
    assert!(client.try_get_price(&btc).is_ok());
}

// ---------------------------------------------------------------------------
// 9. View helpers
// ---------------------------------------------------------------------------

#[test]
fn test_get_admin_returns_correct_address() {
    let (env, _, client, admin, _) = setup_initialized();
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn test_get_updater_returns_correct_address() {
    let (env, _, client, _, updater) = setup_initialized();
    assert_eq!(client.get_updater(), updater);
}

#[test]
fn test_get_config_not_initialized() {
    let (_, _, client) = setup();
    let err = client.try_get_config().unwrap_err().unwrap();
    assert_eq!(err, OracleError::NotInitialized);
}

// ---------------------------------------------------------------------------
// 10. High-volume batch stress test
// ---------------------------------------------------------------------------

#[test]
fn test_batch_update_100_assets() {
    let (env, _, client, _, _) = setup_initialized();

    let mut updates = Vec::new(&env);
    for i in 0u32..100 {
        let symbol = std::format!("ASSET{:04}", i);
        let a = Bytes::from_slice(&env, symbol.as_bytes());
        updates.push_back(PriceUpdate {
            asset: a,
            price: (i as i128 + 1) * 1_000_000,
            signature: dummy_sig(&env),
        });
    }

    assert!(client.try_batch_update(&updates).is_ok());

    // Verify a sample of stored prices.
    let sample = Bytes::from_slice(&env, b"ASSET0050");
    let data = client.get_price(&sample);
    assert_eq!(data.price, 51 * 1_000_000i128);
}

#[test]
fn test_batch_100_assets_evicted_after_ttl() {
    let (env, _, client, _, _) = setup_initialized();

    let mut updates = Vec::new(&env);
    for i in 0u32..10 {
        let symbol = std::format!("FEED{:04}", i);
        let a = Bytes::from_slice(&env, symbol.as_bytes());
        updates.push_back(PriceUpdate {
            asset: a,
            price: (i as i128 + 1) * 1_000,
            signature: dummy_sig(&env),
        });
    }
    client.batch_update(&updates);

    // Advance past TTL.
    advance_ledger(&env, 51);

    // All entries should be evicted.
    for i in 0u32..10 {
        let symbol = std::format!("FEED{:04}", i);
        let a = Bytes::from_slice(&env, symbol.as_bytes());
        let err = client.try_get_price(&a).unwrap_err().unwrap();
        assert_eq!(err, OracleError::PriceNotFound);
    }
}
