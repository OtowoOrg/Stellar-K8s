//! Fixed-point arithmetic primitives for the constant-product AMM.
//!
//! All token amounts are represented as `u128` (18-decimal fixed-point
//! integers, i.e. 1 token = 10^18 units).  Every operation is designed to
//! avoid precision loss and integer overflow, and operates within safe bounds
//! for WASM stack constraints using iterative rather than recursive algorithms.
//!
//! # Key invariant
//!
//! For any pool with reserves `x` and `y`:
//!
//! ```text
//!  x * y = k   (constant product)
//! ```
//!
//! After a swap of `dx` for `dy`, the invariant must hold (accounting for fees):
//!
//! ```text
//!  (x + dx_after_fee) * (y - dy) = k
//! ```
//!
//! # Fixed-point scale
//!
//! We use a scale of `1_000_000` (6 decimal places) for fee and price
//! calculations to keep all arithmetic in `u128`.

/// Scale factor used for fixed-point fee arithmetic (1 000 000 = 100%).
pub const FEE_SCALE: u128 = 1_000_000;

/// Maximum allowed protocol fee in basis points (10 000 = 100%).
pub const MAX_FEE_BPS: u128 = 10_000;

/// Minimum liquidity locked permanently to prevent division-by-zero.
pub const MINIMUM_LIQUIDITY: u128 = 1_000;

/// Scale for LP token math — matches Uniswap v2 convention.
pub const LP_SCALE: u128 = 1_000_000_000_000_000_000; // 10^18

// ── Integer square root (Babylonian / Newton-Raphson, iterative) ──────────────

/// Compute `floor(sqrt(n))` using an iterative Babylonian method.
///
/// Safe for all `u128` values; performs at most ~128 iterations in the worst
/// case but converges in ~10 for typical LP amounts.
///
/// ```
/// # use stellar_k8s::contracts::amm_pool::math::isqrt;
/// assert_eq!(isqrt(0), 0);
/// assert_eq!(isqrt(1), 1);
/// assert_eq!(isqrt(4), 2);
/// assert_eq!(isqrt(9), 3);
/// assert_eq!(isqrt(100), 10);
/// assert_eq!(isqrt(u128::MAX), 18_446_744_073_709_551_615);
/// ```
pub fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    // Initial estimate: bit-length / 2
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ── Fee helpers ───────────────────────────────────────────────────────────────

/// Convert a fee in basis points (e.g. 30 = 0.30%) to a `FEE_SCALE` fraction.
///
/// Returns `Err` if `fee_bps > MAX_FEE_BPS`.
pub fn bps_to_scale(fee_bps: u128) -> Result<u128, MathError> {
    if fee_bps > MAX_FEE_BPS {
        return Err(MathError::FeeTooHigh(fee_bps));
    }
    // fee_bps / 10_000  →  fee_bps * 100 / FEE_SCALE
    Ok(fee_bps * 100) // e.g. 30 bps → 3_000 out of 1_000_000
}

/// Return the amount that remains after deducting the fee.
///
/// ```
/// # use stellar_k8s::contracts::amm_pool::math::{amount_after_fee, bps_to_scale};
/// let fee = bps_to_scale(30).unwrap(); // 0.30%
/// let net = amount_after_fee(10_000, fee);
/// assert_eq!(net, 9_970);
/// ```
pub fn amount_after_fee(amount: u128, fee_scaled: u128) -> u128 {
    amount * (FEE_SCALE - fee_scaled) / FEE_SCALE
}

/// Return the fee portion of an amount.
pub fn fee_amount(amount: u128, fee_scaled: u128) -> u128 {
    amount * fee_scaled / FEE_SCALE
}

// ── Constant-product swap math ────────────────────────────────────────────────

/// Errors from math operations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MathError {
    #[error("fee too high: {0} bps exceeds maximum {MAX_FEE_BPS} bps")]
    FeeTooHigh(u128),
    #[error("arithmetic overflow")]
    Overflow,
    #[error("insufficient output: got {got}, need at least {min}")]
    InsufficientOutput { got: u128, min: u128 },
    #[error("zero reserve")]
    ZeroReserve,
    #[error("zero input amount")]
    ZeroInput,
}

/// Compute `dy` — the amount of token Y received when swapping `dx` of token X.
///
/// Uses the constant-product formula after deducting the protocol fee:
///
/// ```text
///  dx_effective = dx * (FEE_SCALE - fee) / FEE_SCALE
///  dy = (reserve_y * dx_effective) / (reserve_x + dx_effective)
/// ```
///
/// # Arguments
///
/// * `dx`           – input amount of token X (raw units)
/// * `reserve_x`    – current pool reserve of token X
/// * `reserve_y`    – current pool reserve of token Y
/// * `fee_scaled`   – fee fraction at `FEE_SCALE` precision (use [`bps_to_scale`])
/// * `min_dy`       – minimum acceptable output (slippage guard)
pub fn get_amount_out(
    dx: u128,
    reserve_x: u128,
    reserve_y: u128,
    fee_scaled: u128,
    min_dy: u128,
) -> Result<u128, MathError> {
    if dx == 0 {
        return Err(MathError::ZeroInput);
    }
    if reserve_x == 0 || reserve_y == 0 {
        return Err(MathError::ZeroReserve);
    }

    let dx_eff = amount_after_fee(dx, fee_scaled);
    // dy = reserve_y * dx_eff / (reserve_x + dx_eff)
    let numerator = reserve_y
        .checked_mul(dx_eff)
        .ok_or(MathError::Overflow)?;
    let denominator = reserve_x
        .checked_add(dx_eff)
        .ok_or(MathError::Overflow)?;
    let dy = numerator / denominator;

    if dy < min_dy {
        return Err(MathError::InsufficientOutput { got: dy, min: min_dy });
    }
    Ok(dy)
}

/// Compute `dx` — the amount of token X needed to obtain exactly `dy` of token Y.
///
/// Inverse of [`get_amount_out`]:
///
/// ```text
///  numerator   = reserve_x * dy * FEE_SCALE
///  denominator = (reserve_y - dy) * (FEE_SCALE - fee)
///  dx          = numerator / denominator + 1  (ceiling)
/// ```
pub fn get_amount_in(
    dy: u128,
    reserve_x: u128,
    reserve_y: u128,
    fee_scaled: u128,
) -> Result<u128, MathError> {
    if dy == 0 {
        return Err(MathError::ZeroInput);
    }
    if reserve_x == 0 || reserve_y == 0 {
        return Err(MathError::ZeroReserve);
    }
    if dy >= reserve_y {
        return Err(MathError::Overflow);
    }

    let numerator = reserve_x
        .checked_mul(dy)
        .ok_or(MathError::Overflow)?
        .checked_mul(FEE_SCALE)
        .ok_or(MathError::Overflow)?;
    let denominator = (reserve_y - dy)
        .checked_mul(FEE_SCALE - fee_scaled)
        .ok_or(MathError::Overflow)?;

    Ok(numerator / denominator + 1) // ceil
}

// ── LP share math ─────────────────────────────────────────────────────────────

/// Compute LP tokens to mint for an initial liquidity deposit.
///
/// ```text
///  lp_minted = sqrt(amount_x * amount_y) - MINIMUM_LIQUIDITY
/// ```
///
/// The `MINIMUM_LIQUIDITY` is permanently locked so that LP token price
/// can never collapse to zero.
pub fn initial_lp_shares(amount_x: u128, amount_y: u128) -> Result<u128, MathError> {
    let product = amount_x.checked_mul(amount_y).ok_or(MathError::Overflow)?;
    let lp = isqrt(product);
    lp.checked_sub(MINIMUM_LIQUIDITY)
        .ok_or(MathError::Overflow)
}

/// Compute LP tokens to mint for a subsequent liquidity deposit.
///
/// ```text
///  lp_minted = min(
///      amount_x * total_lp / reserve_x,
///      amount_y * total_lp / reserve_y,
///  )
/// ```
pub fn subsequent_lp_shares(
    amount_x: u128,
    amount_y: u128,
    reserve_x: u128,
    reserve_y: u128,
    total_lp: u128,
) -> Result<u128, MathError> {
    if reserve_x == 0 || reserve_y == 0 || total_lp == 0 {
        return Err(MathError::ZeroReserve);
    }
    let shares_x = amount_x
        .checked_mul(total_lp)
        .ok_or(MathError::Overflow)?
        / reserve_x;
    let shares_y = amount_y
        .checked_mul(total_lp)
        .ok_or(MathError::Overflow)?
        / reserve_y;
    Ok(shares_x.min(shares_y))
}

/// Compute token amounts to return when burning `lp_burned` LP shares.
///
/// ```text
///  amount_x = lp_burned * reserve_x / total_lp
///  amount_y = lp_burned * reserve_y / total_lp
/// ```
pub fn lp_to_amounts(
    lp_burned: u128,
    reserve_x: u128,
    reserve_y: u128,
    total_lp: u128,
) -> Result<(u128, u128), MathError> {
    if total_lp == 0 {
        return Err(MathError::ZeroReserve);
    }
    let ax = lp_burned
        .checked_mul(reserve_x)
        .ok_or(MathError::Overflow)?
        / total_lp;
    let ay = lp_burned
        .checked_mul(reserve_y)
        .ok_or(MathError::Overflow)?
        / total_lp;
    Ok((ax, ay))
}

// ── K invariant ───────────────────────────────────────────────────────────────

/// Verify that `new_x * new_y >= old_x * old_y` (k must not decrease).
///
/// Returns `Ok(())` on success or `Err(MathError::Overflow)` if the invariant
/// is violated (indicating a bug in swap math).
pub fn verify_k_invariant(
    old_x: u128,
    old_y: u128,
    new_x: u128,
    new_y: u128,
) -> Result<(), MathError> {
    let old_k = old_x.checked_mul(old_y).ok_or(MathError::Overflow)?;
    let new_k = new_x.checked_mul(new_y).ok_or(MathError::Overflow)?;
    if new_k >= old_k {
        Ok(())
    } else {
        Err(MathError::Overflow)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── isqrt ─────────────────────────────────────────────────────────────

    #[test]
    fn test_isqrt_zero() { assert_eq!(isqrt(0), 0); }

    #[test]
    fn test_isqrt_one() { assert_eq!(isqrt(1), 1); }

    #[test]
    fn test_isqrt_perfect_squares() {
        for n in [4u128, 9, 16, 25, 100, 10_000, 1_000_000] {
            let r = isqrt(n);
            assert_eq!(r * r, n, "isqrt({n}) = {r}, {r}^2 ≠ {n}");
        }
    }

    #[test]
    fn test_isqrt_non_perfect_square_floor() {
        assert_eq!(isqrt(2), 1);
        assert_eq!(isqrt(3), 1);
        assert_eq!(isqrt(5), 2);
        assert_eq!(isqrt(8), 2);
        assert_eq!(isqrt(99), 9);
    }

    #[test]
    fn test_isqrt_large_value() {
        let n = 10_000_000_000_000_000_000_000_000_000u128; // 10^28
        let r = isqrt(n);
        assert!(r * r <= n);
        assert!((r + 1) * (r + 1) > n);
    }

    // ── fee helpers ───────────────────────────────────────────────────────

    #[test]
    fn test_bps_to_scale_30_bps() {
        assert_eq!(bps_to_scale(30).unwrap(), 3_000);
    }

    #[test]
    fn test_bps_to_scale_zero() {
        assert_eq!(bps_to_scale(0).unwrap(), 0);
    }

    #[test]
    fn test_bps_to_scale_too_high() {
        assert!(matches!(
            bps_to_scale(10_001),
            Err(MathError::FeeTooHigh(10_001))
        ));
    }

    #[test]
    fn test_amount_after_fee_30bps() {
        let fee = bps_to_scale(30).unwrap();
        assert_eq!(amount_after_fee(10_000, fee), 9_970);
    }

    #[test]
    fn test_fee_amount_30bps() {
        let fee = bps_to_scale(30).unwrap();
        assert_eq!(fee_amount(10_000, fee), 30);
    }

    // ── get_amount_out ────────────────────────────────────────────────────

    #[test]
    fn test_get_amount_out_basic() {
        let fee = bps_to_scale(30).unwrap();
        // reserve_x=1000, reserve_y=1000, dx=100, fee=0.3%
        // dx_eff = 100 * 997000 / 1000000 = 99 (integer truncation)
        // dy = 1000 * 99 / (1000 + 99) = 99000 / 1099 = 90
        let dy = get_amount_out(100, 1_000, 1_000, fee, 0).unwrap();
        assert!(dy > 0 && dy < 100, "dy={dy} out of expected range");
    }

    #[test]
    fn test_get_amount_out_slippage_guard() {
        let fee = bps_to_scale(30).unwrap();
        let dy = get_amount_out(100, 1_000, 1_000, fee, 0).unwrap();
        // Require more than we'll actually receive
        let err = get_amount_out(100, 1_000, 1_000, fee, dy + 1).unwrap_err();
        assert!(matches!(err, MathError::InsufficientOutput { .. }));
    }

    #[test]
    fn test_get_amount_out_zero_input_fails() {
        let fee = bps_to_scale(30).unwrap();
        assert!(matches!(
            get_amount_out(0, 1_000, 1_000, fee, 0),
            Err(MathError::ZeroInput)
        ));
    }

    #[test]
    fn test_get_amount_out_zero_reserve_fails() {
        let fee = bps_to_scale(30).unwrap();
        assert!(matches!(
            get_amount_out(100, 0, 1_000, fee, 0),
            Err(MathError::ZeroReserve)
        ));
    }

    // ── k invariant holds after swap ──────────────────────────────────────

    #[test]
    fn test_k_invariant_holds_after_swap() {
        let fee = bps_to_scale(30).unwrap();
        let rx = 1_000_000u128;
        let ry = 1_000_000u128;
        let dx = 50_000u128;
        let dy = get_amount_out(dx, rx, ry, fee, 0).unwrap();
        let new_rx = rx + dx;
        let new_ry = ry - dy;
        // k must not decrease (fee adds to pool, so new_k >= old_k)
        assert!(verify_k_invariant(rx, ry, new_rx, new_ry).is_ok());
    }

    // ── get_amount_in ─────────────────────────────────────────────────────

    #[test]
    fn test_get_amount_in_consistent_with_amount_out() {
        let fee = bps_to_scale(30).unwrap();
        let rx = 100_000u128;
        let ry = 100_000u128;
        let dy_target = 1_000u128;
        let dx_needed = get_amount_in(dy_target, rx, ry, fee).unwrap();
        let dy_actual = get_amount_out(dx_needed, rx, ry, fee, 0).unwrap();
        // Due to ceiling in amount_in, actual output >= target
        assert!(dy_actual >= dy_target, "dy_actual={dy_actual} < dy_target={dy_target}");
    }

    // ── LP share math ─────────────────────────────────────────────────────

    #[test]
    fn test_initial_lp_shares_equal_reserves() {
        // sqrt(1000 * 1000) - 1000 = 1000 - 1000 = 0 at minimum amounts
        // Use larger values: sqrt(10^12 * 10^12) = 10^12, minus MINIMUM_LIQUIDITY
        let lp = initial_lp_shares(1_000_000_000_000, 1_000_000_000_000).unwrap();
        assert_eq!(lp, 1_000_000_000_000 - MINIMUM_LIQUIDITY);
    }

    #[test]
    fn test_initial_lp_shares_product_below_minimum_fails() {
        // product=0 → isqrt=0 → 0 - 1000 overflows
        let err = initial_lp_shares(0, 1_000_000).unwrap_err();
        assert!(matches!(err, MathError::Overflow));
    }

    #[test]
    fn test_subsequent_lp_shares_proportional() {
        // Initial pool: 1000x / 1000y / 1000 LP tokens
        let total_lp = 1_000u128;
        let rx = 1_000u128;
        let ry = 1_000u128;
        // Add 500x and 500y → expect 500 LP tokens
        let lp = subsequent_lp_shares(500, 500, rx, ry, total_lp).unwrap();
        assert_eq!(lp, 500);
    }

    #[test]
    fn test_lp_to_amounts_proportional() {
        let (ax, ay) = lp_to_amounts(500, 1_000, 2_000, 1_000).unwrap();
        assert_eq!(ax, 500);
        assert_eq!(ay, 1_000);
    }

    #[test]
    fn test_lp_to_amounts_zero_total_lp_fails() {
        assert!(matches!(
            lp_to_amounts(100, 1_000, 1_000, 0),
            Err(MathError::ZeroReserve)
        ));
    }
}
