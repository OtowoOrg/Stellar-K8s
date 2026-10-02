/// Dutch Auction Pricing Curve
///
/// Implements a linearly-decaying price curve:
///
///   price(t) = start_price - (start_price - reserve_price) * elapsed / duration
///
/// where `elapsed = min(current_time - start_time, duration)`.
///
/// Properties:
/// - At t = start_time:            price == start_price   (highest price)
/// - At t = start_time + duration: price == reserve_price (price floor)
/// - Price never drops below reserve_price (strictly enforced)
/// - All inputs are integer-only (no floating point) for deterministic on-chain execution

/// Calculate the current clearing price for a Dutch Auction.
///
/// # Arguments
/// * `start_price`   – price per token at auction open (in reserve-token units, e.g. stroops)
/// * `reserve_price` – minimum price floor; price never drops below this value
/// * `start_time`    – ledger timestamp when the auction opened (seconds)
/// * `end_time`      – ledger timestamp when the auction closes (seconds); must be > start_time
/// * `current_time`  – ledger timestamp of the current invocation (seconds)
///
/// # Returns
/// The current price per token, clamped to [reserve_price, start_price].
///
/// # Panics
/// Panics if `start_price < reserve_price` or `end_time <= start_time`.
pub fn current_price(
    start_price: i128,
    reserve_price: i128,
    start_time: u64,
    end_time: u64,
    current_time: u64,
) -> i128 {
    assert!(
        start_price >= reserve_price,
        "start_price must be >= reserve_price"
    );
    assert!(end_time > start_time, "end_time must be > start_time");

    // Before the auction starts, use start_price
    if current_time <= start_time {
        return start_price;
    }

    // After the auction ends, use reserve_price
    if current_time >= end_time {
        return reserve_price;
    }

    let duration = (end_time - start_time) as i128;
    let elapsed = (current_time - start_time) as i128;
    let price_drop = start_price - reserve_price;

    // Linear interpolation: price = start_price - price_drop * elapsed / duration
    // Integer arithmetic — rounds toward zero (always stays >= reserve_price when
    // elapsed < duration, which is guaranteed by the branch above).
    let decay = price_drop * elapsed / duration;
    let price = start_price - decay;

    // Clamp: in case of any rounding edge, never drop below the reserve floor.
    if price < reserve_price {
        reserve_price
    } else {
        price
    }
}

/// Calculate the number of tokens a given deposit can purchase at the clearing price.
///
/// Uses integer division (floor). Any remainder stays as a refundable overpayment.
///
/// # Arguments
/// * `deposit_amount` – total funds deposited by the buyer (in reserve-token units)
/// * `price_per_token` – current clearing price per token
///
/// # Returns
/// Number of whole tokens that can be purchased, or 0 if price is 0.
pub fn tokens_for_deposit(deposit_amount: i128, price_per_token: i128) -> i128 {
    if price_per_token <= 0 {
        return 0;
    }
    deposit_amount / price_per_token
}

/// Calculate the cost of purchasing exactly `token_amount` tokens at `price_per_token`.
pub fn cost_for_tokens(token_amount: i128, price_per_token: i128) -> i128 {
    token_amount * price_per_token
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── current_price ───────────────────────────────────────────────────────

    #[test]
    fn price_at_start_equals_start_price() {
        assert_eq!(current_price(1_000, 100, 0, 1_000, 0), 1_000);
    }

    #[test]
    fn price_at_end_equals_reserve_price() {
        assert_eq!(current_price(1_000, 100, 0, 1_000, 1_000), 100);
    }

    #[test]
    fn price_at_midpoint_is_midway() {
        // start=1000, reserve=100, halfway => 550
        let p = current_price(1_000, 100, 0, 1_000, 500);
        assert_eq!(p, 550);
    }

    #[test]
    fn price_decreases_monotonically() {
        let start = 10_000i128;
        let reserve = 1_000i128;
        let mut prev = start;
        for t in (0..=1_000u64).step_by(100) {
            let p = current_price(start, reserve, 0, 1_000, t);
            assert!(p <= prev, "price must be non-increasing at t={}", t);
            prev = p;
        }
    }

    #[test]
    fn price_never_below_reserve() {
        let p = current_price(1_000, 100, 0, 1_000, 999_999);
        assert!(p >= 100, "price must stay >= reserve_price");
    }

    #[test]
    fn price_before_start_is_start_price() {
        // current_time < start_time
        let p = current_price(1_000, 100, 500, 1_000, 200);
        assert_eq!(p, 1_000);
    }

    #[test]
    fn price_after_end_is_reserve_price() {
        let p = current_price(1_000, 100, 0, 1_000, 2_000);
        assert_eq!(p, 100);
    }

    #[test]
    fn flat_curve_when_start_equals_reserve() {
        // If start_price == reserve_price, price never moves.
        let p1 = current_price(500, 500, 0, 1_000, 0);
        let p2 = current_price(500, 500, 0, 1_000, 500);
        let p3 = current_price(500, 500, 0, 1_000, 1_000);
        assert_eq!(p1, 500);
        assert_eq!(p2, 500);
        assert_eq!(p3, 500);
    }

    #[test]
    fn price_uses_immutable_timestamps() {
        // Changing only current_time with fixed start/end gives deterministic values.
        let p_quarter = current_price(1_000, 0, 0, 1_000, 250);
        let p_three_quarter = current_price(1_000, 0, 0, 1_000, 750);
        assert_eq!(p_quarter, 750);
        assert_eq!(p_three_quarter, 250);
    }

    // ─── tokens_for_deposit ──────────────────────────────────────────────────

    #[test]
    fn tokens_for_deposit_exact_multiple() {
        assert_eq!(tokens_for_deposit(1_000, 100), 10);
    }

    #[test]
    fn tokens_for_deposit_rounds_down() {
        // 1_050 / 100 = 10 (not 11)
        assert_eq!(tokens_for_deposit(1_050, 100), 10);
    }

    #[test]
    fn tokens_for_deposit_zero_price() {
        assert_eq!(tokens_for_deposit(1_000, 0), 0);
    }

    #[test]
    fn tokens_for_deposit_large_numbers() {
        // 1e18 / 1e9 = 1e9
        assert_eq!(
            tokens_for_deposit(1_000_000_000_000_000_000, 1_000_000_000),
            1_000_000_000
        );
    }

    // ─── cost_for_tokens ─────────────────────────────────────────────────────

    #[test]
    fn cost_for_tokens_basic() {
        assert_eq!(cost_for_tokens(10, 100), 1_000);
    }

    #[test]
    fn cost_for_zero_tokens() {
        assert_eq!(cost_for_tokens(0, 500), 0);
    }
}
