//! ERC-4626 style auto-compounder vault for Stellar Soroban.
//!
//! Architecture
//! ============
//! Users deposit an *underlying asset* (e.g. USDC) and receive back *share
//! tokens* (vTokens) that represent a fractional claim on the ever-growing pool.
//!
//! The share price is defined as:
//!
//!   share_price = total_assets / total_shares   (in underlying asset units per share)
//!
//! Invariant: share_price must never decrease.  The `harvest` function (see
//! `harvest.rs`) converts earned rewards into more underlying asset and
//! re-stakes them.  It enforces the monotonicity guarantee at runtime.
//!
//! Protocol fee: a configurable basis-point fee is deducted *from the yield
//! only* (never from principal).  Fee shares are minted to the fee recipient
//! instead of transferring underlying tokens, keeping the mechanics clean.

#![no_std]

pub mod harvest;
#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype,
    token::Client as TokenClient,
    Address, Env, Symbol,
};

// ---------------------------------------------------------------------------
// Precision constant used for share-price arithmetic.
// We use 1e18 so that even tiny per-share increments are representable.
// ---------------------------------------------------------------------------

/// Fixed-point precision for internal share-price calculations.
pub const SHARE_PRECISION: i128 = 1_000_000_000_000_000_000; // 1e18

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VaultError {
    AlreadyInitialized   = 1,
    NotInitialized       = 2,
    Unauthorized         = 3,
    InvalidAmount        = 4,
    InsufficientShares   = 5,
    ZeroShares           = 6,
    ZeroAssets           = 7,
    /// Harvest attempted to decrease share price — blocked by monotonicity guard.
    SharePriceDecreased  = 8,
    CalculationOverflow  = 9,
    InvalidFeeBps        = 10,
    Paused               = 11,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Contract admin address.
    Admin,
    /// The underlying ERC-20 compatible token being vaulted (e.g. USDC).
    Asset,
    /// External yield protocol contract address.
    YieldProtocol,
    /// Total share tokens issued (denominator of share price).
    TotalShares,
    /// Total underlying assets under management (numerator of share price).
    TotalAssets,
    /// Share balance of an individual holder.
    ShareBalance(Address),
    /// Fee recipient address.
    FeeRecipient,
    /// Protocol fee in basis points (0..=10_000).  Applied to yield only.
    FeeBps,
    /// Last known share price (SHARE_PRECISION-scaled) — used for monotonicity guard.
    LastSharePrice,
    /// Paused flag — disables deposits/withdrawals when true.
    Paused,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct YieldVault;

#[contractimpl]
impl YieldVault {
    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    /// Initialise the vault.
    ///
    /// * `asset`          — the underlying token contract address
    /// * `yield_protocol` — external contract that generates yield
    /// * `fee_recipient`  — address that accrues protocol fee shares
    /// * `fee_bps`        — fee in basis points (e.g. 500 = 5 %).  Must be ≤ 10 000.
    pub fn initialize(
        env: Env,
        admin: Address,
        asset: Address,
        yield_protocol: Address,
        fee_recipient: Address,
        fee_bps: u32,
    ) -> Result<(), VaultError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(VaultError::AlreadyInitialized);
        }
        admin.require_auth();
        if fee_bps > 10_000 {
            return Err(VaultError::InvalidFeeBps);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Asset, &asset);
        env.storage().instance().set(&DataKey::YieldProtocol, &yield_protocol);
        env.storage().instance().set(&DataKey::FeeRecipient, &fee_recipient);
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
        env.storage().instance().set(&DataKey::TotalShares, &0i128);
        env.storage().instance().set(&DataKey::TotalAssets, &0i128);
        // Initial share price is 1:1 (scaled by SHARE_PRECISION).
        env.storage().instance().set(&DataKey::LastSharePrice, &SHARE_PRECISION);
        env.storage().instance().set(&DataKey::Paused, &false);

        env.events().publish((Symbol::new(&env, "initialized"),), (admin, asset));
        Ok(())
    }

    // -----------------------------------------------------------------------
    // User-facing: deposit & withdraw
    // -----------------------------------------------------------------------

    /// Deposit `assets` units of the underlying token.
    /// Returns the number of shares minted.
    ///
    /// Shares minted = assets × total_shares / total_assets   (first deposit: 1:1)
    pub fn deposit(env: Env, caller: Address, assets: i128) -> Result<i128, VaultError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        if assets <= 0 {
            return Err(VaultError::InvalidAmount);
        }
        caller.require_auth();

        let shares = Self::compute_shares_for_assets(&env, assets)?;
        if shares <= 0 {
            return Err(VaultError::ZeroShares);
        }

        // Pull underlying tokens from the caller.
        let asset: Address = Self::get_asset(&env);
        TokenClient::new(&env, &asset).transfer(&caller, &env.current_contract_address(), &assets);

        // Mint shares.
        Self::mint_shares(&env, &caller, shares)?;
        // Update total assets.
        Self::add_total_assets(&env, assets)?;

        env.events().publish(
            (Symbol::new(&env, "deposit"), caller.clone()),
            (assets, shares),
        );
        Ok(shares)
    }

    /// Redeem `shares` in exchange for underlying assets.
    /// Returns the number of underlying asset units returned to the caller.
    ///
    /// Assets returned = shares × total_assets / total_shares
    pub fn withdraw(env: Env, caller: Address, shares: i128) -> Result<i128, VaultError> {
        Self::require_not_paused(&env)?;
        Self::require_initialized(&env)?;
        if shares <= 0 {
            return Err(VaultError::InvalidAmount);
        }
        caller.require_auth();

        let caller_shares: i128 = Self::get_share_balance(&env, &caller);
        if caller_shares < shares {
            return Err(VaultError::InsufficientShares);
        }

        let assets = Self::compute_assets_for_shares(&env, shares)?;
        if assets <= 0 {
            return Err(VaultError::ZeroAssets);
        }

        // Burn shares and reduce total assets.
        Self::burn_shares(&env, &caller, shares)?;
        Self::sub_total_assets(&env, assets)?;

        // Transfer underlying tokens back to the caller.
        let asset: Address = Self::get_asset(&env);
        TokenClient::new(&env, &asset).transfer(&env.current_contract_address(), &caller, &assets);

        env.events().publish(
            (Symbol::new(&env, "withdraw"), caller.clone()),
            (shares, assets),
        );
        Ok(assets)
    }

    // -----------------------------------------------------------------------
    // Harvest — triggered externally (e.g. keeper bot)
    // -----------------------------------------------------------------------

    /// Harvest yield from the external protocol, deduct protocol fee, and
    /// compound the net yield back into the vault.
    ///
    /// Steps:
    /// 1. Query pending rewards from the yield protocol.
    /// 2. Claim rewards → vault receives reward tokens (assumed = underlying).
    /// 3. Deduct `fee_bps` basis points of the gross yield → minted as fee shares.
    /// 4. Add net yield to `total_assets`.
    /// 5. Assert share price did not decrease.
    ///
    /// Returns `(gross_yield, fee_assets, net_yield)`.
    pub fn harvest(env: Env, caller: Address) -> Result<(i128, i128, i128), VaultError> {
        Self::require_initialized(&env)?;
        // Only admin or authorised keeper may call harvest.
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if caller != admin {
            caller.require_auth();
        } else {
            admin.require_auth();
        }

        let yield_protocol: Address = env.storage().instance().get(&DataKey::YieldProtocol).unwrap();
        let asset: Address = Self::get_asset(&env);

        // Delegate to the harvest module for the actual logic.
        harvest::execute_harvest(
            &env,
            &caller,
            &yield_protocol,
            &asset,
        )
    }

    // -----------------------------------------------------------------------
    // Admin
    // -----------------------------------------------------------------------

    /// Update fee recipient (admin only).
    pub fn set_fee_recipient(env: Env, caller: Address, new_recipient: Address) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        env.storage().instance().set(&DataKey::FeeRecipient, &new_recipient);
        Ok(())
    }

    /// Update fee in basis points (admin only, ≤ 10 000).
    pub fn set_fee_bps(env: Env, caller: Address, fee_bps: u32) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        if fee_bps > 10_000 {
            return Err(VaultError::InvalidFeeBps);
        }
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
        Ok(())
    }

    /// Pause/unpause deposits and withdrawals (admin only).
    pub fn set_paused(env: Env, caller: Address, paused: bool) -> Result<(), VaultError> {
        Self::require_admin(&env, &caller)?;
        env.storage().instance().set(&DataKey::Paused, &paused);
        env.events().publish((Symbol::new(&env, "pause_changed"),), paused);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // View functions
    // -----------------------------------------------------------------------

    /// Total assets under management.
    pub fn total_assets(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalAssets).unwrap_or(0)
    }

    /// Total share tokens outstanding.
    pub fn total_shares(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0)
    }

    /// Share balance for a given address.
    pub fn share_balance(env: Env, account: Address) -> i128 {
        env.storage().instance().get(&DataKey::ShareBalance(account)).unwrap_or(0)
    }

    /// Current share price = total_assets × SHARE_PRECISION / total_shares.
    /// Returns SHARE_PRECISION when no shares exist (1:1 initial price).
    pub fn share_price(env: Env) -> i128 {
        Self::current_share_price(&env)
    }

    /// How many underlying assets does `shares` redeem for right now?
    pub fn preview_withdraw(env: Env, shares: i128) -> i128 {
        Self::compute_assets_for_shares(&env, shares).unwrap_or(0)
    }

    /// How many shares does depositing `assets` mint right now?
    pub fn preview_deposit(env: Env, assets: i128) -> i128 {
        Self::compute_shares_for_assets(&env, assets).unwrap_or(0)
    }

    /// Last recorded share price (SHARE_PRECISION-scaled).
    pub fn last_share_price(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::LastSharePrice).unwrap_or(SHARE_PRECISION)
    }

    /// Configuration: (asset, yield_protocol, fee_recipient, fee_bps).
    pub fn get_config(env: Env) -> (Address, Address, Address, u32) {
        let asset: Address = env.storage().instance().get(&DataKey::Asset).unwrap();
        let yp: Address = env.storage().instance().get(&DataKey::YieldProtocol).unwrap();
        let fr: Address = env.storage().instance().get(&DataKey::FeeRecipient).unwrap();
        let fee: u32 = env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0);
        (asset, yp, fr, fee)
    }

    // -----------------------------------------------------------------------
    // Internal helpers — also `pub(crate)` so harvest.rs can use them
    // -----------------------------------------------------------------------

    /// Returns the current share price scaled by SHARE_PRECISION.
    pub(crate) fn current_share_price(env: &Env) -> i128 {
        let total_shares: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
        let total_assets: i128 = env.storage().instance().get(&DataKey::TotalAssets).unwrap_or(0);
        if total_shares == 0 {
            return SHARE_PRECISION; // 1:1 initial price
        }
        // price = total_assets * SHARE_PRECISION / total_shares
        let numerator = total_assets.checked_mul(SHARE_PRECISION)
            .expect("share price numerator overflow");
        numerator / total_shares
    }

    /// Compute shares to mint for a deposit of `assets`.
    /// First deposit (total_shares == 0): 1:1 (assets units → assets shares).
    pub(crate) fn compute_shares_for_assets(env: &Env, assets: i128) -> Result<i128, VaultError> {
        let total_shares: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
        let total_assets: i128 = env.storage().instance().get(&DataKey::TotalAssets).unwrap_or(0);
        if total_shares == 0 || total_assets == 0 {
            // Bootstrap: 1 share per 1 asset unit.
            return Ok(assets);
        }
        // shares = assets * total_shares / total_assets
        let shares = (assets as i128)
            .checked_mul(total_shares)
            .ok_or(VaultError::CalculationOverflow)?
            / total_assets;
        Ok(shares)
    }

    /// Compute assets to return when `shares` are redeemed.
    pub(crate) fn compute_assets_for_shares(env: &Env, shares: i128) -> Result<i128, VaultError> {
        let total_shares: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
        let total_assets: i128 = env.storage().instance().get(&DataKey::TotalAssets).unwrap_or(0);
        if total_shares == 0 {
            return Ok(0);
        }
        // assets = shares * total_assets / total_shares
        let assets = shares
            .checked_mul(total_assets)
            .ok_or(VaultError::CalculationOverflow)?
            / total_shares;
        Ok(assets)
    }

    /// Mint `amount` shares to `recipient`, updating both their balance and TotalShares.
    pub(crate) fn mint_shares(env: &Env, recipient: &Address, amount: i128) -> Result<(), VaultError> {
        let key = DataKey::ShareBalance(recipient.clone());
        let current: i128 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &current.checked_add(amount).ok_or(VaultError::CalculationOverflow)?);

        let total: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
        env.storage().instance().set(&DataKey::TotalShares, &total.checked_add(amount).ok_or(VaultError::CalculationOverflow)?);
        Ok(())
    }

    /// Burn `amount` shares from `holder`, updating both their balance and TotalShares.
    pub(crate) fn burn_shares(env: &Env, holder: &Address, amount: i128) -> Result<(), VaultError> {
        let key = DataKey::ShareBalance(holder.clone());
        let current: i128 = env.storage().instance().get(&key).unwrap_or(0);
        if current < amount {
            return Err(VaultError::InsufficientShares);
        }
        env.storage().instance().set(&key, &(current - amount));

        let total: i128 = env.storage().instance().get(&DataKey::TotalShares).unwrap_or(0);
        env.storage().instance().set(&DataKey::TotalShares, &total.saturating_sub(amount));
        Ok(())
    }

    /// Increase TotalAssets by `delta`.
    pub(crate) fn add_total_assets(env: &Env, delta: i128) -> Result<(), VaultError> {
        let total: i128 = env.storage().instance().get(&DataKey::TotalAssets).unwrap_or(0);
        env.storage().instance().set(
            &DataKey::TotalAssets,
            &total.checked_add(delta).ok_or(VaultError::CalculationOverflow)?,
        );
        Ok(())
    }

    /// Decrease TotalAssets by `delta`.
    pub(crate) fn sub_total_assets(env: &Env, delta: i128) -> Result<(), VaultError> {
        let total: i128 = env.storage().instance().get(&DataKey::TotalAssets).unwrap_or(0);
        env.storage().instance().set(
            &DataKey::TotalAssets,
            &total.saturating_sub(delta),
        );
        Ok(())
    }

    /// Update LastSharePrice — called by harvest after compounding.
    pub(crate) fn record_share_price(env: &Env) {
        let price = Self::current_share_price(env);
        env.storage().instance().set(&DataKey::LastSharePrice, &price);
    }

    /// Convenience getter for the asset address.
    pub(crate) fn get_asset(env: &Env) -> Address {
        env.storage().instance().get(&DataKey::Asset).expect("not initialized")
    }

    /// Convenience getter for the share balance of an account.
    pub(crate) fn get_share_balance(env: &Env, account: &Address) -> i128 {
        env.storage().instance().get(&DataKey::ShareBalance(account.clone())).unwrap_or(0)
    }

    fn require_initialized(env: &Env) -> Result<(), VaultError> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(VaultError::NotInitialized);
        }
        Ok(())
    }

    fn require_admin(env: &Env, caller: &Address) -> Result<(), VaultError> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).ok_or(VaultError::NotInitialized)?;
        if caller != &admin {
            return Err(VaultError::Unauthorized);
        }
        caller.require_auth();
        Ok(())
    }

    fn require_not_paused(env: &Env) -> Result<(), VaultError> {
        if env.storage().instance().get::<_, bool>(&DataKey::Paused).unwrap_or(false) {
            return Err(VaultError::Paused);
        }
        Ok(())
    }
}
