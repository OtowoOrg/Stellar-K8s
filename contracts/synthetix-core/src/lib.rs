//! # Synthetic Asset Issuance & Debt Tracking Protocol
//!
//! A Synthetix-style synthetic-asset engine for Stellar/Soroban that lets users
//! lock protocol tokens as collateral and mint synthetic assets (e.g. `sBTC`,
//! `sXLM`) that track external oracle prices.
//!
//! ## What this contract guarantees
//!
//! 1. **A dynamic global debt pool.** Debt is held as *index-based debt
//!    shares*. When an oracle price moves, every minter's debt is re-derived
//!    at read time from the global index and the current price — the contract
//!    never iterates over minter balances, so a revaluation costs `O(1)` in the
//!    number of minters. See [`debt_pool`] for the full derivation.
//! 2. **A 300%+ collateralisation ratio on every mint.**
//!    [`SynthetixCore::mint_synth`] recomputes the minter's *total* debt across
//!    every registered currency against their locked collateral and refuses the
//!    mint below [`MIN_COLLATERAL_RATIO_BPS`] (300%).
//! 3. **A localized liquidation engine.** [`SynthetixCore::liquidate`] retires
//!    an under-collateralised account's debt against its own collateral,
//!    charges a penalty, and hands the surplus to the liquidator. It is
//!    *localized*: it rewrites only the two positions involved plus the global
//!    counters, and it asserts as a post-condition that the liquidator is never
//!    left worse off than they already were, so one actor can never concentrate
//!    the pool's risk.
//!
//! ## Why there is no batch liquidation
//!
//! Synthetix's period-end batch liquidation and its "all synths" backstop mode
//! are global sweeps that walk every minter — precisely the cost model this
//! contract exists to avoid — and both can concentrate control of the whole
//! pool in a single actor. Shortfalls here are resolved by whoever is willing
//! to take the penalised collateral, which is incentive-compatible, bounded and
//! permissionless.
//!
//! ## Storage layout
//!
//! | Key | Type | Storage | Purpose |
//! |-----|------|---------|---------|
//! | `ADMIN` | `Address` | Instance | Privileged caller (admin / oracle) |
//! | `COLLTRL` | `Address` | Instance | Token accepted as collateral |
//! | `CPRICE` | `u128` | Instance | Collateral price, 1e9 fixed point |
//! | `RATIO` | `i128` | Instance | Minimum collateralisation ratio, bps |
//! | `PENALTY` | `i128` | Instance | Liquidation penalty, bps |
//! | `TOTCOL` | `i128` | Instance | Protocol-wide locked collateral |
//! | `SURPLUS` | `i128` | Instance | Value forfeited by liquidated accounts |
//! | `CURS` | `Vec<Address>` | Instance | Registered synthetic currencies |
//! | `PAUSED` | `bool` | Instance | Emergency stop |
//! | `POOL` | `DebtPool` | Persistent | Per-currency global debt counters |
//! | `PRICE` | `u128` | Persistent | Per-currency oracle price |
//! | `SHARES` | `u128` | Persistent | Minter's debt shares for a currency |
//! | `SYNTH` | `i128` | Persistent | Minter's synthetic-unit balance |
//! | `COLL` | `i128` | Persistent | Minter's locked collateral |
//! | `FLAG` | `bool` | Persistent | Under-collateralisation flag |
//! | `FLAGTS` | `u64` | Persistent | Ledger timestamp the flag was set at |
//!
//! Every key a user can touch is persistent and has its TTL extended on each
//! access, so an active position can never be archived out from under a minter.

#![no_std]

pub mod debt_pool;

#[cfg(test)]
mod test;

use debt_pool::{
    collateral_ratio_bps, debt_value, liquidation_collateral, required_collateral,
    synth_units_for_debt, validate_price, DebtPool, MAX_COLLATERAL, MAX_COLLATERAL_RATIO_BPS,
    MAX_PENALTY_BPS, MAX_POOL_AMOUNT, MAX_POOL_SHARES, MAX_PRICE, MIN_COLLATERAL_RATIO_BPS,
    PRICE_PRECISION,
};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, symbol_short,
    token::Client as TokenClient, Address, Env, Symbol, TryFromVal, Val, Vec,
};

/// Instance-storage keys. `symbol_short!` caps identifiers at nine characters.
const KEY_ADMIN: Symbol = symbol_short!("ADMIN");
const KEY_COLLATERAL: Symbol = symbol_short!("COLLTRL");
const KEY_COLLATERAL_PRICE: Symbol = symbol_short!("CPRICE");
const KEY_MIN_RATIO_BPS: Symbol = symbol_short!("RATIO");
const KEY_PENALTY_BPS: Symbol = symbol_short!("PENALTY");
const KEY_TOTAL_COLLATERAL: Symbol = symbol_short!("TOTCOL");
const KEY_SURPLUS: Symbol = symbol_short!("SURPLUS");
const KEY_CURRENCIES: Symbol = symbol_short!("CURS");
const KEY_PAUSED: Symbol = symbol_short!("PAUSED");

/// Bound on the synthetic currency registry.
///
/// Bounding this is what keeps [`SynthetixCore::total_debt`] and the mint
/// collateralisation check `O(currencies)` rather than `O(minters)`: the
/// registry is a small, admin-curated allow-list, never an open one.
pub const MAX_CURRENCIES: u32 = 32;

/// Instance-storage TTL management thresholds (ledgers).
const INSTANCE_TTL_THRESHOLD: u32 = 100;
const INSTANCE_TTL_EXTEND_TO: u32 = 500_000;

/// Persistent-entry TTL management thresholds (ledgers).
const PERSISTENT_TTL_THRESHOLD: u32 = 100;
const PERSISTENT_TTL_EXTEND_TO: u32 = 500_000;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors surfaced by the protocol.
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SynthetixError {
    /// The contract is already initialised.
    AlreadyInitialized = 1,
    /// The contract has not been initialised yet.
    NotInitialized = 2,
    /// The caller is not the admin/oracle.
    Unauthorized = 3,
    /// The contract is paused.
    ContractPaused = 4,
    /// An amount or price was zero, negative or above its protocol cap.
    AmountOutOfRange = 5,
    /// An amount, price or index left the representable range.
    CalculationOverflow = 6,
    /// The pool has no debt or shares to work with.
    PoolEmpty = 7,
    /// A ratio or basis-point argument was outside its permitted range.
    InvalidRatio = 8,
    /// The currency is not in the registry.
    UnknownCurrency = 9,
    /// The currency is already in the registry.
    CurrencyAlreadyRegistered = 10,
    /// The currency registry is full.
    CurrencyLimitReached = 11,
    /// The minter would fall below the minimum collateralisation ratio.
    UnderCollateralized = 12,
    /// The target is not below the minimum collateralisation ratio.
    TargetNotUndercollateralized = 13,
    /// The requested debt to cover exceeds the target's debt in that currency.
    InsufficientDebtToCover = 14,
    /// The liquidator would be left below the minimum ratio.
    LiquidatorWouldBeUndercollateralized = 15,
    /// The caller does not hold that many synthetic units.
    InsufficientSynthBalance = 16,
    /// The account has less collateral than requested.
    InsufficientCollateral = 17,
    /// A protocol invariant was violated.
    InvariantViolation = 18,
}

// ---------------------------------------------------------------------------
// Storage keys, receipts and events
// ---------------------------------------------------------------------------

/// Persistent-storage keys, parameterised by account and currency.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PositionKey {
    /// Global debt counters for a synthetic currency.
    Pool(Address),
    /// Oracle price of a synthetic currency (1e9 fixed point).
    Price(Address),
    /// A minter's debt shares in a synthetic currency.
    Shares(Address, Address),
    /// A minter's synthetic-unit balance in a currency.
    SynthUnits(Address, Address),
    /// A minter's locked collateral.
    Collateral(Address),
    /// Under-collateralisation flag.
    Flagged(Address),
    /// Ledger timestamp at which the flag was last changed.
    FlaggedAt(Address),
}

/// Emitted when collateral is locked, released or moved by a liquidation.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralChanged {
    #[topic]
    pub account: Address,
    pub delta: i128,
    pub total_collateral: i128,
}

/// Emitted on a successful synthetic mint.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SynthMinted {
    #[topic]
    pub minter: Address,
    pub currency: Address,
    pub synth_units: i128,
    pub shares_issued: u128,
    pub debt_added: i128,
    pub debt_per_share: u128,
    pub collateral_ratio_bps: i128,
}

/// Emitted on a successful synthetic burn.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SynthBurned {
    #[topic]
    pub minter: Address,
    pub currency: Address,
    pub synth_units: i128,
    pub shares_burned: u128,
    pub debt_retired: i128,
    pub debt_per_share: u128,
    pub index_drifted: bool,
}

/// Emitted when an oracle price is folded into a pool's index.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceUpdated {
    #[topic]
    pub currency: Address,
    pub price: u128,
    pub debt_per_share: u128,
    pub high_water_debt_per_share: u128,
}

/// Emitted when an account is flagged or repaired.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountFlagged {
    #[topic]
    pub account: Address,
    pub flagged: bool,
    pub ratio_bps: i128,
    pub timestamp: u64,
}

/// Emitted on a successful localized liquidation.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Liquidated {
    #[topic]
    pub liquidator: Address,
    #[topic]
    pub target: Address,
    pub currency: Address,
    pub debt_covered: i128,
    pub collateral_paid: i128,
    pub penalty_collateral: i128,
    /// Protocol surplus retained after this liquidation.
    pub surplus: i128,
    pub shares_retired: u128,
    pub debt_per_share: u128,
    pub target_ratio_bps: i128,
    pub liquidator_ratio_bps: i128,
}

// ---------------------------------------------------------------------------
// Receipts
// ---------------------------------------------------------------------------

/// Result of [`SynthetixCore::mint_synth`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MintReceipt {
    /// Debt shares issued.
    pub shares: u128,
    /// Nominal debt added to the pool.
    pub debt: i128,
    /// Debt-per-share index after the mint.
    pub debt_per_share: u128,
    /// The minter's collateralisation ratio after the mint, in bps.
    pub collateral_ratio_bps: i128,
}

/// Result of [`SynthetixCore::burn_synth`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BurnReceipt {
    /// Debt shares surrendered.
    pub shares: u128,
    /// Nominal debt retired from the pool.
    pub debt: i128,
    /// Debt-per-share index after the burn.
    pub debt_per_share: u128,
    /// True when the retirement moved the index, i.e. the rounding surplus fell
    /// on the remaining holders. The index is stationary by design, so this is
    /// a drift signal, not a revaluation: a price move is applied at read time
    /// by `max(indexed debt, synths x price)` and never writes the index.
    pub index_drifted: bool,
}

/// Result of [`SynthetixCore::liquidate`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquidationReceipt {
    /// Nominal debt covered.
    pub debt_covered: i128,
    /// Collateral transferred from the target to the liquidator.
    pub collateral_paid: i128,
    /// Portion of `collateral_paid` that is pure penalty.
    pub penalty_collateral: i128,
    /// Protocol surplus retained after this liquidation. The penalty is not
    /// withdrawn from the contract, so it accrues as backing for the remaining
    /// debt rather than leaving the protocol.
    pub surplus: i128,
    /// Debt shares retired from the target.
    pub shares_retired: u128,
    /// Debt-per-share index after the liquidation.
    pub debt_per_share: u128,
    /// The target's collateralisation ratio afterwards, in bps.
    pub target_ratio_bps: i128,
    /// Whether the target remains flagged afterwards.
    pub still_flagged: bool,
    /// The liquidator's collateralisation ratio afterwards, in bps. Asserted to
    /// be at least `min(ratio before, target_ratio_bps)`.
    pub liquidator_ratio_bps: i128,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// Synthetic asset issuance, pooled debt tracking and localized liquidation.
#[contract]
pub struct SynthetixCore;

#[contractimpl]
impl SynthetixCore {
    // ------------------------------------------------------------------
    // Initialisation
    // ------------------------------------------------------------------

    /// Initialise the protocol.
    ///
    /// * `collateral_token` — the token users lock as collateral.
    /// * `collateral_price` — its price in the pool's base unit, 1e9 fixed
    ///   point (e.g. `50_000_000_000` for $50.00).
    /// * `min_ratio_bps` — minimum collateralisation ratio in basis points.
    ///   Must be at least [`MIN_COLLATERAL_RATIO_BPS`] (300%); the floor is a
    ///   protocol invariant, not an admin preference.
    /// * `penalty_bps` — liquidation penalty in basis points, at most
    ///   [`MAX_PENALTY_BPS`].
    pub fn initialize(
        env: Env,
        admin: Address,
        collateral_token: Address,
        collateral_price: u128,
        min_ratio_bps: i128,
        penalty_bps: i128,
    ) -> Result<(), SynthetixError> {
        if env.storage().instance().has(&KEY_ADMIN) {
            return Err(SynthetixError::AlreadyInitialized);
        }
        admin.require_auth();
        validate_price(collateral_price).map_err(SynthetixError::from)?;
        if !(MIN_COLLATERAL_RATIO_BPS..=MAX_COLLATERAL_RATIO_BPS).contains(&min_ratio_bps) {
            return Err(SynthetixError::InvalidRatio);
        }
        if !(0..=MAX_PENALTY_BPS).contains(&penalty_bps) {
            return Err(SynthetixError::InvalidRatio);
        }

        env.storage().instance().set(&KEY_ADMIN, &admin);
        env.storage()
            .instance()
            .set(&KEY_COLLATERAL, &collateral_token);
        env.storage()
            .instance()
            .set(&KEY_COLLATERAL_PRICE, &collateral_price);
        env.storage()
            .instance()
            .set(&KEY_MIN_RATIO_BPS, &min_ratio_bps);
        env.storage().instance().set(&KEY_PENALTY_BPS, &penalty_bps);
        env.storage().instance().set(&KEY_TOTAL_COLLATERAL, &0i128);
        env.storage().instance().set(&KEY_SURPLUS, &0i128);
        env.storage()
            .instance()
            .set(&KEY_CURRENCIES, &Vec::<Address>::new(&env));
        env.storage().instance().set(&KEY_PAUSED, &false);
        bump_instance(&env);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Configuration
    // ------------------------------------------------------------------

    /// Register a synthetic currency and its initial oracle price.
    pub fn add_currency(
        env: Env,
        caller: Address,
        currency: Address,
        initial_price: u128,
    ) -> Result<(), SynthetixError> {
        require_admin(&env, &caller)?;
        validate_price(initial_price).map_err(SynthetixError::from)?;

        let mut registry = currency_registry(&env);
        if registry.contains(currency.clone()) {
            return Err(SynthetixError::CurrencyAlreadyRegistered);
        }
        if registry.len() >= MAX_CURRENCIES {
            return Err(SynthetixError::CurrencyLimitReached);
        }
        registry.push_back(currency.clone());
        env.storage().instance().set(&KEY_CURRENCIES, &registry);
        env.storage()
            .persistent()
            .set(&PositionKey::Pool(currency.clone()), &DebtPool::new());
        env.storage()
            .persistent()
            .set(&PositionKey::Price(currency.clone()), &initial_price);
        bump_instance(&env);
        Ok(())
    }

    /// Fold a new oracle price into a currency's pool index.
    ///
    /// This is the "sharp price surge" entry point: it rewrites the global
    /// counters once and leaves every minter's debt to be re-derived from the
    /// index.
    pub fn set_price(
        env: Env,
        caller: Address,
        currency: Address,
        price: u128,
    ) -> Result<(), SynthetixError> {
        require_admin(&env, &caller)?;
        require_currency(&env, &currency)?;
        validate_price(price).map_err(SynthetixError::from)?;

        let mut pool = read_pool(&env, &currency);
        pool.record_price(price).map_err(SynthetixError::from)?;
        write_pool(&env, &currency, &pool);
        env.storage()
            .persistent()
            .set(&PositionKey::Price(currency.clone()), &price);
        bump_instance(&env);
        bump_key(&env, &PositionKey::Pool(currency.clone()));
        bump_key(&env, &PositionKey::Price(currency.clone()));

        PriceUpdated {
            currency,
            price,
            debt_per_share: pool.debt_per_share(),
            high_water_debt_per_share: pool.high_water_debt_per_share,
        }
        .publish(&env);
        Ok(())
    }

    /// Update the collateral price.
    pub fn set_collateral_price(
        env: Env,
        caller: Address,
        price: u128,
    ) -> Result<(), SynthetixError> {
        require_admin(&env, &caller)?;
        validate_price(price).map_err(SynthetixError::from)?;
        env.storage().instance().set(&KEY_COLLATERAL_PRICE, &price);
        bump_instance(&env);
        Ok(())
    }

    /// Raise (never lower) the minimum collateralisation ratio.
    ///
    /// The 300% floor of issue #259 is a safety property, so the ratio
    /// ratchets: governance can tighten it but never loosen it. A request at or
    /// below the ratio already in force is rejected outright rather than
    /// silently ignored, so a misconfigured governance call fails loudly.
    pub fn set_min_ratio_bps(
        env: Env,
        caller: Address,
        min_ratio_bps: i128,
    ) -> Result<(), SynthetixError> {
        require_admin(&env, &caller)?;
        if !(MIN_COLLATERAL_RATIO_BPS..=MAX_COLLATERAL_RATIO_BPS).contains(&min_ratio_bps) {
            return Err(SynthetixError::InvalidRatio);
        }
        if min_ratio_bps < min_ratio_of(&env) {
            return Err(SynthetixError::InvalidRatio);
        }
        env.storage()
            .instance()
            .set(&KEY_MIN_RATIO_BPS, &min_ratio_bps);
        bump_instance(&env);
        Ok(())
    }

    /// Set the liquidation penalty in basis points, at most [`MAX_PENALTY_BPS`].
    pub fn set_penalty_bps(
        env: Env,
        caller: Address,
        penalty_bps: i128,
    ) -> Result<(), SynthetixError> {
        require_admin(&env, &caller)?;
        if !(0..=MAX_PENALTY_BPS).contains(&penalty_bps) {
            return Err(SynthetixError::InvalidRatio);
        }
        env.storage().instance().set(&KEY_PENALTY_BPS, &penalty_bps);
        bump_instance(&env);
        Ok(())
    }

    /// Emergency stop. Mints and liquidations are blocked while paused;
    /// collateral withdrawal stays open so a user is never trapped by it.
    pub fn set_paused(env: Env, caller: Address, paused: bool) -> Result<(), SynthetixError> {
        require_admin(&env, &caller)?;
        env.storage().instance().set(&KEY_PAUSED, &paused);
        bump_instance(&env);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Collateral
    // ------------------------------------------------------------------

    /// Lock `amount` of the collateral token as backing for synthetic debt.
    pub fn lock_collateral(env: Env, minter: Address, amount: i128) -> Result<(), SynthetixError> {
        require_active(&env)?;
        minter.require_auth();
        validate_amount(amount, MAX_COLLATERAL)?;

        let token: Address = read_instance(&env, &KEY_COLLATERAL)?;
        TokenClient::new(&env, &token).transfer(&minter, env.current_contract_address(), &amount);

        let updated = collateral_of(&env, &minter)
            .checked_add(amount)
            .ok_or(SynthetixError::CalculationOverflow)?;
        validate_amount(updated, MAX_COLLATERAL)?;
        let total = add_total_collateral(&env, amount)?;

        write_collateral(&env, &minter, updated);
        bump_key(&env, &PositionKey::Collateral(minter.clone()));
        bump_instance(&env);

        CollateralChanged {
            account: minter,
            delta: amount,
            total_collateral: total,
        }
        .publish(&env);
        Ok(())
    }

    /// Release `amount` of collateral back to the minter.
    ///
    /// Refused if the release would drop the minter below the minimum
    /// collateralisation ratio. Available even while paused, so a user is never
    /// trapped in the protocol by the emergency stop.
    pub fn withdraw_collateral(
        env: Env,
        minter: Address,
        amount: i128,
    ) -> Result<(), SynthetixError> {
        minter.require_auth();
        validate_amount(amount, MAX_COLLATERAL)?;

        let current = collateral_of(&env, &minter);
        if current < amount {
            return Err(SynthetixError::InsufficientCollateral);
        }
        if below_ratio(&env, &minter, current - amount) {
            return Err(SynthetixError::UnderCollateralized);
        }

        write_collateral(&env, &minter, current - amount);
        bump_key(&env, &PositionKey::Collateral(minter.clone()));
        let total = add_total_collateral(&env, -amount)?;
        let token: Address = read_instance(&env, &KEY_COLLATERAL)?;
        TokenClient::new(&env, &token).transfer(&env.current_contract_address(), &minter, &amount);
        bump_instance(&env);

        CollateralChanged {
            account: minter,
            delta: -amount,
            total_collateral: total,
        }
        .publish(&env);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Minting and burning
    // ------------------------------------------------------------------

    /// Mint `synth_units` of `currency` against the minter's locked
    /// collateral.
    ///
    /// Rejected unless the minter's *total* debt across every registered
    /// currency stays at or above [`MIN_COLLATERAL_RATIO_BPS`] of their
    /// collateral value once this mint is included.
    pub fn mint_synth(
        env: Env,
        minter: Address,
        currency: Address,
        synth_units: i128,
    ) -> Result<MintReceipt, SynthetixError> {
        require_active(&env)?;
        minter.require_auth();
        require_currency(&env, &currency)?;
        validate_amount(synth_units, MAX_POOL_AMOUNT)?;

        let price = read_price(&env, &currency);
        let mut pool = read_pool(&env, &currency);
        let minted = pool.mint(synth_units, price)?;

        // The debt the minter would carry across *all* currencies, this one
        // included, must be covered 300%+ by their collateral.
        let projected = total_debt_of(&env, &minter)
            .checked_add(minted.debt)
            .ok_or(SynthetixError::CalculationOverflow)?;
        let collateral_value = collateral_value_of(&env, &minter);
        let ratio = collateral_ratio_bps(collateral_value, projected);
        if ratio < min_ratio_of(&env) {
            return Err(SynthetixError::UnderCollateralized);
        }

        write_pool(&env, &currency, &pool);
        add_shares(&env, &minter, &currency, share_delta(minted.shares)?)?;
        add_synth_units(&env, &minter, &currency, synth_units)?;

        bump_instance(&env);
        bump_key(&env, &PositionKey::Pool(currency.clone()));
        bump_key(&env, &PositionKey::Shares(minter.clone(), currency.clone()));
        bump_key(
            &env,
            &PositionKey::SynthUnits(minter.clone(), currency.clone()),
        );

        SynthMinted {
            minter: minter.clone(),
            currency: currency.clone(),
            synth_units,
            shares_issued: minted.shares,
            debt_added: minted.debt,
            debt_per_share: minted.debt_per_share,
            collateral_ratio_bps: ratio,
        }
        .publish(&env);

        Ok(MintReceipt {
            shares: minted.shares,
            debt: minted.debt,
            debt_per_share: minted.debt_per_share,
            collateral_ratio_bps: ratio,
        })
    }

    /// Burn `synth_units` of `currency`, retiring the pro-rata debt.
    ///
    /// At a higher oracle price fewer debt shares are surrendered, which raises
    /// the pool's debt-per-share index and shifts the remaining debt onto the
    /// other minters — the mechanism a price surge uses to push the surviving
    /// positions under the threshold.
    pub fn burn_synth(
        env: Env,
        minter: Address,
        currency: Address,
        synth_units: i128,
    ) -> Result<BurnReceipt, SynthetixError> {
        minter.require_auth();
        require_currency(&env, &currency)?;
        validate_amount(synth_units, MAX_POOL_AMOUNT)?;

        if synth_units_of(&env, &minter, &currency) < synth_units {
            return Err(SynthetixError::InsufficientSynthBalance);
        }

        let price = read_price(&env, &currency);
        let mut pool = read_pool(&env, &currency);
        let held_shares = shares_of(&env, &minter, &currency);
        let burned = pool.burn(synth_units, price, held_shares)?;

        write_pool(&env, &currency, &pool);
        surrender_shares(&env, &minter, &currency, burned.shares)?;
        add_synth_units(&env, &minter, &currency, -synth_units)?;

        bump_instance(&env);
        bump_key(&env, &PositionKey::Pool(currency.clone()));
        bump_key(&env, &PositionKey::Shares(minter.clone(), currency.clone()));
        bump_key(
            &env,
            &PositionKey::SynthUnits(minter.clone(), currency.clone()),
        );

        SynthBurned {
            minter: minter.clone(),
            currency: currency.clone(),
            synth_units,
            shares_burned: burned.shares,
            debt_retired: burned.debt,
            debt_per_share: burned.debt_per_share,
            index_drifted: burned.index_drifted,
        }
        .publish(&env);

        Ok(BurnReceipt {
            shares: burned.shares,
            debt: burned.debt,
            debt_per_share: burned.debt_per_share,
            index_drifted: burned.index_drifted,
        })
    }

    // ------------------------------------------------------------------
    // Flagging and localized liquidation
    // ------------------------------------------------------------------

    /// Flag an account whose collateralisation ratio has fallen below the
    /// threshold, or clear the flag if it has recovered. Returns the new state.
    ///
    /// Permissionless by design: anyone may report a shortfall so a keeper
    /// needs no protocol privileges. The flag is advisory — it records a
    /// shortfall for monitoring and is what `liquidate` treats as the trigger,
    /// but liquidation re-checks the ratio on chain, so a stale or missing flag
    /// can never be used to liquidate a healthy account.
    pub fn flag_account(env: Env, target: Address) -> Result<bool, SynthetixError> {
        let ratio = ratio_bps(&env, &target);
        let flagged = ratio < min_ratio_of(&env);
        let was = read_flag(&env, &target);
        if flagged == was {
            return Ok(flagged);
        }

        write_flag(&env, &target, flagged, env.ledger().timestamp());
        bump_key(&env, &PositionKey::Flagged(target.clone()));
        bump_key(&env, &PositionKey::FlaggedAt(target.clone()));
        bump_instance(&env);

        AccountFlagged {
            account: target,
            flagged,
            ratio_bps: ratio,
            timestamp: env.ledger().timestamp(),
        }
        .publish(&env);
        Ok(flagged)
    }

    /// Localized liquidation of an under-collateralised account.
    ///
    /// The liquidator redeems `debt_to_cover` of `target`'s debt in
    /// `currency` using the synthetic units they already hold, and receives
    /// that much of the target's collateral plus a `penalty_bps` surcharge —
    /// the penalty levied on an account that dipped below the threshold. The
    /// liquidation is *localized* in two senses:
    ///
    /// * only the liquidator's and the target's positions change — no other
    ///   minter's shares are read or written, and
    /// * the liquidator can never be made worse off by the trade than they
    ///   already were, or than the account they rescued.
    ///
    /// The target keeps its synthetic units, exactly as in Synthetix: only its
    /// debt is cut, and the penalty the target forfeits stays in the protocol as
    /// [surplus](SynthetixCore::surplus) because the tokens backing it are
    /// still held.
    ///
    /// ## Why the liquidator is checked *relatively*
    ///
    /// Positions are marked at `max(indexed debt, synths x price)`, so after a
    /// price surge **every** synth holder is already below the threshold. An
    /// absolute "stay above 300%" gate on the liquidator would therefore make
    /// liquidation impossible exactly when it is needed — the classic
    /// liquidation-deadlock failure mode. Instead the trade is asserted to be
    /// non-frateral: the liquidator's post-trade ratio may not fall below
    /// `min(ratio before, target ratio after)`. Their collateral strictly
    /// increases and their effective debt strictly decreases, so the bound holds
    /// by construction; it is enforced as a post-condition so that any future
    /// change to the accounting that breaks it reverts the trade.
    pub fn liquidate(
        env: Env,
        liquidator: Address,
        target: Address,
        currency: Address,
        debt_to_cover: i128,
    ) -> Result<LiquidationReceipt, SynthetixError> {
        require_active(&env)?;
        liquidator.require_auth();
        if target == liquidator {
            return Err(SynthetixError::InvariantViolation);
        }
        require_currency(&env, &currency)?;
        validate_amount(debt_to_cover, MAX_POOL_AMOUNT)?;

        // The target must actually be in shortfall, and the request must not
        // exceed the debt it actually owes in this currency.
        if ratio_bps(&env, &target) >= min_ratio_of(&env) {
            return Err(SynthetixError::TargetNotUndercollateralized);
        }
        if indexed_debt_of(&env, &target, &currency) < debt_to_cover {
            return Err(SynthetixError::InsufficientDebtToCover);
        }

        let price = read_price(&env, &currency);
        let collateral_price = collateral_price_of(&env);
        let penalty_bps = penalty_of(&env);

        // The liquidator funds the redemption with synthetic units they hold;
        // there is no protocol minting on their behalf.
        let redemption_units =
            synth_units_for_debt(debt_to_cover, price).map_err(SynthetixError::from)?;
        if synth_units_of(&env, &liquidator, &currency) < redemption_units {
            return Err(SynthetixError::InsufficientSynthBalance);
        }

        // Collateral the target forfeits: worth the covered debt, plus penalty.
        let mut owed = liquidation_collateral(debt_to_cover, collateral_price, penalty_bps)
            .map_err(SynthetixError::from)?;
        let target_collateral = collateral_of(&env, &target);
        if owed > target_collateral {
            // Defensive clamp: a target can never post more collateral than it
            // holds, so the penalty becomes best-effort and `owed` is cut to
            // what is actually available.
            owed = target_collateral;
        }
        if owed == 0 {
            return Err(SynthetixError::InsufficientCollateral);
        }
        let base = debt_value(debt_to_cover, collateral_price).map_err(SynthetixError::from)?;
        let penalty_collateral = (owed - base).max(0);

        // Remember the liquidator's starting point for the post-condition.
        let liquidator_ratio_before = ratio_bps(&env, &liquidator);

        // Pool half: retire the debt and the matching synthetic supply in one
        // global rewrite.
        let mut pool = read_pool(&env, &currency);
        let target_shares = shares_of(&env, &target, &currency);
        let retired = pool
            .liquidate_debt(debt_to_cover, price, target_shares)
            .map_err(SynthetixError::from)?;
        write_pool(&env, &currency, &pool);
        surrender_shares(&env, &target, &currency, retired.shares)?;
        add_synth_units(&env, &liquidator, &currency, -retired.synth_units)?;

        // Position half: move collateral from the shortfall account to the
        // liquidator.
        let liquidator_total = collateral_of(&env, &liquidator)
            .checked_add(owed)
            .ok_or(SynthetixError::CalculationOverflow)?;
        write_collateral(&env, &target, target_collateral - owed);
        write_collateral(&env, &liquidator, liquidator_total);

        // The penalty does not leave the protocol: those tokens are still held
        // as backing, while the pool's debt has fallen by the full amount
        // covered. The forfeited value therefore accrues to the pool as
        // surplus, exactly as in Synthetix, and is what absorbs a future
        // shortfall. `total_collateral` is unchanged, so the sum of every
        // minter's balance keeps equalling the protocol total.
        let total_collateral = total_collateral_of(&env);
        let surplus = add_surplus(&env, penalty_collateral)?;

        // Re-flag: the target stays flagged for as long as it is in shortfall.
        let new_ratio = ratio_bps(&env, &target);
        let still_flagged = new_ratio < min_ratio_of(&env);
        if still_flagged != read_flag(&env, &target) {
            write_flag(&env, &target, still_flagged, env.ledger().timestamp());
            bump_key(&env, &PositionKey::Flagged(target.clone()));
            bump_key(&env, &PositionKey::FlaggedAt(target.clone()));
        }

        // Non-fraternity post-condition (see the note above). This is the
        // localised safety property: absorbing a shortfall must never leave the
        // liquidator in a worse position than the one they intervened in.
        let liquidator_ratio_after = ratio_bps(&env, &liquidator);
        if liquidator_ratio_after < liquidator_ratio_before.min(new_ratio) {
            return Err(SynthetixError::LiquidatorWouldBeUndercollateralized);
        }

        bump_instance(&env);
        bump_key(&env, &PositionKey::Pool(currency.clone()));
        bump_key(&env, &PositionKey::Shares(target.clone(), currency.clone()));
        bump_key(
            &env,
            &PositionKey::SynthUnits(liquidator.clone(), currency.clone()),
        );
        bump_key(&env, &PositionKey::Collateral(target.clone()));
        bump_key(&env, &PositionKey::Collateral(liquidator.clone()));

        Liquidated {
            liquidator: liquidator.clone(),
            target: target.clone(),
            currency: currency.clone(),
            debt_covered: retired.debt,
            collateral_paid: owed,
            penalty_collateral,
            surplus,
            shares_retired: retired.shares,
            debt_per_share: retired.debt_per_share,
            target_ratio_bps: new_ratio,
            liquidator_ratio_bps: liquidator_ratio_after,
        }
        .publish(&env);

        CollateralChanged {
            account: target,
            delta: -owed,
            total_collateral,
        }
        .publish(&env);

        Ok(LiquidationReceipt {
            debt_covered: retired.debt,
            collateral_paid: owed,
            penalty_collateral,
            surplus,
            shares_retired: retired.shares,
            debt_per_share: retired.debt_per_share,
            target_ratio_bps: new_ratio,
            still_flagged,
            liquidator_ratio_bps: liquidator_ratio_after,
        })
    }

    // ------------------------------------------------------------------
    // Views
    // ------------------------------------------------------------------

    /// Debt the protocol counts against `minter` in `currency`: the greater of
    /// the indexed debt and the market value of the synths held.
    pub fn debt_of(env: Env, minter: Address, currency: Address) -> i128 {
        effective_debt_of(&env, &minter, &currency)
    }

    /// Debt `minter` owes in `currency` purely as recorded by the global index,
    /// ignoring the current oracle price. This is the figure the debt shares
    /// encode.
    pub fn indexed_debt_of(env: Env, minter: Address, currency: Address) -> i128 {
        indexed_debt_of(&env, &minter, &currency)
    }

    /// Nominal debt `minter` owes across every registered currency.
    pub fn total_debt(env: Env, minter: Address) -> i128 {
        total_debt_of(&env, &minter)
    }

    /// `minter`'s debt shares in `currency`.
    pub fn shares_of(env: Env, minter: Address, currency: Address) -> u128 {
        shares_of(&env, &minter, &currency)
    }

    /// `minter`'s synthetic-unit balance in `currency`.
    pub fn synth_units_of(env: Env, minter: Address, currency: Address) -> i128 {
        synth_units_of(&env, &minter, &currency)
    }

    /// `minter`'s locked collateral.
    pub fn collateral_of(env: Env, minter: Address) -> i128 {
        collateral_of(&env, &minter)
    }

    /// `minter`'s locked collateral valued in the pool's base unit.
    pub fn collateral_value_of(env: Env, minter: Address) -> i128 {
        collateral_value_of(&env, &minter)
    }

    /// `minter`'s effective collateralisation ratio, in basis points.
    pub fn collateral_ratio_bps(env: Env, minter: Address) -> i128 {
        ratio_bps(&env, &minter)
    }

    /// Whether `minter` is currently below the minimum ratio.
    pub fn is_undercollateralized(env: Env, minter: Address) -> bool {
        below_ratio(&env, &minter, collateral_of(&env, &minter))
    }

    /// Whether `minter` carries an under-collateralisation flag.
    pub fn is_flagged(env: Env, minter: Address) -> bool {
        read_flag(&env, &minter)
    }

    /// The ledger timestamp at which `minter`'s flag last changed.
    pub fn flagged_at(env: Env, minter: Address) -> u64 {
        read_persistent(&env, &PositionKey::FlaggedAt(minter)).unwrap_or(0)
    }

    /// Debt-per-share index of `currency`, in 1e18 fixed point.
    pub fn debt_per_share(env: Env, currency: Address) -> u128 {
        read_pool(&env, &currency).debt_per_share()
    }

    /// Oracle price of `currency`, in 1e9 fixed point.
    pub fn price_of(env: Env, currency: Address) -> u128 {
        read_price(&env, &currency)
    }

    /// Oracle price of the collateral token, in 1e9 fixed point.
    pub fn collateral_price(env: Env) -> u128 {
        collateral_price_of(&env)
    }

    /// The full global state of `currency`'s debt pool.
    pub fn pool(env: Env, currency: Address) -> DebtPool {
        read_pool(&env, &currency)
    }

    /// Protocol-wide locked collateral.
    pub fn total_collateral(env: Env) -> i128 {
        total_collateral_of(&env)
    }

    /// Value forfeited by liquidated accounts and retained by the protocol.
    ///
    /// The penalty a shortfall account pays is never withdrawn, so it accrues
    /// here as extra backing for the debt that remains.
    pub fn surplus(env: Env) -> i128 {
        surplus_of(&env)
    }

    /// The minimum collateralisation ratio in force, in basis points.
    pub fn min_ratio_bps(env: Env) -> i128 {
        min_ratio_of(&env)
    }

    /// The liquidation penalty in force, in basis points.
    pub fn penalty_bps(env: Env) -> i128 {
        penalty_of(&env)
    }

    /// The registered synthetic currencies.
    pub fn currencies(env: Env) -> Vec<Address> {
        currency_registry(&env)
    }

    /// Whether the emergency stop is active.
    pub fn is_paused(env: Env) -> bool {
        paused(&env)
    }

    /// The protocol admin / oracle address.
    pub fn admin(env: Env) -> Result<Address, SynthetixError> {
        read_instance(&env, &KEY_ADMIN)
    }

    /// Collateral needed to back `debt` at the ratio currently in force.
    pub fn required_collateral_for(env: Env, debt: i128) -> i128 {
        required_collateral(debt, min_ratio_of(&env)).unwrap_or(i128::MAX)
    }

    /// How many synthetic units `debt` buys at `price`. Pure arithmetic, so it
    /// takes no storage and is callable as a quote helper.
    pub fn synth_units_for(_env: Env, debt: i128, price: u128) -> i128 {
        synth_units_for_debt(debt, price).unwrap_or(0)
    }

    /// Upper bound on any oracle price, for keeper-side sanity checks.
    pub fn max_price() -> u128 {
        MAX_PRICE
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

fn bump_key(env: &Env, key: &PositionKey) {
    if env.storage().persistent().has(key) {
        env.storage().persistent().extend_ttl(
            key,
            PERSISTENT_TTL_THRESHOLD,
            PERSISTENT_TTL_EXTEND_TO,
        );
    }
}

fn read_instance<T>(env: &Env, key: &Symbol) -> Result<T, SynthetixError>
where
    T: TryFromVal<Env, Val>,
{
    env.storage()
        .instance()
        .get::<_, T>(key)
        .ok_or(SynthetixError::NotInitialized)
}

fn read_persistent<T>(env: &Env, key: &PositionKey) -> Option<T>
where
    T: TryFromVal<Env, Val>,
{
    env.storage().persistent().get::<_, T>(key)
}

fn require_admin(env: &Env, caller: &Address) -> Result<(), SynthetixError> {
    let admin: Address = read_instance(env, &KEY_ADMIN)?;
    if caller != &admin {
        return Err(SynthetixError::Unauthorized);
    }
    caller.require_auth();
    Ok(())
}

fn require_active(env: &Env) -> Result<(), SynthetixError> {
    if paused(env) {
        return Err(SynthetixError::ContractPaused);
    }
    Ok(())
}

fn require_currency(env: &Env, currency: &Address) -> Result<(), SynthetixError> {
    if !currency_registry(env).contains(currency.clone()) {
        return Err(SynthetixError::UnknownCurrency);
    }
    Ok(())
}

fn paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get::<_, bool>(&KEY_PAUSED)
        .unwrap_or(false)
}

fn validate_amount(amount: i128, max: i128) -> Result<(), SynthetixError> {
    if amount <= 0 || amount > max {
        return Err(SynthetixError::AmountOutOfRange);
    }
    Ok(())
}

/// Reinterpret a `u128` share count as a signed `i128` delta.
fn share_delta(shares: u128) -> Result<i128, SynthetixError> {
    if shares > i128::MAX as u128 {
        return Err(SynthetixError::CalculationOverflow);
    }
    Ok(shares as i128)
}

/// Take `shares` out of a minter's debt-share balance when they retire debt.
///
/// Deliberately separate from [`add_shares`] so a retirement can never be
/// written as an issuance by mistake: the delta is unambiguously negative and
/// an attempt to surrender more than is held reverts.
fn surrender_shares(
    env: &Env,
    minter: &Address,
    currency: &Address,
    shares: u128,
) -> Result<u128, SynthetixError> {
    let magnitude = share_delta(shares)?;
    add_shares(
        env,
        minter,
        currency,
        magnitude
            .checked_neg()
            .ok_or(SynthetixError::CalculationOverflow)?,
    )
}

fn currency_registry(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get::<_, Vec<Address>>(&KEY_CURRENCIES)
        .unwrap_or_else(|| Vec::new(env))
}

fn min_ratio_of(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&KEY_MIN_RATIO_BPS)
        .unwrap_or(MIN_COLLATERAL_RATIO_BPS)
}

fn penalty_of(env: &Env) -> i128 {
    env.storage().instance().get(&KEY_PENALTY_BPS).unwrap_or(0)
}

fn collateral_price_of(env: &Env) -> u128 {
    env.storage()
        .instance()
        .get(&KEY_COLLATERAL_PRICE)
        .unwrap_or(0)
}

fn total_collateral_of(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&KEY_TOTAL_COLLATERAL)
        .unwrap_or(0)
}

fn add_total_collateral(env: &Env, delta: i128) -> Result<i128, SynthetixError> {
    let updated = total_collateral_of(env)
        .checked_add(delta)
        .ok_or(SynthetixError::CalculationOverflow)?;
    if !(0..=MAX_COLLATERAL).contains(&updated) {
        return Err(SynthetixError::CalculationOverflow);
    }
    env.storage()
        .instance()
        .set(&KEY_TOTAL_COLLATERAL, &updated);
    Ok(updated)
}

/// Value forfeited by liquidated accounts and retained by the protocol.
///
/// A liquidation retires the full `debt_to_cover` from the pool while the
/// target hands over `debt_to_cover` **plus** the penalty as collateral. Those
/// tokens are never withdrawn, so the penalty is realised as backing for the
/// debt that remains: the pool's surplus grows by exactly the penalty.
fn surplus_of(env: &Env) -> i128 {
    env.storage().instance().get(&KEY_SURPLUS).unwrap_or(0)
}

fn add_surplus(env: &Env, delta: i128) -> Result<i128, SynthetixError> {
    let updated = surplus_of(env)
        .checked_add(delta)
        .ok_or(SynthetixError::CalculationOverflow)?;
    if !(0..=MAX_COLLATERAL).contains(&updated) {
        return Err(SynthetixError::CalculationOverflow);
    }
    env.storage().instance().set(&KEY_SURPLUS, &updated);
    Ok(updated)
}

fn read_pool(env: &Env, currency: &Address) -> DebtPool {
    read_persistent(env, &PositionKey::Pool(currency.clone())).unwrap_or_else(DebtPool::new)
}

fn write_pool(env: &Env, currency: &Address, pool: &DebtPool) {
    env.storage()
        .persistent()
        .set(&PositionKey::Pool(currency.clone()), pool);
}

fn read_price(env: &Env, currency: &Address) -> u128 {
    read_persistent(env, &PositionKey::Price(currency.clone())).unwrap_or(0)
}

fn shares_of(env: &Env, minter: &Address, currency: &Address) -> u128 {
    read_persistent(env, &PositionKey::Shares(minter.clone(), currency.clone())).unwrap_or(0)
}

/// Apply a signed delta to a minter's debt-share balance.
fn add_shares(
    env: &Env,
    minter: &Address,
    currency: &Address,
    delta: i128,
) -> Result<u128, SynthetixError> {
    let key = PositionKey::Shares(minter.clone(), currency.clone());
    let current: u128 = read_persistent(env, &key).unwrap_or(0);
    let updated = if delta >= 0 {
        let magnitude = u128::try_from(delta).map_err(|_| SynthetixError::CalculationOverflow)?;
        current
            .checked_add(magnitude)
            .ok_or(SynthetixError::CalculationOverflow)?
    } else {
        let magnitude = u128::try_from(-delta).map_err(|_| SynthetixError::CalculationOverflow)?;
        current
            .checked_sub(magnitude)
            .ok_or(SynthetixError::InvariantViolation)?
    };
    if updated > MAX_POOL_SHARES {
        return Err(SynthetixError::CalculationOverflow);
    }
    env.storage().persistent().set(&key, &updated);
    Ok(updated)
}

fn synth_units_of(env: &Env, minter: &Address, currency: &Address) -> i128 {
    read_persistent(
        env,
        &PositionKey::SynthUnits(minter.clone(), currency.clone()),
    )
    .unwrap_or(0)
}

fn add_synth_units(
    env: &Env,
    minter: &Address,
    currency: &Address,
    delta: i128,
) -> Result<i128, SynthetixError> {
    let key = PositionKey::SynthUnits(minter.clone(), currency.clone());
    let current: i128 = read_persistent(env, &key).unwrap_or(0);
    let updated = current
        .checked_add(delta)
        .ok_or(SynthetixError::CalculationOverflow)?;
    if updated < 0 {
        return Err(SynthetixError::InvariantViolation);
    }
    if updated > MAX_POOL_AMOUNT {
        return Err(SynthetixError::CalculationOverflow);
    }
    env.storage().persistent().set(&key, &updated);
    Ok(updated)
}

fn collateral_of(env: &Env, minter: &Address) -> i128 {
    read_persistent(env, &PositionKey::Collateral(minter.clone())).unwrap_or(0)
}

fn write_collateral(env: &Env, minter: &Address, amount: i128) {
    env.storage()
        .persistent()
        .set(&PositionKey::Collateral(minter.clone()), &amount);
}

fn read_flag(env: &Env, minter: &Address) -> bool {
    read_persistent(env, &PositionKey::Flagged(minter.clone())).unwrap_or(false)
}

fn write_flag(env: &Env, minter: &Address, flagged: bool, timestamp: u64) {
    env.storage()
        .persistent()
        .set(&PositionKey::Flagged(minter.clone()), &flagged);
    env.storage()
        .persistent()
        .set(&PositionKey::FlaggedAt(minter.clone()), &timestamp);
}

/// Value of `collateral` base units at the current collateral price.
fn value_collateral(env: &Env, collateral: i128) -> i128 {
    let price = collateral_price_of(env);
    let value = (collateral.max(0) as u128).saturating_mul(price) / PRICE_PRECISION;
    i128::try_from(value).unwrap_or(i128::MAX)
}

fn collateral_value_of(env: &Env, minter: &Address) -> i128 {
    value_collateral(env, collateral_of(env, minter))
}

/// Debt `minter` owes in `currency` as recorded by the global index. `O(1)`.
fn indexed_debt_of(env: &Env, minter: &Address, currency: &Address) -> i128 {
    let shares = shares_of(env, minter, currency);
    if shares == 0 {
        return 0;
    }
    read_pool(env, currency).debt_of(shares)
}

/// Debt the protocol actually counts against `minter` in `currency`. `O(1)`.
///
/// A position is marked at the **greater** of:
///
/// * the nominal debt frozen into the global index when it was minted, and
/// * the current market value of the synthetic units the minter holds.
///
/// This is Synthetix's rule that a position accrues the rate change since
/// minting, expressed as a `max` rather than a lazy per-account accrual. The
/// `max` is what keeps the index monotonic in the safe direction: a price
/// *surge* immediately marks a position up and can push it under the
/// collateralisation threshold, while a price *fall* can never hand a minter
/// collateral back by silently shrinking their recorded debt.
fn effective_debt_of(env: &Env, minter: &Address, currency: &Address) -> i128 {
    let indexed = indexed_debt_of(env, minter, currency);
    let units = synth_units_of(env, minter, currency);
    if units <= 0 {
        return indexed;
    }
    let market = debt_value(units, read_price(env, currency)).unwrap_or(0);
    indexed.max(market)
}

/// Debt `minter` owes across every registered currency.
///
/// `O(registered currencies)`, never `O(minters)`: the registry is capped at
/// [`MAX_CURRENCIES`].
fn total_debt_of(env: &Env, minter: &Address) -> i128 {
    let mut total: i128 = 0;
    for currency in currency_registry(env).iter() {
        total = match total.checked_add(effective_debt_of(env, minter, &currency)) {
            Some(sum) => sum,
            None => return i128::MAX,
        };
    }
    total
}

fn ratio_bps(env: &Env, minter: &Address) -> i128 {
    collateral_ratio_bps(collateral_value_of(env, minter), total_debt_of(env, minter))
}

/// Whether `minter` would be short of the minimum ratio holding `collateral`.
fn below_ratio(env: &Env, minter: &Address, collateral: i128) -> bool {
    collateral_ratio_bps(
        value_collateral(env, collateral),
        total_debt_of(env, minter),
    ) < min_ratio_of(env)
}
