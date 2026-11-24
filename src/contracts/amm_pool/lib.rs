//! Dynamic Liquidity Pool Automated Market Maker (AMM) Primitive
//!
//! Implements a Soroban-compatible constant-product AMM with:
//!
//! - **LP token minting / burning** — liquidity providers receive fungible
//!   shares proportional to their contribution.
//! - **Constant-product swaps** — `x * y = k` invariant is maintained and
//!   verified after every trade.
//! - **Protocol fee** — a configurable fee (in basis points) is charged on
//!   each swap; the fee stays in the pool, increasing the value of LP shares.
//! - **Slippage protection** — every swap enforces a `min_output` parameter;
//!   the call reverts if the actual output falls below the minimum.
//!
//! # Invariant
//!
//! ```text
//!  reserve_x * reserve_y = k   (must be non-decreasing after any operation)
//! ```
//!
//! See [`math`] for the fixed-point arithmetic used by swap and LP computations.

use serde::{Deserialize, Serialize};

use crate::contracts::amm_pool::math::{
    bps_to_scale, get_amount_in, get_amount_out, initial_lp_shares, lp_to_amounts,
    subsequent_lp_shares, verify_k_invariant, MathError, MINIMUM_LIQUIDITY,
};

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by the AMM pool contract.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AmmError {
    /// The pool already has liquidity (second call to `initialise`).
    #[error("pool already initialised")]
    AlreadyInitialised,

    /// Operation requires the pool to be initialised first.
    #[error("pool not initialised")]
    NotInitialised,

    /// Zero-amount operations are rejected.
    #[error("amount must be greater than zero")]
    ZeroAmount,

    /// Slippage guard: actual output is below `min_output`.
    #[error("slippage exceeded: got {got}, needed {needed}")]
    SlippageExceeded { got: u128, needed: u128 },

    /// LP burn would exceed the redeemable supply.
    #[error("insufficient LP balance: have {have}, need {need}")]
    InsufficientLp { have: u128, need: u128 },

    /// Underlying arithmetic error (overflow, zero-reserve, etc.).
    #[error("math error: {0}")]
    Math(#[from] MathError),
}

// ── Pool state ────────────────────────────────────────────────────────────────

/// Snapshot of AMM pool state, useful for assertions and audit logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolState {
    pub reserve_x: u128,
    pub reserve_y: u128,
    pub total_lp: u128,
    pub fee_bps: u128,
}

impl PoolState {
    /// Compute the current k value (may overflow for very large reserves; use with care).
    pub fn k(&self) -> Option<u128> {
        self.reserve_x.checked_mul(self.reserve_y)
    }
}

// ── Events (logs) ─────────────────────────────────────────────────────────────

/// Events emitted by pool operations (returned in the result for inspection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolEvent {
    Initialised {
        reserve_x: u128,
        reserve_y: u128,
        lp_minted: u128,
    },
    LiquidityAdded {
        amount_x: u128,
        amount_y: u128,
        lp_minted: u128,
    },
    LiquidityRemoved {
        lp_burned: u128,
        amount_x: u128,
        amount_y: u128,
    },
    Swap {
        token_in: TokenSide,
        amount_in: u128,
        amount_out: u128,
        fee_charged: u128,
    },
}

/// Which side of the pool is being swapped into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSide {
    X,
    Y,
}

// ── AMM Pool ──────────────────────────────────────────────────────────────────

/// A constant-product AMM liquidity pool.
#[derive(Debug, Default)]
pub struct AmmPool {
    reserve_x: u128,
    reserve_y: u128,
    total_lp: u128,
    /// Fee in basis points (e.g. 30 = 0.30%)
    fee_bps: u128,
    /// Cached scaled fee value (derived from fee_bps)
    fee_scaled: u128,
    initialised: bool,
}

impl AmmPool {
    /// Create a new, uninitialised pool with the given fee.
    ///
    /// `fee_bps` is validated against [`math::MAX_FEE_BPS`].
    pub fn new(fee_bps: u128) -> Result<Self, AmmError> {
        let fee_scaled = bps_to_scale(fee_bps)?;
        Ok(Self {
            fee_bps,
            fee_scaled,
            ..Default::default()
        })
    }

    /// Return the current pool state.
    pub fn state(&self) -> PoolState {
        PoolState {
            reserve_x: self.reserve_x,
            reserve_y: self.reserve_y,
            total_lp: self.total_lp,
            fee_bps: self.fee_bps,
        }
    }

    // ── Liquidity operations ───────────────────────────────────────────────

    /// Deposit the initial liquidity, minting the first LP shares.
    ///
    /// The `MINIMUM_LIQUIDITY` is permanently locked to prevent LP price
    /// manipulation.
    ///
    /// Returns the number of LP tokens minted to the caller.
    pub fn initialise(
        &mut self,
        amount_x: u128,
        amount_y: u128,
    ) -> Result<(u128, PoolEvent), AmmError> {
        if self.initialised {
            return Err(AmmError::AlreadyInitialised);
        }
        if amount_x == 0 || amount_y == 0 {
            return Err(AmmError::ZeroAmount);
        }

        let lp_minted = initial_lp_shares(amount_x, amount_y)?;

        self.reserve_x = amount_x;
        self.reserve_y = amount_y;
        // total_lp includes MINIMUM_LIQUIDITY (locked) + minted to caller
        self.total_lp = lp_minted + MINIMUM_LIQUIDITY;
        self.initialised = true;

        Ok((
            lp_minted,
            PoolEvent::Initialised {
                reserve_x: amount_x,
                reserve_y: amount_y,
                lp_minted,
            },
        ))
    }

    /// Add liquidity to an existing pool.
    ///
    /// Returns the number of LP tokens minted to the caller.
    pub fn add_liquidity(
        &mut self,
        amount_x: u128,
        amount_y: u128,
    ) -> Result<(u128, PoolEvent), AmmError> {
        self.require_initialised()?;
        if amount_x == 0 || amount_y == 0 {
            return Err(AmmError::ZeroAmount);
        }

        let lp_minted = subsequent_lp_shares(
            amount_x,
            amount_y,
            self.reserve_x,
            self.reserve_y,
            self.total_lp,
        )?;

        self.reserve_x += amount_x;
        self.reserve_y += amount_y;
        self.total_lp += lp_minted;

        Ok((
            lp_minted,
            PoolEvent::LiquidityAdded {
                amount_x,
                amount_y,
                lp_minted,
            },
        ))
    }

    /// Remove liquidity by burning `lp_burned` LP shares.
    ///
    /// Returns `(amount_x, amount_y)` sent back to the caller.
    pub fn remove_liquidity(
        &mut self,
        lp_burned: u128,
    ) -> Result<((u128, u128), PoolEvent), AmmError> {
        self.require_initialised()?;
        if lp_burned == 0 {
            return Err(AmmError::ZeroAmount);
        }
        // Cannot burn more than total (minus locked minimum)
        let redeemable = self.total_lp.saturating_sub(MINIMUM_LIQUIDITY);
        if lp_burned > redeemable {
            return Err(AmmError::InsufficientLp {
                have: redeemable,
                need: lp_burned,
            });
        }

        let (amount_x, amount_y) =
            lp_to_amounts(lp_burned, self.reserve_x, self.reserve_y, self.total_lp)?;

        self.reserve_x -= amount_x;
        self.reserve_y -= amount_y;
        self.total_lp -= lp_burned;

        Ok((
            (amount_x, amount_y),
            PoolEvent::LiquidityRemoved {
                lp_burned,
                amount_x,
                amount_y,
            },
        ))
    }

    // ── Swap operations ────────────────────────────────────────────────────

    /// Swap `dx` of token X for token Y.
    ///
    /// `min_dy` is the slippage protection floor.  Returns `dy` received.
    pub fn swap_x_for_y(
        &mut self,
        dx: u128,
        min_dy: u128,
    ) -> Result<(u128, PoolEvent), AmmError> {
        self.require_initialised()?;
        if dx == 0 {
            return Err(AmmError::ZeroAmount);
        }

        let fee_charged = crate::contracts::amm_pool::math::fee_amount(dx, self.fee_scaled);
        let dy = get_amount_out(dx, self.reserve_x, self.reserve_y, self.fee_scaled, min_dy)?;

        let old_rx = self.reserve_x;
        let old_ry = self.reserve_y;

        self.reserve_x += dx;
        self.reserve_y -= dy;

        verify_k_invariant(old_rx, old_ry, self.reserve_x, self.reserve_y)?;

        Ok((
            dy,
            PoolEvent::Swap {
                token_in: TokenSide::X,
                amount_in: dx,
                amount_out: dy,
                fee_charged,
            },
        ))
    }

    /// Swap `dy` of token Y for token X.
    ///
    /// `min_dx` is the slippage protection floor.  Returns `dx` received.
    pub fn swap_y_for_x(
        &mut self,
        dy: u128,
        min_dx: u128,
    ) -> Result<(u128, PoolEvent), AmmError> {
        self.require_initialised()?;
        if dy == 0 {
            return Err(AmmError::ZeroAmount);
        }

        let fee_charged = crate::contracts::amm_pool::math::fee_amount(dy, self.fee_scaled);
        let dx = get_amount_out(dy, self.reserve_y, self.reserve_x, self.fee_scaled, min_dx)?;

        let old_rx = self.reserve_x;
        let old_ry = self.reserve_y;

        self.reserve_y += dy;
        self.reserve_x -= dx;

        verify_k_invariant(old_rx, old_ry, self.reserve_x, self.reserve_y)?;

        Ok((
            dx,
            PoolEvent::Swap {
                token_in: TokenSide::Y,
                amount_in: dy,
                amount_out: dx,
                fee_charged,
            },
        ))
    }

    /// Quote: how much token Y would I receive for `dx` of X (no state change)?
    pub fn quote_x_for_y(&self, dx: u128) -> Result<u128, AmmError> {
        self.require_initialised()?;
        Ok(get_amount_out(dx, self.reserve_x, self.reserve_y, self.fee_scaled, 0)?)
    }

    /// Quote: how much X must I provide to receive exactly `dy` of Y?
    pub fn quote_amount_in_for_dy(&self, dy: u128) -> Result<u128, AmmError> {
        self.require_initialised()?;
        Ok(get_amount_in(dy, self.reserve_x, self.reserve_y, self.fee_scaled)?)
    }

    // ── Internal ──────────────────────────────────────────────────────────

    fn require_initialised(&self) -> Result<(), AmmError> {
        if self.initialised {
            Ok(())
        } else {
            Err(AmmError::NotInitialised)
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_with_liquidity(rx: u128, ry: u128) -> AmmPool {
        let mut pool = AmmPool::new(30).unwrap(); // 0.30% fee
        pool.initialise(rx, ry).unwrap();
        pool
    }

    // ── Construction ─────────────────────────────────────────────────────

    #[test]
    fn test_new_pool_invalid_fee_fails() {
        assert!(AmmPool::new(10_001).is_err());
    }

    #[test]
    fn test_new_pool_zero_fee_ok() {
        let pool = AmmPool::new(0).unwrap();
        assert_eq!(pool.fee_bps, 0);
    }

    // ── Initialise ────────────────────────────────────────────────────────

    #[test]
    fn test_initialise_sets_reserves() {
        let pool = pool_with_liquidity(1_000_000, 2_000_000);
        let s = pool.state();
        assert_eq!(s.reserve_x, 1_000_000);
        assert_eq!(s.reserve_y, 2_000_000);
    }

    #[test]
    fn test_initialise_mints_lp_tokens() {
        let mut pool = AmmPool::new(30).unwrap();
        let (lp, _) = pool.initialise(1_000_000, 1_000_000).unwrap();
        assert!(lp > 0);
        assert_eq!(pool.total_lp, lp + MINIMUM_LIQUIDITY);
    }

    #[test]
    fn test_double_initialise_fails() {
        let mut pool = pool_with_liquidity(1_000, 1_000);
        assert!(matches!(
            pool.initialise(1_000, 1_000),
            Err(AmmError::AlreadyInitialised)
        ));
    }

    #[test]
    fn test_initialise_zero_amount_fails() {
        let mut pool = AmmPool::new(30).unwrap();
        assert!(matches!(
            pool.initialise(0, 1_000),
            Err(AmmError::ZeroAmount)
        ));
    }

    // ── add_liquidity ─────────────────────────────────────────────────────

    #[test]
    fn test_add_liquidity_increases_reserves() {
        let mut pool = pool_with_liquidity(1_000_000, 1_000_000);
        let rx_before = pool.reserve_x;
        pool.add_liquidity(100_000, 100_000).unwrap();
        assert_eq!(pool.reserve_x, rx_before + 100_000);
    }

    #[test]
    fn test_add_liquidity_mints_proportional_lp() {
        let mut pool = pool_with_liquidity(1_000_000, 1_000_000);
        let total_before = pool.total_lp;
        let (lp, _) = pool.add_liquidity(100_000, 100_000).unwrap();
        // Should be ~10% of existing supply
        let expected = total_before / 10;
        assert!(
            lp >= expected - 1 && lp <= expected + 1,
            "lp={lp} expected≈{expected}"
        );
    }

    // ── remove_liquidity ──────────────────────────────────────────────────

    #[test]
    fn test_remove_liquidity_returns_tokens() {
        let mut pool = pool_with_liquidity(1_000_000, 1_000_000);
        let redeemable = pool.total_lp - MINIMUM_LIQUIDITY;
        let ((ax, ay), _) = pool.remove_liquidity(redeemable / 2).unwrap();
        assert!(ax > 0);
        assert!(ay > 0);
    }

    #[test]
    fn test_remove_liquidity_excess_fails() {
        let mut pool = pool_with_liquidity(1_000_000, 1_000_000);
        let redeemable = pool.total_lp - MINIMUM_LIQUIDITY;
        assert!(matches!(
            pool.remove_liquidity(redeemable + 1),
            Err(AmmError::InsufficientLp { .. })
        ));
    }

    // ── swap_x_for_y ──────────────────────────────────────────────────────

    #[test]
    fn test_swap_x_for_y_returns_nonzero() {
        let mut pool = pool_with_liquidity(1_000_000, 1_000_000);
        let (dy, _) = pool.swap_x_for_y(10_000, 0).unwrap();
        assert!(dy > 0);
    }

    #[test]
    fn test_swap_x_for_y_reserves_update() {
        let mut pool = pool_with_liquidity(1_000_000, 1_000_000);
        let (dy, _) = pool.swap_x_for_y(10_000, 0).unwrap();
        assert_eq!(pool.reserve_x, 1_010_000);
        assert_eq!(pool.reserve_y, 1_000_000 - dy);
    }

    #[test]
    fn test_swap_slippage_protection() {
        let mut pool = pool_with_liquidity(1_000_000, 1_000_000);
        let fair_out = pool.quote_x_for_y(10_000).unwrap();
        // Demand more than the pool can give
        let err = pool.swap_x_for_y(10_000, fair_out + 1).unwrap_err();
        assert!(matches!(err, AmmError::Math(MathError::InsufficientOutput { .. })));
    }

    // ── k invariant ───────────────────────────────────────────────────────

    #[test]
    fn test_k_invariant_holds_after_multiple_swaps() {
        let mut pool = pool_with_liquidity(1_000_000, 2_000_000);
        let k_before = pool.state().k().unwrap();
        for _ in 0..10 {
            pool.swap_x_for_y(1_000, 0).unwrap();
        }
        let k_after = pool.state().k().unwrap();
        assert!(k_after >= k_before, "k decreased: before={k_before} after={k_after}");
    }

    #[test]
    fn test_k_invariant_holds_after_swap_y_for_x() {
        let mut pool = pool_with_liquidity(1_000_000, 1_000_000);
        let k_before = pool.state().k().unwrap();
        pool.swap_y_for_x(5_000, 0).unwrap();
        let k_after = pool.state().k().unwrap();
        assert!(k_after >= k_before);
    }

    // ── quote ─────────────────────────────────────────────────────────────

    #[test]
    fn test_quote_is_consistent_with_swap() {
        let mut pool = pool_with_liquidity(1_000_000, 1_000_000);
        let quoted = pool.quote_x_for_y(5_000).unwrap();
        let (received, _) = pool.swap_x_for_y(5_000, 0).unwrap();
        assert_eq!(quoted, received);
    }

    #[test]
    fn test_quote_amount_in_for_dy() {
        let pool = pool_with_liquidity(100_000, 100_000);
        let dx = pool.quote_amount_in_for_dy(1_000).unwrap();
        assert!(dx > 1_000, "dx={dx} must be > 1000 due to fee");
    }

    // ── add then remove liquidity roundtrip ──────────────────────────────

    #[test]
    fn test_add_then_remove_liquidity_roundtrip() {
        let mut pool = pool_with_liquidity(1_000_000, 1_000_000);
        let (lp, _) = pool.add_liquidity(200_000, 200_000).unwrap();
        let ((ax, ay), _) = pool.remove_liquidity(lp).unwrap();
        // Should get back approximately what we put in (small rounding allowed)
        assert!(ax >= 199_900 && ax <= 200_100, "ax={ax}");
        assert!(ay >= 199_900 && ay <= 200_100, "ay={ay}");
    }

    // ── uninitialised pool ────────────────────────────────────────────────

    #[test]
    fn test_swap_on_uninitialised_pool_fails() {
        let mut pool = AmmPool::new(30).unwrap();
        assert!(matches!(
            pool.swap_x_for_y(100, 0),
            Err(AmmError::NotInitialised)
        ));
    }

    #[test]
    fn test_add_liquidity_on_uninitialised_pool_fails() {
        let mut pool = AmmPool::new(30).unwrap();
        assert!(matches!(
            pool.add_liquidity(100, 100),
            Err(AmmError::NotInitialised)
        ));
    }
}
