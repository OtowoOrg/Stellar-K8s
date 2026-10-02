//! Tick-based pricing in Q64.96 fixed point.
//!
//! # Model
//!
//! Prices are segregated into discrete bins called *ticks*. Tick `i`
//! corresponds to the price
//!
//! ```text
//! p(i) = 1.0001^i            (token1 per token0)
//! ```
//!
//! so adjacent ticks differ by exactly 0.01% (1 basis point). The pool
//! stores the square root of the price in Q64.96 fixed point:
//!
//! ```text
//! sqrtP(i) = sqrt(1.0001)^i · 2^96
//! ```
//!
//! Working with √P keeps the liquidity formulas linear
//! (`Δy = L·Δ√P`, `Δx = L·Δ(1/√P)`), so a swap inside one bin needs no
//! square roots at runtime.
//!
//! # Bounds
//!
//! `MIN_TICK = -887272` and `MAX_TICK = 887272` are the extreme ticks whose
//! √P lies in the Q64.96 range `[2^-64, 2^64)`, i.e.
//! `sqrt(1.0001)^887272 < 2^64 ≤ sqrt(1.0001)^887273`. Hence `sqrtP`
//! always fits in 160 bits (asserted by the tests on `MAX_SQRT_PRICE`).
//!
//! # `sqrt_price_at_tick` — correctness
//!
//! Write `|i| = Σ b_k·2^k` (binary, `k < 20` since `887272 < 2^20`). Then
//!
//! ```text
//! sqrt(1.0001)^-|i| = Π_{b_k = 1} c_k,     c_k = sqrt(1.0001)^-(2^k) < 1
//! ```
//!
//! Each `c_k` is stored in Q128.128 as `round(c_k · 2^128)` (verified to be
//! within 0.5 ulp of the exact value by the tests). The product is
//! accumulated as `r ← (r · c_k) >> 128`. Because every `c_k < 1`, `r` never
//! exceeds `2^128`, so `r · c_k < 2^256` and the 256-bit multiply cannot
//! overflow. For `i > 0` the result is inverted with `(2^256 - 1) / r`,
//! and finally converted from Q128.128 to Q64.96 with `>> 32`, **rounding
//! up**. Rounding up guarantees `sqrt_price_at_tick(tick_at_sqrt_price(p)) ≤ p`
//! is never violated by truncation.
//!
//! Each step truncates by < 1 ulp of Q128.128; with at most 20 steps and
//! one inversion the relative error is bounded by ~2^-32, measured at
//! < 2.1·10⁻¹⁰ across the full range (see tests). Adjacent ticks differ by
//! a relative 5·10⁻⁵, i.e. 5 orders of magnitude more than the error, so the
//! function is **strictly increasing** in `i` — the property the inverse
//! relies on and the fuzz tests check.
//!
//! # `tick_at_sqrt_price` — correctness
//!
//! Returns the greatest tick `i` with `sqrt_price_at_tick(i) ≤ p`. It
//! computes `log_{sqrt(1.0001)}(p / 2^96)` as
//! `log2(p · 2^32 / 2^128) / log2(sqrt(1.0001))`:
//!
//! 1. the integer part of `log2` is the most significant bit (O(1));
//! 2. 14 fractional bits are produced by repeated squaring of the
//!    normalised mantissa, bounding the approximation error;
//! 3. multiplying by `2^64 / log2(sqrt(1.0001))` yields a Q128.128 value.
//!    The two error-bound constants give the lowest and highest tick the
//!    exact logarithm can round to; they differ by at most one, and the
//!    ambiguity is resolved exactly with one call to `sqrt_price_at_tick`.
//!
//! The result is therefore exact, which the fuzz tests verify against the
//! defining inequality `sqrtP(i) ≤ p < sqrtP(i + 1)`.
//!
//! # Tick spacing and liquidity bounds
//!
//! Positions may only start and end on multiples of `tick_spacing`. The
//! usable range is `[⌈MIN_TICK/s⌉·s, ⌊MAX_TICK/s⌋·s]`, which holds
//!
//! ```text
//! n(s) = (max_usable - min_usable)/s + 1
//! ```
//!
//! initialisable ticks. Capping the gross liquidity referenced by any one
//! tick at `L_max(s) = ⌊(2^128 - 1) / n(s)⌋` ensures that the pool's active
//! liquidity — a sum of at most `n(s)` such contributions — satisfies
//! `L_active ≤ n(s) · L_max(s) ≤ 2^128 - 1`, so it can never overflow a
//! `u128` no matter how positions are arranged.

use ethnum::{I256, U256};

pub const MIN_TICK: i32 = -887_272;
pub const MAX_TICK: i32 = -MIN_TICK;

/// `sqrt_price_at_tick(MIN_TICK)`.
pub const MIN_SQRT_PRICE: U256 = U256::new(4_295_128_739);
/// `sqrt_price_at_tick(MAX_TICK)`.
pub const MAX_SQRT_PRICE: U256 = U256::from_words(0xfffd8963, 0xefd1fc6a506488495d951d5263988d26);

/// Largest permitted tick spacing.
pub const MAX_TICK_SPACING: i32 = 16_384;

/// `round(sqrt(1.0001)^-(2^k) · 2^128)` for `k = 0..20`.
const TICK_FACTORS: [u128; 20] = [
    0xfffcb933bd6fad37aa2d162d1a594001,
    0xfff97272373d413259a46990580e213a,
    0xfff2e50f5f656932ef12357cf3c7fdcc,
    0xffe5caca7e10e4e61c3624eaa0941cd0,
    0xffcb9843d60f6159c9db58835c926644,
    0xff973b41fa98c081472e6896dfb254c0,
    0xff2ea16466c96a3843ec78b326b52861,
    0xfe5dee046a99a2a811c461f1969c3053,
    0xfcbe86c7900a88aedcffc83b479aa3a4,
    0xf987a7253ac413176f2b074cf7815e54,
    0xf3392b0822b70005940c7a398e4b70f3,
    0xe7159475a2c29b7443b29c7fa6e889d9,
    0xd097f3bdfd2022b8845ad8f792aa5825,
    0xa9f746462d870fdf8a65dc1f90e061e5,
    0x70d869a156d2a1b890bb3df62baf32f7,
    0x31be135f97d08fd981231505542fcfa6,
    0x9aa508b5b7a84e1c677de54f3e99bc9,
    0x5d6af8dedb81196699c329225ee604,
    0x2216e584f5fa1ea926041bedfe98,
    0x48a170391f7dc42444e8fa2,
];

/// `2^64 / log2(sqrt(1.0001))`, rounded up (Q64.64 → Q128.128 scaling).
const LOG_SQRT10001_SCALE: i128 = 255_738_958_999_603_826_347_141;
/// Error bounds of the 14-bit log2 approximation, in Q128.128.
const TICK_LOW_ERROR: I256 = U256::new(3_402_992_956_809_132_418_596_140_100_660_247_210).as_i256();
const TICK_HIGH_ERROR: I256 =
    U256::new(291_339_464_771_989_622_907_027_621_153_398_088_495).as_i256();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TickError {
    TickOutOfRange,
    SqrtPriceOutOfRange,
    InvalidTickSpacing,
    TickNotAligned,
    InvalidRange,
}

/// `sqrt(1.0001)^tick · 2^96`, rounded up. Strictly increasing in `tick`.
pub fn sqrt_price_at_tick(tick: i32) -> Result<U256, TickError> {
    if !(MIN_TICK..=MAX_TICK).contains(&tick) {
        return Err(TickError::TickOutOfRange);
    }
    let abs = tick.unsigned_abs();
    let mut ratio = if abs & 1 != 0 {
        U256::new(TICK_FACTORS[0])
    } else {
        U256::from_words(1, 0) // 1.0 in Q128.128
    };
    for (k, factor) in TICK_FACTORS.iter().enumerate().skip(1) {
        if abs & (1 << k) != 0 {
            ratio = (ratio * U256::new(*factor)) >> 128u32;
        }
    }
    if tick > 0 {
        ratio = U256::MAX / ratio;
    }
    // Q128.128 → Q64.96, rounding up.
    let round_up = if ratio & U256::new(0xffff_ffff) != 0 {
        U256::ONE
    } else {
        U256::ZERO
    };
    Ok((ratio >> 32u32) + round_up)
}

/// Greatest tick `i` with `sqrt_price_at_tick(i) <= sqrt_price`.
///
/// `sqrt_price` must lie in `[MIN_SQRT_PRICE, MAX_SQRT_PRICE)`.
pub fn tick_at_sqrt_price(sqrt_price: U256) -> Result<i32, TickError> {
    if sqrt_price < MIN_SQRT_PRICE || sqrt_price >= MAX_SQRT_PRICE {
        return Err(TickError::SqrtPriceOutOfRange);
    }
    // Q64.96 → Q128.128.
    let ratio = sqrt_price << 32u32;
    let msb = 255 - ratio.leading_zeros();
    // Normalise the mantissa into [2^127, 2^128).
    let mut r = if msb >= 128 {
        ratio >> (msb - 127)
    } else {
        ratio << (127 - msb)
    };

    // Integer part of log2 in Q64.64, then 14 fractional bits.
    let mut log_2 = (I256::new(msb as i128) - 128) << 64u32;
    for bit in (50u32..=63).rev() {
        r = (r * r) >> 127u32;
        let f = r >> 128u32; // 0 or 1
        log_2 |= f.as_i256() << bit;
        r >>= f.as_u32();
    }

    let log_sqrt10001 = log_2 * I256::new(LOG_SQRT10001_SCALE);
    let tick_low = ((log_sqrt10001 - TICK_LOW_ERROR) >> 128u32).as_i32();
    let tick_high = ((log_sqrt10001 + TICK_HIGH_ERROR) >> 128u32).as_i32();

    Ok(
        if tick_low == tick_high || sqrt_price_at_tick(tick_high)? > sqrt_price {
            tick_low
        } else {
            tick_high
        },
    )
}

pub fn validate_tick_spacing(spacing: i32) -> Result<(), TickError> {
    if (1..=MAX_TICK_SPACING).contains(&spacing) {
        Ok(())
    } else {
        Err(TickError::InvalidTickSpacing)
    }
}

/// Smallest multiple of `spacing` that is `>= MIN_TICK`.
pub fn min_usable_tick(spacing: i32) -> i32 {
    -(MAX_TICK / spacing) * spacing
}

/// Largest multiple of `spacing` that is `<= MAX_TICK`.
pub fn max_usable_tick(spacing: i32) -> i32 {
    (MAX_TICK / spacing) * spacing
}

/// Number of initialisable ticks `n(s)`.
pub fn usable_tick_count(spacing: i32) -> u32 {
    ((max_usable_tick(spacing) - min_usable_tick(spacing)) / spacing) as u32 + 1
}

/// Per-tick gross liquidity cap `L_max(s) = ⌊u128::MAX / n(s)⌋`.
pub fn max_liquidity_per_tick(spacing: i32) -> u128 {
    u128::MAX / u128::from(usable_tick_count(spacing))
}

/// Validates a position range `[lower, upper)` for the given spacing.
pub fn check_range(lower: i32, upper: i32, spacing: i32) -> Result<(), TickError> {
    validate_tick_spacing(spacing)?;
    if lower >= upper {
        return Err(TickError::InvalidRange);
    }
    if lower < MIN_TICK || upper > MAX_TICK {
        return Err(TickError::TickOutOfRange);
    }
    if lower % spacing != 0 || upper % spacing != 0 {
        return Err(TickError::TickNotAligned);
    }
    Ok(())
}
