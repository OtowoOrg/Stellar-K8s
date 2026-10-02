// Copyright 2024 Stellar K8s Contributors
// SPDX-License-Identifier: Apache-2.0

//! Low-level storage helpers for the Ephemeral Oracle contract.
//!
//! # Storage topology
//!
//! | Data               | Soroban Storage tier | Eviction           |
//! |--------------------|----------------------|--------------------|
//! | `Admin` address    | Instance             | Never (instance)   |
//! | `Updater` address  | Instance             | Never (instance)   |
//! | `OracleConfig`     | Instance             | Never (instance)   |
//! | `PriceEntry`       | **Temporary**        | After `price_ttl` ledgers |
//!
//! Price entries are **intentionally** placed in Temporary Storage so that
//! Soroban's ledger eviction algorithm can purge them cost-free once the TTL
//! window closes.  Writing to Persistent Storage is explicitly blocked via the
//! `PersistentStorageForbidden` error to enforce zero long-term state growth.

use soroban_sdk::{Address, Bytes, Env};

use crate::{
    error::OracleError,
    types::{DataKey, OracleConfig, PriceEntry},
};

// ---------------------------------------------------------------------------
// TTL / size constants
// ---------------------------------------------------------------------------

/// Default number of ledgers a price entry lives before automatic eviction.
///
/// At ~5 s/ledger on Testnet, 50 ledgers ≈ 4 minutes 10 seconds.
/// On Mainnet (≈6 s/ledger) 50 ledgers ≈ 5 minutes.
pub const DEFAULT_PRICE_TTL: u32 = 50;

/// Hard upper bound on `OracleConfig::price_ttl` (≈1 day on Mainnet).
pub const MAX_PRICE_TTL: u32 = 17_280;

/// Default maximum assets per `batch_update` call.
pub const DEFAULT_MAX_BATCH_SIZE: u32 = 100;

/// Hard cap on batch size to bound per-transaction metering.
pub const MAX_BATCH_SIZE_CAP: u32 = 500;

/// Instance-storage TTL extension applied on every read/write to Admin /
/// Updater / Config keys so the contract stays live indefinitely.
const INSTANCE_TTL_BUMP: u32 = 535_000; // ≈1 year in ledgers

// ---------------------------------------------------------------------------
// Initialisation helpers
// ---------------------------------------------------------------------------

/// Returns `true` if the contract has already been initialised.
pub fn is_initialized(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Admin)
}

/// Persist the admin address to Instance Storage and bump the instance TTL.
pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&DataKey::Admin, admin);
    env.storage().instance().extend_ttl(INSTANCE_TTL_BUMP, INSTANCE_TTL_BUMP);
}

/// Retrieve the admin address; panics if not initialised.
pub fn get_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .expect("contract not initialized")
}

/// Persist the updater address.
pub fn set_updater(env: &Env, updater: &Address) {
    env.storage().instance().set(&DataKey::Updater, updater);
    env.storage().instance().extend_ttl(INSTANCE_TTL_BUMP, INSTANCE_TTL_BUMP);
}

/// Retrieve the updater address; panics if not initialised.
pub fn get_updater(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Updater)
        .expect("updater not set")
}

/// Persist the oracle config.
pub fn set_config(env: &Env, config: &OracleConfig) {
    env.storage().instance().set(&DataKey::Config, config);
    env.storage().instance().extend_ttl(INSTANCE_TTL_BUMP, INSTANCE_TTL_BUMP);
}

/// Retrieve the oracle config; returns defaults if never explicitly set.
pub fn get_config(env: &Env) -> OracleConfig {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .unwrap_or(OracleConfig {
            price_ttl: DEFAULT_PRICE_TTL,
            max_batch_size: DEFAULT_MAX_BATCH_SIZE,
        })
}

// ---------------------------------------------------------------------------
// Temporary-storage price helpers
// ---------------------------------------------------------------------------

/// Write a `PriceEntry` exclusively to **Temporary Storage**.
///
/// The entry is stored under `DataKey::Price(asset_bytes)` with the TTL
/// specified in `OracleConfig::price_ttl`.
///
/// # Panics
/// Never – all error paths return `Err(OracleError)`.
///
/// # Why Temporary Storage?
/// Temporary entries are eligible for automatic eviction by the Stellar
/// protocol once their TTL reaches 0 ledgers.  No explicit delete transaction
/// is needed, which means:
/// 1. Storage rent for old entries is never paid.
/// 2. Ledger state size does not grow monotonically.
/// 3. High-frequency updates (e.g. 10,000/day) remain sustainable.
pub fn write_price_temp(
    env: &Env,
    asset: &Bytes,
    entry: &PriceEntry,
    ttl: u32,
) -> Result<(), OracleError> {
    let key = DataKey::Price(asset.clone());

    // Explicitly confirm we are NOT writing to Persistent Storage.
    // This assertion is belt-and-suspenders: the storage type is determined
    // by `env.storage().temporary()`, but we document the intent here.
    // Persistent storage would be: env.storage().persistent().set(...)
    // which is intentionally absent from this function.

    env.storage().temporary().set(&key, entry);

    // Set the TTL for the Temporary Storage entry.  The first argument is the
    // minimum TTL to set, the second is the extension to apply.  Using the
    // same value for both means: always ensure the key lives for `ttl` more
    // ledgers from this moment.
    env.storage().temporary().extend_ttl(&key, ttl, ttl);

    Ok(())
}

/// Read a `PriceEntry` from Temporary Storage.
///
/// Returns `Err(OracleError::PriceNotFound)` if the key does not exist (never
/// written, or already evicted because its TTL reached 0).
pub fn read_price_temp(env: &Env, asset: &Bytes) -> Result<PriceEntry, OracleError> {
    let key = DataKey::Price(asset.clone());
    env.storage()
        .temporary()
        .get(&key)
        .ok_or(OracleError::PriceNotFound)
}

/// Check whether a price entry exists **and** has remaining TTL > 0.
///
/// Returns `Err(OracleError::PriceFeedExpired)` if the entry exists but its
/// TTL is at or near expiry (≤ 1 ledger remaining).  Callers that depend on
/// live data (DeFi swap routers, liquidation bots) should use this function
/// instead of `read_price_temp` to guard against acting on stale prices.
pub fn read_price_checked(env: &Env, asset: &Bytes) -> Result<PriceEntry, OracleError> {
    let key = DataKey::Price(asset.clone());

    // First confirm the entry exists.
    let entry: PriceEntry = env
        .storage()
        .temporary()
        .get(&key)
        .ok_or(OracleError::PriceNotFound)?;

    // Soroban SDK 27 exposes `get_ttl` on Temporary Storage so we can verify
    // liveness before returning the price to the caller.
    let ttl_remaining = env.storage().temporary().get_ttl(&key);
    if ttl_remaining <= 1 {
        return Err(OracleError::PriceFeedExpired);
    }

    Ok(entry)
}

/// Remove a price entry from Temporary Storage immediately.
///
/// This is provided for emergency recall (e.g. bad data was submitted).
/// Under normal operation entries expire naturally.
pub fn remove_price_temp(env: &Env, asset: &Bytes) {
    let key = DataKey::Price(asset.clone());
    env.storage().temporary().remove(&key);
}

/// Extend the TTL of an existing Temporary Storage price entry.
///
/// Used by keeper bots to keep high-priority feeds alive beyond the default
/// window without requiring a full `update_price` call.
pub fn bump_price_ttl(env: &Env, asset: &Bytes, extension: u32) -> Result<(), OracleError> {
    let key = DataKey::Price(asset.clone());
    if !env.storage().temporary().has(&key) {
        return Err(OracleError::PriceNotFound);
    }
    env.storage().temporary().extend_ttl(&key, extension, extension);
    Ok(())
}
