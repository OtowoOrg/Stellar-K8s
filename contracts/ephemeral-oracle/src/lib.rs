// Copyright 2024 Stellar K8s Contributors
// SPDX-License-Identifier: Apache-2.0

//! # Ephemeral Oracle — Temporary Storage Price Feed Contract
//!
//! A high-frequency oracle that ingests signed off-chain price data and stores
//! it **exclusively** in Soroban [Temporary Storage], so it is automatically
//! evicted by the network after a configurable TTL window (default 50 ledgers
//! ≈ 5 minutes).  Persistent Storage is never written, guaranteeing
//! **zero long-term state growth** regardless of update frequency.
//!
//! ## Design goals
//!
//! 1. **Zero-growth state footprint** — all price data lives in Temporary
//!    Storage.  Old entries are purged by the ledger eviction engine at no
//!    cost to the operator.
//!
//! 2. **Signed price feed integrity** — the registered `updater` address must
//!    `require_auth()` on every `update_price` and `batch_update` call,
//!    leveraging Soroban's native account auth.
//!
//! 3. **DeFi-safe expiry checks** — `get_price_checked` reverts with
//!    `PriceFeedExpired` if the TTL is at or near 0, preventing dependent
//!    contracts from acting on stale prices.
//!
//! 4. **High-frequency updates** — a single `batch_update` processes up to
//!    `OracleConfig::max_batch_size` assets (hard cap 500) in one transaction.
//!
//! ## Storage rent comparison
//!
//! | Storage type  | Rent model          | Long-term cost for 10 k updates/day |
//! |---------------|---------------------|-------------------------------------|
//! | Persistent    | Pay once to create, then rent every ~12 h | Grows linearly |
//! | **Temporary** | Pay TTL-proportional rent, **no renewal**  | **Constant** (entries evicted) |
//!
//! At 50-ledger TTL, the entry disappears automatically after ≈5 minutes.
//! Persistent storage for the same key would require recurring rent payments
//! and would accumulate ledger bloat proportional to the number of unique
//! assets tracked over time.
//!
//! [Temporary Storage]: https://developers.stellar.org/docs/smart-contracts/storage/state-archival

#![no_std]

use soroban_sdk::{contract, contractimpl, Address, Bytes, Env, String, Vec};

pub mod error;
pub mod storage;
pub mod types;

#[cfg(test)]
mod test;

use error::OracleError;
use storage::{
    bump_price_ttl, get_admin, get_config, get_updater, is_initialized, read_price_checked,
    read_price_temp, remove_price_temp, set_admin, set_config, set_updater, write_price_temp,
    DEFAULT_MAX_BATCH_SIZE, DEFAULT_PRICE_TTL, MAX_BATCH_SIZE_CAP, MAX_PRICE_TTL,
};
use types::{OracleConfig, PriceData, PriceEntry, PriceUpdate};

// ---------------------------------------------------------------------------
// Contract definition
// ---------------------------------------------------------------------------

#[contract]
pub struct EphemeralOracle;

#[contractimpl]
impl EphemeralOracle {
    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Initialise the oracle contract.
    ///
    /// Must be called exactly once.  Stores the `admin` and `updater`
    /// addresses in Instance Storage and writes an `OracleConfig` with
    /// sensible defaults (or with the supplied overrides).
    ///
    /// # Arguments
    ///
    /// * `admin`   — privileged address for config changes and updater
    ///   rotation.
    /// * `updater` — address whose `require_auth()` is checked on every price
    ///   write.  May be the same as `admin` for simple deployments.
    /// * `price_ttl` — optional TTL override (ledgers); defaults to 50.
    /// * `max_batch_size` — optional batch cap override; defaults to 100.
    ///
    /// # Errors
    ///
    /// * `AlreadyInitialized` — if called a second time.
    /// * `InvalidTtl` — if `price_ttl` is 0 or > `MAX_PRICE_TTL`.
    pub fn initialize(
        env: Env,
        admin: Address,
        updater: Address,
        price_ttl: Option<u32>,
        max_batch_size: Option<u32>,
    ) -> Result<(), OracleError> {
        if is_initialized(&env) {
            return Err(OracleError::AlreadyInitialized);
        }

        admin.require_auth();

        let ttl = price_ttl.unwrap_or(DEFAULT_PRICE_TTL);
        if ttl == 0 || ttl > MAX_PRICE_TTL {
            return Err(OracleError::InvalidTtl);
        }

        let batch_cap = max_batch_size
            .unwrap_or(DEFAULT_MAX_BATCH_SIZE)
            .min(MAX_BATCH_SIZE_CAP);

        set_admin(&env, &admin);
        set_updater(&env, &updater);
        set_config(
            &env,
            &OracleConfig {
                price_ttl: ttl,
                max_batch_size: batch_cap,
            },
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Admin operations
    // -----------------------------------------------------------------------

    /// Rotate the updater address.
    ///
    /// Only callable by the admin.
    pub fn set_updater(env: Env, new_updater: Address) -> Result<(), OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        set_updater(&env, &new_updater);
        Ok(())
    }

    /// Update oracle runtime configuration.
    ///
    /// Only callable by the admin.
    ///
    /// # Errors
    ///
    /// * `NotInitialized` — contract not yet initialised.
    /// * `Unauthorized` — caller is not the admin.
    /// * `InvalidTtl` — if `price_ttl` is 0 or > `MAX_PRICE_TTL`.
    pub fn update_config(
        env: Env,
        price_ttl: Option<u32>,
        max_batch_size: Option<u32>,
    ) -> Result<(), OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();

        let mut config = get_config(&env);

        if let Some(ttl) = price_ttl {
            if ttl == 0 || ttl > MAX_PRICE_TTL {
                return Err(OracleError::InvalidTtl);
            }
            config.price_ttl = ttl;
        }
        if let Some(bs) = max_batch_size {
            config.max_batch_size = bs.min(MAX_BATCH_SIZE_CAP);
        }
        set_config(&env, &config);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Price ingestion
    // -----------------------------------------------------------------------

    /// Submit a single price update for one asset.
    ///
    /// The `updater` address must `require_auth()`.  The entry is written
    /// **exclusively** to Soroban Temporary Storage with the TTL from the
    /// current `OracleConfig`.
    ///
    /// # Arguments
    ///
    /// * `asset`     — raw asset symbol bytes (e.g. `b"XLM/USD"`).
    /// * `price`     — price in smallest unit (e.g. × 10^7 for USD pairs).
    /// * `signature` — 64-byte Ed25519 signature over `(asset ‖ price ‖ ledger)`.
    ///
    /// # Errors
    ///
    /// * `NotInitialized` — contract not yet initialised.
    /// * `Unauthorized` — auth check on the updater failed.
    /// * `InvalidAsset` — empty or oversized asset bytes.
    /// * `InvalidPrice` — price is 0 or negative.
    pub fn update_price(
        env: Env,
        asset: Bytes,
        price: i128,
        signature: Bytes,
    ) -> Result<(), OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }

        // Validate inputs before requiring auth to fail fast on bad data.
        if asset.is_empty() || asset.len() > 32 {
            return Err(OracleError::InvalidAsset);
        }
        if price <= 0 {
            return Err(OracleError::InvalidPrice);
        }

        let updater = get_updater(&env);
        updater.require_auth();

        let config = get_config(&env);
        let current_ledger = env.ledger().sequence();

        let entry = PriceEntry {
            asset: asset.clone(),
            price,
            timestamp_ledger: current_ledger,
            signature,
            signer: updater,
        };

        // Write ONLY to Temporary Storage — never to Persistent Storage.
        write_price_temp(&env, &asset, &entry, config.price_ttl)?;

        // Emit an event so off-chain indexers can track the update without
        // needing to query ledger state.
        env.events().publish(
            (soroban_sdk::symbol_short!("PRICE_UPD"),),
            (asset, price, current_ledger),
        );

        Ok(())
    }

    /// Submit price updates for multiple assets in a single transaction.
    ///
    /// Processes up to `OracleConfig::max_batch_size` entries.  All entries
    /// share a single `require_auth()` from the updater address.
    ///
    /// # Arguments
    ///
    /// * `updates` — vector of `PriceUpdate` structs (asset + price + sig).
    ///
    /// # Errors
    ///
    /// * `EmptyBatch` — if `updates` is empty.
    /// * `BatchTooLarge` — if `updates.len() > max_batch_size`.
    /// * All single-price errors propagated for each entry.
    pub fn batch_update(env: Env, updates: Vec<PriceUpdate>) -> Result<(), OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }
        if updates.is_empty() {
            return Err(OracleError::EmptyBatch);
        }

        let config = get_config(&env);
        if updates.len() > config.max_batch_size {
            return Err(OracleError::BatchTooLarge);
        }

        let updater = get_updater(&env);
        updater.require_auth();

        let current_ledger = env.ledger().sequence();

        for update in updates.iter() {
            let asset = update.asset.clone();
            let price = update.price;

            if asset.is_empty() || asset.len() > 32 {
                return Err(OracleError::InvalidAsset);
            }
            if price <= 0 {
                return Err(OracleError::InvalidPrice);
            }

            let entry = PriceEntry {
                asset: asset.clone(),
                price,
                timestamp_ledger: current_ledger,
                signature: update.signature.clone(),
                signer: updater.clone(),
            };

            write_price_temp(&env, &asset, &entry, config.price_ttl)?;

            env.events().publish(
                (soroban_sdk::symbol_short!("PRICE_UPD"),),
                (asset, price, current_ledger),
            );
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Price queries
    // -----------------------------------------------------------------------

    /// Return the latest price for `asset` without liveness checks.
    ///
    /// If the entry has already been evicted (TTL = 0) this returns
    /// `Err(PriceNotFound)`.  Use `get_price_checked` in DeFi contexts.
    pub fn get_price(env: Env, asset: Bytes) -> Result<PriceData, OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }
        let entry = read_price_temp(&env, &asset)?;
        let ttl = env
            .storage()
            .temporary()
            .get_ttl(&types::DataKey::Price(asset.clone()));

        // Convert the raw asset bytes to a readable String for the response.
        let asset_str = String::from_bytes(&env, &asset);

        Ok(PriceData {
            asset: asset_str,
            price: entry.price,
            timestamp_ledger: entry.timestamp_ledger,
            ttl_remaining: ttl,
        })
    }

    /// Return the latest price for `asset` with an expiry liveness guard.
    ///
    /// Returns `Err(PriceFeedExpired)` if the remaining TTL is ≤ 1 ledger,
    /// preventing DeFi contracts from acting on prices that are about to
    /// vanish from ledger state.
    ///
    /// This is the **recommended** function for use in dependent smart
    /// contracts (swap routers, liquidation engines, etc.).
    pub fn get_price_checked(env: Env, asset: Bytes) -> Result<PriceData, OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }
        let entry = read_price_checked(&env, &asset)?;
        let ttl = env
            .storage()
            .temporary()
            .get_ttl(&types::DataKey::Price(asset.clone()));

        let asset_str = String::from_bytes(&env, &asset);

        Ok(PriceData {
            asset: asset_str,
            price: entry.price,
            timestamp_ledger: entry.timestamp_ledger,
            ttl_remaining: ttl,
        })
    }

    // -----------------------------------------------------------------------
    // Keeper / TTL management
    // -----------------------------------------------------------------------

    /// Extend the TTL of a Temporary Storage price entry.
    ///
    /// Useful for keeper bots that want to keep high-priority feeds alive for
    /// an additional `extension_ledgers` without a full re-price submission.
    /// Anyone may call this — no auth required.
    ///
    /// # Errors
    ///
    /// * `PriceNotFound` — if the entry does not exist (never written or
    ///   already evicted).
    pub fn bump_ttl(env: Env, asset: Bytes, extension_ledgers: u32) -> Result<(), OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }
        bump_price_ttl(&env, &asset, extension_ledgers)
    }

    // -----------------------------------------------------------------------
    // Emergency operations
    // -----------------------------------------------------------------------

    /// Immediately remove a price entry from Temporary Storage.
    ///
    /// Only callable by the admin.  Under normal operation entries expire
    /// naturally; this is for emergency recall (e.g. bad data was submitted).
    pub fn revoke_price(env: Env, asset: Bytes) -> Result<(), OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }
        let admin = get_admin(&env);
        admin.require_auth();
        remove_price_temp(&env, &asset);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // View helpers
    // -----------------------------------------------------------------------

    /// Return the current oracle configuration.
    pub fn get_config(env: Env) -> Result<OracleConfig, OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }
        Ok(get_config(&env))
    }

    /// Return the registered updater address.
    pub fn get_updater(env: Env) -> Result<Address, OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }
        Ok(get_updater(&env))
    }

    /// Return the admin address.
    pub fn get_admin(env: Env) -> Result<Address, OracleError> {
        if !is_initialized(&env) {
            return Err(OracleError::NotInitialized);
        }
        Ok(get_admin(&env))
    }
}
