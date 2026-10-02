//! # Index-Based Global Debt Pool
//!
//! The accounting core of the Synthetix-style synthetic issuance protocol.
//!
//! ## The problem this module solves
//!
//! Every synthetic (e.g. `sBTC`) is backed by a *shared* pool of locked
//! collateral. When the oracle price of a synthetic moves, the debt burden of
//! **every** minter of that synthetic has to move with it. A naive
//! implementation walks a list of minter balances and re-values each one — an
//! `O(n)` pass whose cost grows without bound and which blows the Soroban
//! resource budget long before the pool is large.
//!
//! ## The index
//!
//! Instead of tracking per-user balances and re-valuing them, the pool tracks
//! **debt shares** (Synthetix calls them *issuance*). A minter holds a share
//! count; the *global* pool holds the total share count and the total debt.
//! A minter's debt is then a pure function of the two global counters:
//!
//! ```text
//! debt_per_share = total_debt / total_shares          (O(1))
//! debt_of(s)     = total_debt * s / total_shares      (O(1))
//! ```
//!
//! A price move never requires touching a single minter record. The index
//! simply re-prices itself for everyone at read time. `DebtPool::mutations`
//! makes that property observable: it counts how many times a *global* pool
//! record was rewritten, and it stays constant per user action no matter how
//! many minters are active (see `tests/stress_debt_pool.rs`).
//!
//! ## How a price surge shifts the debt burden
//!
//! Nominal debt is denominated in the pool's base unit and is *frozen at mint
//! price*, so a price surge alone does not inflate `total_debt`. The burden
//! therefore lands where it does in Synthetix — on whoever is left holding the
//! asset — but through the **read-time mark** rather than through the index.
//! The contract counts a position at
//!
//! ```text
//! marked_debt = max(indexed_debt, synth_units_held * current_price)
//! ```
//!
//! so a surge re-prices every position at once, with no write at all, and the
//! minter who *burns* has that synth removed from their mark while every other
//! holder keeps the full re-priced exposure. `indexed_debt` only ever rises, so
//! the `max` is monotone in the safe direction: a price fall can never shrink a
//! recorded debt and hand collateral back to a minter.
//!
//! The index itself (`debt_per_share`) is deliberately *not* the revaluation
//! mechanism. Because every mint issues shares pro rata against the live index
//! and every retirement surrenders them pro rata, the two global counters stay
//! in step and the index only drifts by the `ceil` rounding surplus that cannot
//! be given back — see [`DebtPool::shares_to_burn`] and the
//! `high_water_debt_per_share` high-water mark that makes the drift observable
//! instead of silent.
//!
//! ## Relationship to the Ethereum reference implementation
//!
//! Synthetix V2 re-values each account's `debtBalance` *lazily*, on the
//! account's next touch, and leaves the global `synthTotalSupply` untouched.
//! This module instead re-derives debt from the global index on every read.
//! The consequences are:
//!
//! * Debt is always current, so there is no staleness window to exploit.
//! * The marginal cost of servicing an extra minter is zero.
//! * Retiring debt always benefits the remaining holders, so rounding never
//!   confiscates value from a position that did nothing wrong.
//!
//! ## Overflow discipline
//!
//! Every amount in a pool is bounded by [`MAX_POOL_AMOUNT`] and every price by
//! [`MAX_PRICE`]. With [`PRICE_PRECISION`] = `1e9` and [`PRECISION`] = `1e18`
//! that makes the widest intermediate product `1e18 * 1e18 = 1e36`, comfortably
//! inside `u128::MAX ~= 3.4e38`. All arithmetic is therefore performed in
//! `u128` and every narrowing back to the contract's `i128` amount type is
//! checked, so a pool can never be driven into a silent wrap-around.

use soroban_sdk::contracttype;

/// Fixed-point scale (1e18) for ratio and index arithmetic.
pub const PRECISION: u128 = 1_000_000_000_000_000_000;

/// Fixed-point scale (1e9) for oracle prices: `price = human_price * 1e9`.
///
/// Nine decimals is the resolution Stellar oracles conventionally publish and
/// it keeps `synth_units * price` inside `u128` (see module docs).
pub const PRICE_PRECISION: u128 = 1_000_000_000;

/// Basis-point denominator.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Basis-point denominator as `u128`, for internal arithmetic.
const BPS_DENOMINATOR_U128: u128 = 10_000;

/// Default minimum collateralisation ratio: **300%** (issue #259).
///
/// Deliberately far above 100%: a minter's position is marked against the
/// *global* debt index, so the protocol must carry a wide margin to absorb a
/// price surge before the shortfall cascade reaches any given account.
pub const MIN_COLLATERAL_RATIO_BPS: i128 = 30_000;

/// Upper bound on the configurable collateralisation ratio (1000%).
pub const MAX_COLLATERAL_RATIO_BPS: i128 = 1_000_000;

/// Upper bound on `total_debt`, `total_shares` and `synthetic_supply` of a
/// single pool. Keeps every product inside `u128` (see module docs).
pub const MAX_POOL_AMOUNT: i128 = 1_000_000_000_000_000_000;

/// Upper bound on `DebtPool::total_shares`.
pub const MAX_POOL_SHARES: u128 = 1_000_000_000_000_000_000;

/// Upper bound on `total_collateral` protocol-wide.
pub const MAX_COLLATERAL: i128 = 1_000_000_000_000_000_000_000;

/// Upper bound on an oracle price, i.e. `1e9` base units per synth unit.
pub const MAX_PRICE: u128 = 1_000_000_000_000_000_000;

/// Upper bound on the liquidation penalty, in basis points (50%).
pub const MAX_PENALTY_BPS: i128 = 5_000;

/// Number of times a *global* pool record has been rewritten.
///
/// This is deliberately not a minter counter: it exists so tests can assert
/// that servicing a minter costs a constant number of global mutations
/// regardless of how many minters are active.
pub type MutationCount = u64;

/// Failures raised by the pure index arithmetic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathError {
    /// A divisor was zero, i.e. the pool has no outstanding debt or shares.
    DivisionByZero,
    /// An intermediate or final value left the representable range.
    Overflow,
    /// An amount or price fell outside its documented bounds.
    OutOfRange,
    /// A ratio argument was not positive.
    InvalidRatio,
}

impl From<MathError> for crate::SynthetixError {
    fn from(err: MathError) -> Self {
        match err {
            MathError::DivisionByZero => crate::SynthetixError::PoolEmpty,
            MathError::Overflow => crate::SynthetixError::CalculationOverflow,
            MathError::OutOfRange => crate::SynthetixError::AmountOutOfRange,
            MathError::InvalidRatio => crate::SynthetixError::InvalidRatio,
        }
    }
}

/// Global, per-currency debt pool state.
///
/// Every field is a *global* counter. There is deliberately no per-minter data
/// here: an account's debt is `debt_of(shares)` evaluated against these
/// counters at read time.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebtPool {
    /// Total debt shares outstanding — the denominator of the index.
    pub total_shares: u128,
    /// Sum of every minter's debt, in pool base units. Non-negative.
    pub total_debt: i128,
    /// Total synthetic units outstanding across all minters.
    pub synthetic_supply: i128,
    /// Cumulative sum of observed oracle prices. O(1) volume-weighted average.
    pub price_index: u128,
    /// Number of oracle observations folded into `price_index`.
    pub price_updates: u32,
    /// High-water mark of [`DebtPool::debt_per_share`].
    ///
    /// Mirrors Synthetix's `highWaterMark`: it records the worst debt-per-share
    /// the pool has ever reached, so liquidations can be priced against the
    /// peak rather than the instantaneous value.
    pub high_water_debt_per_share: u128,
    /// Count of global pool rewrites; see [`MutationCount`].
    pub mutations: MutationCount,
}

/// Outcome of a successful [`DebtPool::mint`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MintResult {
    /// Debt shares issued to the minter.
    pub shares: u128,
    /// Nominal debt added to the pool.
    pub debt: i128,
    /// Debt-per-share index *after* the mint.
    pub debt_per_share: u128,
}

/// Outcome of a successful [`DebtPool::burn`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BurnResult {
    /// Debt shares surrendered.
    pub shares: u128,
    /// Nominal debt retired from the pool.
    pub debt: i128,
    /// Synthetic units retired.
    pub synth_units: i128,
    /// Debt-per-share index *after* the burn.
    pub debt_per_share: u128,
    /// True when the retirement moved the index at all.
    ///
    /// Minting and retiring are both pro rata against the live index, so the
    /// index is stationary by design and only ever moves by the rounding surplus
    /// [`DebtPool::shares_to_burn`] cannot give back. This flag makes that drift
    /// observable to callers and monitoring.
    pub index_drifted: bool,
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Validate a price against [`PRICE_PRECISION`] and [`MAX_PRICE`].
pub fn validate_price(price: u128) -> Result<(), MathError> {
    if price == 0 || price > MAX_PRICE {
        return Err(MathError::OutOfRange);
    }
    Ok(())
}

/// Validate an amount against `max` and reject negatives.
fn validate_amount(amount: i128, max: i128) -> Result<(), MathError> {
    if amount < 0 || amount > max {
        return Err(MathError::OutOfRange);
    }
    Ok(())
}

/// Narrow a `u128` back to the contract's `i128` amount type.
fn to_amount(value: u128) -> Result<i128, MathError> {
    i128::try_from(value).map_err(|_| MathError::Overflow)
}

/// Multiply then divide in `u128`, rounding down. `d` must be non-zero.
fn mul_div_floor(a: u128, b: u128, d: u128) -> Result<u128, MathError> {
    if d == 0 {
        return Err(MathError::DivisionByZero);
    }
    let product = a.checked_mul(b).ok_or(MathError::Overflow)?;
    Ok(product / d)
}

/// Multiply then divide in `u128`, rounding up. `d` must be non-zero.
fn mul_div_ceil(a: u128, b: u128, d: u128) -> Result<u128, MathError> {
    if d == 0 {
        return Err(MathError::DivisionByZero);
    }
    let product = a.checked_mul(b).ok_or(MathError::Overflow)?;
    let (quotient, remainder) = (product / d, product % d);
    Ok(if remainder == 0 {
        quotient
    } else {
        quotient + 1
    })
}

// ---------------------------------------------------------------------------
// Free-standing, pure helpers
// ---------------------------------------------------------------------------

/// Nominal debt represented by `synth_units` of a synthetic priced at `price`.
///
/// The fundamental `O(1)` conversion: this single multiply is what makes a
/// price surge re-price every position in the pool at once.
pub fn debt_value(synth_units: i128, price: u128) -> Result<i128, MathError> {
    validate_amount(synth_units, MAX_POOL_AMOUNT)?;
    validate_price(price)?;
    let value = mul_div_floor(synth_units as u128, price, PRICE_PRECISION)?;
    validate_amount(to_amount(value)?, MAX_POOL_AMOUNT)?;
    to_amount(value)
}

/// Inverse of [`debt_value`]: how many synth units a nominal debt is worth.
///
/// Rounds **down** so the protocol never issues more synth than it can back.
pub fn synth_units_for_debt(debt: i128, price: u128) -> Result<i128, MathError> {
    validate_amount(debt, MAX_POOL_AMOUNT)?;
    validate_price(price)?;
    let units = mul_div_floor(debt as u128, PRICE_PRECISION, price)?;
    validate_amount(to_amount(units)?, MAX_POOL_AMOUNT)?;
    to_amount(units)
}

/// Collateral (in base units) needed to back `debt` at `min_ratio_bps`.
pub fn required_collateral(debt: i128, min_ratio_bps: i128) -> Result<i128, MathError> {
    validate_amount(debt, MAX_POOL_AMOUNT)?;
    if min_ratio_bps <= 0 {
        return Err(MathError::InvalidRatio);
    }
    let required = mul_div_ceil(debt as u128, min_ratio_bps as u128, BPS_DENOMINATOR_U128)?;
    validate_amount(to_amount(required)?, MAX_COLLATERAL)?;
    to_amount(required)
}

/// Effective collateralisation ratio of a position, in basis points.
///
/// `i128::MAX` is returned for a debt-free but collateralised position so that
/// comparisons against [`MIN_COLLATERAL_RATIO_BPS`] stay correct.
pub fn collateral_ratio_bps(collateral_value: i128, debt: i128) -> i128 {
    if collateral_value < 0 || debt < 0 {
        return 0;
    }
    if debt == 0 {
        // A debt-free account is never under-collateralised — not even one that
        // has released all of its collateral, which must never read as a
        // shortfall or it could never be withdrawn.
        return i128::MAX;
    }
    let numerator = (collateral_value as u128).saturating_mul(BPS_DENOMINATOR_U128);
    let ratio = numerator / debt as u128;
    i128::try_from(ratio).unwrap_or(i128::MAX)
}

/// Collateral handed to a liquidator to retire `debt` of a target's position.
///
/// The liquidator receives the collateral worth `debt` **plus** `penalty_bps`,
/// which is the penalty levied on an account that dips below the threshold.
/// The target is charged the full amount; the protocol books no fee, so a
/// liquidation can never make the pool insolvent.
pub fn liquidation_collateral(
    debt: i128,
    collateral_price: u128,
    penalty_bps: i128,
) -> Result<i128, MathError> {
    validate_amount(debt, MAX_POOL_AMOUNT)?;
    validate_price(collateral_price)?;
    if !(0..=MAX_PENALTY_BPS).contains(&penalty_bps) {
        return Err(MathError::OutOfRange);
    }
    // Value the debt in collateral first, then scale. Splitting it this way
    // keeps the widest intermediate at ~1e21 * 1.5e4, far inside u128.
    let collateral_value = mul_div_floor(debt as u128, collateral_price, PRICE_PRECISION)?;
    let penalised = mul_div_floor(
        collateral_value,
        BPS_DENOMINATOR_U128 + penalty_bps as u128,
        BPS_DENOMINATOR_U128,
    )?;
    validate_amount(to_amount(penalised)?, MAX_COLLATERAL)?;
    to_amount(penalised)
}

// ---------------------------------------------------------------------------
// DebtPool
// ---------------------------------------------------------------------------

impl DebtPool {
    /// An empty pool: no debt, no shares, no index history.
    pub fn new() -> Self {
        DebtPool {
            total_shares: 0,
            total_debt: 0,
            synthetic_supply: 0,
            price_index: 0,
            price_updates: 0,
            high_water_debt_per_share: 0,
            mutations: 0,
        }
    }

    /// True when no synth has ever been issued in this pool.
    pub fn is_empty(&self) -> bool {
        self.total_shares == 0
    }

    /// The index: nominal debt owed per whole debt share, in 1e18 fixed point.
    ///
    /// `O(1)`. This single value re-prices every position in the pool.
    pub fn debt_per_share(&self) -> u128 {
        if self.total_shares == 0 || self.total_debt <= 0 {
            return 0;
        }
        let total = self.total_debt as u128;
        let shares = self.total_shares;
        // Split into quotient and remainder so the product stays inside u128.
        total / shares * PRECISION + total % shares * PRECISION / shares
    }

    /// Nominal debt attributable to a holder of `shares`. `O(1)`.
    ///
    /// A holder can never hold more shares than the pool has outstanding, so
    /// the input is clamped before the (quotient, remainder) split.
    pub fn debt_of(&self, shares: u128) -> i128 {
        if self.total_shares == 0 || self.total_debt <= 0 || shares == 0 {
            return 0;
        }
        let total = self.total_debt as u128;
        let total_shares = self.total_shares;
        let shares = shares.min(total_shares);
        let debt = total / total_shares * shares + total % total_shares * shares / total_shares;
        i128::try_from(debt).unwrap_or(i128::MAX)
    }

    /// Volume-weighted average of every oracle price folded in so far. `O(1)`.
    pub fn average_price(&self) -> u128 {
        if self.price_updates == 0 {
            return 0;
        }
        self.price_index / self.price_updates as u128
    }

    /// Fold an oracle observation into the pool's `O(1)` accumulators.
    ///
    /// Rewrites the global record exactly once — the reason servicing a price
    /// update costs the same whether one minter or ten thousand are active.
    pub fn record_price(&mut self, price: u128) -> Result<(), MathError> {
        validate_price(price)?;
        let index = self
            .price_index
            .checked_add(price)
            .ok_or(MathError::Overflow)?;
        self.price_index = index;
        self.price_updates = self.price_updates.saturating_add(1);
        self.touch_high_water_mark();
        self.mutations += 1;
        Ok(())
    }

    /// Refresh the high-water mark from the current index.
    fn touch_high_water_mark(&mut self) {
        let current = self.debt_per_share();
        if current > self.high_water_debt_per_share {
            self.high_water_debt_per_share = current;
        }
    }

    /// Debt shares that represent `debt` of nominal debt.
    ///
    /// An empty pool bootstraps at one share per base unit so the very first
    /// minter does not mint an unbounded share count. Thereafter shares are
    /// issued pro rata against the live index.
    pub fn shares_for_debt(&self, debt: i128) -> Result<u128, MathError> {
        validate_amount(debt, MAX_POOL_AMOUNT)?;
        if debt == 0 {
            return Ok(0);
        }
        if self.total_debt <= 0 || self.total_shares == 0 {
            return Ok(debt as u128);
        }
        let shares = mul_div_ceil(debt as u128, self.total_shares, self.total_debt as u128)?;
        if shares > MAX_POOL_SHARES {
            return Err(MathError::Overflow);
        }
        Ok(shares)
    }

    /// Debt shares a holder must surrender to retire `debt`.
    ///
    /// Rounds **up** so the target's position is never left carrying residual
    /// dust debt, and the debt actually retired is then re-derived from the
    /// shares surrendered (floored). The two roundings cancel to within one
    /// base unit, which lands on the remaining holders as a marginal shift in
    /// `debt_per_share` — the only way the index ever moves, and why
    /// `high_water_debt_per_share` is tracked.
    pub fn shares_to_burn(&self, debt: i128) -> Result<u128, MathError> {
        validate_amount(debt, MAX_POOL_AMOUNT)?;
        if debt == 0 {
            return Ok(0);
        }
        if self.total_debt <= 0 || self.total_shares == 0 {
            return Err(MathError::DivisionByZero);
        }
        let shares = mul_div_ceil(debt as u128, self.total_shares, self.total_debt as u128)?;
        Ok(shares.min(self.total_shares))
    }

    /// Issue `synth_units` of a synthetic priced at `price`.
    ///
    /// Writes the global record exactly once. The caller is responsible for the
    /// 300% collateralisation check; this function only maintains the index.
    pub fn mint(&mut self, synth_units: i128, price: u128) -> Result<MintResult, MathError> {
        let debt = debt_value(synth_units, price)?;
        let shares = self.shares_for_debt(debt)?;
        let total_debt = self
            .total_debt
            .checked_add(debt)
            .ok_or(MathError::Overflow)?;
        validate_amount(total_debt, MAX_POOL_AMOUNT)?;
        let total_shares = self
            .total_shares
            .checked_add(shares)
            .ok_or(MathError::Overflow)?;
        if total_shares > MAX_POOL_SHARES {
            return Err(MathError::Overflow);
        }
        validate_amount(
            self.synthetic_supply
                .checked_add(synth_units)
                .ok_or(MathError::Overflow)?,
            MAX_POOL_AMOUNT,
        )?;
        validate_price(price)?;

        self.total_debt = total_debt;
        self.total_shares = total_shares;
        self.synthetic_supply += synth_units;
        self.price_index = self
            .price_index
            .checked_add(price)
            .ok_or(MathError::Overflow)?;
        self.price_updates = self.price_updates.saturating_add(1);
        self.touch_high_water_mark();
        self.mutations += 1;

        Ok(MintResult {
            shares,
            debt,
            debt_per_share: self.debt_per_share(),
        })
    }

    /// Retire `synth_units` of a synthetic priced at `price`.
    ///
    /// `holder_shares` caps the surrender at the caller's actual position. The
    /// debt actually retired is re-derived from the shares surrendered so that
    /// `total_debt` and `total_shares` can never drift apart.
    pub fn burn(
        &mut self,
        synth_units: i128,
        price: u128,
        holder_shares: u128,
    ) -> Result<BurnResult, MathError> {
        validate_amount(synth_units, MAX_POOL_AMOUNT)?;
        if synth_units > self.synthetic_supply {
            return Err(MathError::OutOfRange);
        }
        let target = debt_value(synth_units, price)?;
        let index_before = self.debt_per_share();
        let (shares, debt) = self.retire_inner(target, holder_shares)?;
        self.synthetic_supply -= synth_units;
        self.touch_high_water_mark();
        self.mutations += 1;
        let index_after = self.debt_per_share();
        Ok(BurnResult {
            shares,
            debt,
            synth_units,
            debt_per_share: index_after,
            index_drifted: index_after != index_before,
        })
    }

    /// Retire `target` debt from a holder with `holder_shares` shares.
    ///
    /// Used directly when retiring debt *without* retiring synthetic units,
    /// which is what the localized liquidation engine does to a shortfall
    /// account: Synthetix leaves the shortfall account's synth balance intact
    /// and only cuts its debt.
    pub fn retire(&mut self, target: i128, holder_shares: u128) -> Result<(u128, i128), MathError> {
        let (shares, debt) = self.retire_inner(target, holder_shares)?;
        self.touch_high_water_mark();
        self.mutations += 1;
        Ok((shares, debt))
    }

    /// Retire `debt_to_cover` of *nominal debt* on behalf of a liquidator and
    /// shrink the synthetic supply by the units that debt represents.
    ///
    /// This is the pool half of a localized liquidation: a single global
    /// rewrite that touches no per-minter record. Note that, as in Synthetix,
    /// the shortfall account keeps its synthetic units — only its debt is cut.
    pub fn liquidate_debt(
        &mut self,
        debt_to_cover: i128,
        price: u128,
        target_shares: u128,
    ) -> Result<BurnResult, MathError> {
        validate_amount(debt_to_cover, MAX_POOL_AMOUNT)?;
        let units = synth_units_for_debt(debt_to_cover, price)?;
        let units = units.min(self.synthetic_supply);
        let index_before = self.debt_per_share();
        let (shares, debt) = self.retire_inner(debt_to_cover, target_shares)?;
        self.synthetic_supply -= units;
        self.touch_high_water_mark();
        self.mutations += 1;
        let index_after = self.debt_per_share();
        Ok(BurnResult {
            shares,
            debt,
            synth_units: units,
            debt_per_share: index_after,
            index_drifted: index_after != index_before,
        })
    }

    /// Shared, mutation-free core of every retirement path.
    fn retire_inner(
        &mut self,
        target: i128,
        holder_shares: u128,
    ) -> Result<(u128, i128), MathError> {
        validate_amount(target, MAX_POOL_AMOUNT)?;
        if target == 0 {
            return Ok((0, 0));
        }
        if self.total_debt <= 0 || self.total_shares == 0 {
            return Err(MathError::DivisionByZero);
        }
        let shares = self.shares_to_burn(target)?.min(holder_shares);
        if shares == 0 {
            return Err(MathError::DivisionByZero);
        }
        // Re-derive the retired debt from the shares actually surrendered so
        // the two global counters stay exactly consistent with each other.
        let debt = self.debt_of(shares).min(self.total_debt);
        self.total_debt -= debt;
        self.total_shares -= shares;
        Ok((shares, debt))
    }
}

impl Default for DebtPool {
    fn default() -> Self {
        Self::new()
    }
}
