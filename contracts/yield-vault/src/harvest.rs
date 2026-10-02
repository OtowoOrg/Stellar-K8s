//! Harvest logic for the ERC-4626 auto-compounder vault.
//!
//! Responsibilities
//! ================
//! 1. **Claim rewards** — calls `pending_rewards` and `claim_rewards` on the
//!    external yield protocol.
//! 2. **Deduct protocol fee** — a basis-point fee is taken *from the gross
//!    yield only*, never from principal.  The fee is converted into vault
//!    shares minted directly to the fee recipient (avoids extra token transfers).
//! 3. **Compound net yield** — add net yield to `TotalAssets` so every existing
//!    share is worth more underlying asset.
//! 4. **Monotonicity guard** — revert if the new share price is lower than the
//!    last recorded price.

use soroban_sdk::{Address, Env, Symbol};

use crate::{DataKey, VaultError, YieldVault, SHARE_PRECISION};

// ---------------------------------------------------------------------------
// Yield-protocol interface (trait-like via direct call)
// ---------------------------------------------------------------------------

/// We call two methods on the external yield protocol:
///   - `pending_rewards(vault_address) -> i128`   — how much is claimable
///   - `claim_rewards(vault_address)  -> i128`   — claim and return amount received
///
/// Both are assumed to deal in the *same token* as the vault's underlying asset.

fn pending_rewards(env: &Env, protocol: &Address, vault: &Address) -> i128 {
    env.invoke_contract(
        protocol,
        &Symbol::new(env, "pending_rewards"),
        soroban_sdk::vec![env, vault.to_val()],
    )
}

fn claim_rewards(env: &Env, protocol: &Address, vault: &Address) -> i128 {
    env.invoke_contract(
        protocol,
        &Symbol::new(env, "claim_rewards"),
        soroban_sdk::vec![env, vault.to_val()],
    )
}

// ---------------------------------------------------------------------------
// Public entry-point called from YieldVault::harvest
// ---------------------------------------------------------------------------

/// Execute a full harvest cycle:
///
/// 1. Check pending rewards (early exit if none).
/// 2. Claim rewards from the yield protocol.
/// 3. Calculate fee on gross yield.
/// 4. Mint fee shares to the fee recipient.
/// 5. Add net yield to TotalAssets.
/// 6. Assert share price ≥ last recorded price (monotonicity guard).
/// 7. Persist new share price.
///
/// Returns `(gross_yield, fee_assets, net_yield)`.
pub fn execute_harvest(
    env: &Env,
    _caller: &Address,
    yield_protocol: &Address,
    _asset: &Address,
) -> Result<(i128, i128, i128), VaultError> {
    let vault_address = env.current_contract_address();

    // -----------------------------------------------------------------------
    // Step 1 — Check how much is pending.  Skip harvest if nothing to do.
    // -----------------------------------------------------------------------
    let pending: i128 = pending_rewards(env, yield_protocol, &vault_address);
    if pending <= 0 {
        return Ok((0, 0, 0));
    }

    // -----------------------------------------------------------------------
    // Step 2 — Claim rewards.  The yield protocol transfers `gross_yield`
    // units of the underlying asset to this contract.
    // -----------------------------------------------------------------------
    let gross_yield: i128 = claim_rewards(env, yield_protocol, &vault_address);
    if gross_yield <= 0 {
        return Ok((0, 0, 0));
    }

    // -----------------------------------------------------------------------
    // Step 3 — Compute protocol fee on gross yield only.
    // -----------------------------------------------------------------------
    let fee_bps: u32 = env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0);
    let fee_assets: i128 = compute_fee(gross_yield, fee_bps)?;
    let net_yield: i128 = gross_yield
        .checked_sub(fee_assets)
        .ok_or(VaultError::CalculationOverflow)?;

    // -----------------------------------------------------------------------
    // Step 4 — Mint fee shares to the fee recipient.
    //
    // We mint fee shares *before* adding net_yield to TotalAssets so the
    // recipient's shares are priced at the current (pre-harvest) share price.
    // This way the fee recipient pays the same share price as existing holders.
    //
    // fee_shares = fee_assets * TotalShares / TotalAssets  (standard deposit math)
    // -----------------------------------------------------------------------
    if fee_assets > 0 {
        let fee_recipient: Address = env.storage().instance()
            .get(&DataKey::FeeRecipient)
            .ok_or(VaultError::NotInitialized)?;

        let fee_shares = YieldVault::compute_shares_for_assets(env, fee_assets)?;
        if fee_shares > 0 {
            YieldVault::mint_shares(env, &fee_recipient, fee_shares)?;
            // TotalAssets must also increase by fee_assets so TotalAssets
            // continues to reflect all assets held by the contract.
            YieldVault::add_total_assets(env, fee_assets)?;
        }
    }

    // -----------------------------------------------------------------------
    // Step 5 — Compound: add net yield to TotalAssets.
    // This is what makes shares worth more — same number of shares, more assets.
    // -----------------------------------------------------------------------
    if net_yield > 0 {
        YieldVault::add_total_assets(env, net_yield)?;
    }

    // -----------------------------------------------------------------------
    // Step 6 — Monotonicity guard.
    //
    // New share price must be ≥ the last persisted price.  If it is lower the
    // harvest is reverted.  This should never happen under normal operation;
    // it catches accounting bugs or malicious protocol interactions.
    // -----------------------------------------------------------------------
    let last_price: i128 = env.storage().instance()
        .get(&DataKey::LastSharePrice)
        .unwrap_or(SHARE_PRECISION);
    let new_price = YieldVault::current_share_price(env);

    if new_price < last_price {
        return Err(VaultError::SharePriceDecreased);
    }

    // -----------------------------------------------------------------------
    // Step 7 — Persist new share price.
    // -----------------------------------------------------------------------
    YieldVault::record_share_price(env);

    env.events().publish(
        (Symbol::new(env, "harvest"),),
        (gross_yield, fee_assets, net_yield, new_price),
    );

    Ok((gross_yield, fee_assets, net_yield))
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Compute protocol fee from gross yield.
///
/// `fee = gross_yield * fee_bps / 10_000`
///
/// `fee_bps` must be ≤ 10 000 (validated at initialization / `set_fee_bps`).
pub fn compute_fee(gross_yield: i128, fee_bps: u32) -> Result<i128, VaultError> {
    if fee_bps == 0 {
        return Ok(0);
    }
    let fee = gross_yield
        .checked_mul(fee_bps as i128)
        .ok_or(VaultError::CalculationOverflow)?
        / 10_000;
    Ok(fee)
}

/// Compute net yield after fee deduction.
pub fn compute_net_yield(gross_yield: i128, fee_bps: u32) -> Result<i128, VaultError> {
    let fee = compute_fee(gross_yield, fee_bps)?;
    gross_yield.checked_sub(fee).ok_or(VaultError::CalculationOverflow)
}

/// Verify the share-price monotonicity invariant: new ≥ old.
///
/// Returns `Err(VaultError::SharePriceDecreased)` if the invariant is violated.
pub fn assert_share_price_monotonic(old_price: i128, new_price: i128) -> Result<(), VaultError> {
    if new_price < old_price {
        return Err(VaultError::SharePriceDecreased);
    }
    Ok(())
}

// (no re-exports needed — VaultError is already public in the crate root)
