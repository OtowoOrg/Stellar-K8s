//! Fixed-point exponential moving average used by the gas fee oracle.
//!
//! This is the normative reference for the arithmetic described in
//! `docs/architecture/gas-oracle.md` (section "Fixed-point EMA model").
//! All values are unsigned; no signed intermediate is ever formed.

/// Scaling factor `S`: the EMA is stored in units of 10^-7 stroops.
pub const SCALE: u128 = 10_000_000;

/// Denominator `D` of the smoothing factor: `alpha = alpha_bps / D`.
pub const ALPHA_DENOMINATOR: u32 = 10_000;

/// Lifts an observation `x` (stroops per operation) into fixed point: `X = x * S`.
pub fn to_fixed(fee_per_op: u64) -> u128 {
    fee_per_op as u128 * SCALE
}

/// Computes `E_t = floor((a * X_t + (D - a) * E_{t-1}) / D)`.
///
/// Preconditions (enforced by the contract, checked in debug builds):
/// `1 <= alpha_bps <= D` and `prev <= u64::MAX * S`. Under these the
/// numerator is at most `D * u64::MAX * S < 2^128`, so it cannot overflow.
pub fn update(prev: u128, fee_per_op: u64, alpha_bps: u32) -> u128 {
    debug_assert!((1..=ALPHA_DENOMINATOR).contains(&alpha_bps));
    debug_assert!(prev <= to_fixed(u64::MAX));

    let a = alpha_bps as u128;
    let d = ALPHA_DENOMINATOR as u128;
    (a * to_fixed(fee_per_op) + (d - a) * prev) / d
}

/// Converts the fixed-point EMA to whole stroops, rounding up so the
/// quoted fee never under-bids the stored average.
pub fn to_stroops_ceil(ema: u128) -> u64 {
    // `ema <= u64::MAX * S`, so the quotient always fits in a u64.
    ema.div_ceil(SCALE) as u64
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    /// Deterministic pseudo-random fees in `[100, 100_100)` stroops.
    fn fee_sequence(len: usize) -> impl Iterator<Item = u64> {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        (0..len).map(move |_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            100 + (state >> 33) % 100_000
        })
    }

    #[test]
    fn worked_example_from_spec() {
        let e1 = to_fixed(100);
        assert_eq!(e1, 1_000_000_000);

        let e2 = update(e1, 150, 2_000);
        assert_eq!(e2, 1_100_000_000);
        assert_eq!(to_stroops_ceil(e2), 110);

        let e3 = update(e2, 137, 2_000);
        assert_eq!(e3, 1_154_000_000);
        assert_eq!(to_stroops_ceil(e3), 116);
    }

    #[test]
    fn alpha_one_tracks_latest_observation() {
        let ema = update(to_fixed(500), 1_234, ALPHA_DENOMINATOR);
        assert_eq!(ema, to_fixed(1_234));
    }

    #[test]
    fn truncation_error_stays_within_proven_bound() {
        // Reference EMA carried with K extra fractional digits. By the same
        // bound it lies within D/a units of K * (exact EMA), so
        // K * (exact - E_t) lies in [diff, diff + D/a) with diff = R_t - K * E_t.
        const K: u128 = 1_000_000_000_000;
        let d = ALPHA_DENOMINATOR as u128;

        for alpha_bps in [1, 7, 333, 2_000, 9_999] {
            let a = alpha_bps as u128;
            let reference_slack = d.div_ceil(a) as i128;

            let mut fees = fee_sequence(5_000);
            let seed = to_fixed(fees.next().unwrap());
            let (mut fixed, mut reference) = (seed, seed * K);

            for x in fees {
                fixed = update(fixed, x, alpha_bps);
                reference = (a * to_fixed(x) * K + (d - a) * reference) / d;

                let diff = reference as i128 - (fixed * K) as i128;
                // Lower bound: E_t never exceeds the exact EMA (up to 1/K ulp).
                assert!(
                    diff > -reference_slack,
                    "alpha={alpha_bps}: E_t above exact EMA"
                );
                // Upper bound: exact - E_t < D/a ulp.
                assert!(
                    (diff + reference_slack) as u128 * a <= d * K,
                    "alpha={alpha_bps}: truncation error exceeded D/a"
                );
            }
        }
    }

    #[test]
    fn constant_input_converges_within_deadband() {
        for alpha_bps in [1, 3, 2_000, ALPHA_DENOMINATOR] {
            let target = to_fixed(10_000);
            let mut ema = to_fixed(100);
            for _ in 0..200_000 {
                let next = update(ema, 10_000, alpha_bps);
                assert!(next >= ema && next <= target);
                if next == ema {
                    break;
                }
                ema = next;
            }
            let gap = target - ema;
            assert!(gap * (alpha_bps as u128) < ALPHA_DENOMINATOR as u128);
        }
    }

    #[test]
    fn extreme_inputs_do_not_overflow() {
        for alpha_bps in [1, ALPHA_DENOMINATOR / 2, ALPHA_DENOMINATOR] {
            let mut ema = to_fixed(u64::MAX);
            for x in [u64::MAX, 0, u64::MAX, 1, u64::MAX] {
                ema = update(ema, x, alpha_bps);
                assert!(ema <= to_fixed(u64::MAX));
            }
            assert_eq!(to_stroops_ceil(to_fixed(u64::MAX)), u64::MAX);
        }
    }
}
