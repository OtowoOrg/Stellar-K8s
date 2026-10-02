extern crate std;

use ethnum::U256;
use proptest::prelude::*;
use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::ticks::{self, TickError, MAX_SQRT_PRICE, MAX_TICK, MIN_SQRT_PRICE, MIN_TICK};
use crate::{ClAmm, ClAmmClient, PoolError};

fn u(s: &str) -> U256 {
    s.parse().unwrap()
}

fn sqrt(t: i32) -> U256 {
    ticks::sqrt_price_at_tick(t).unwrap()
}

// ---------------------------------------------------------------------------
// sqrt_price_at_tick
// ---------------------------------------------------------------------------

/// `(tick, expected output, floor(exact · 2^96))`. Expected outputs match
/// Uniswap v3's TickMath; exact values were computed independently with
/// 150-digit decimal arithmetic.
const VECTORS: [(i32, &str, &str); 13] = [
    (
        0,
        "79228162514264337593543950336",
        "79228162514264337593543950336",
    ),
    (
        1,
        "79232123823359799118286999568",
        "79232123823359799118286999567",
    ),
    (
        -1,
        "79224201403219477170569942574",
        "79224201403219477170569942573",
    ),
    (
        50,
        "79426470787362580746886972461",
        "79426470787362580746886972460",
    ),
    (
        -50,
        "79030349367926598376800521322",
        "79030349367926598376800521321",
    ),
    (
        1000,
        "83290069058676223003182343270",
        "83290069058676223003182343269",
    ),
    (
        -1000,
        "75364347830767020784054125655",
        "75364347830767020784054125654",
    ),
    (
        100000,
        "11755562826496067164730007768450",
        "11755562826496067164730007768449",
    ),
    (
        -100000,
        "533968626430936354154228408",
        "533968626430936354154228407",
    ),
    (
        443636,
        "340275971719517849884101479065584693834",
        "340275971719517849884101479063205952279",
    ),
    (-443636, "18447090764788882728", "18447090764788882727"),
    (
        887271,
        "1461373636630004318706518188784493106690254656249",
        "1461373636630004318672046398259762639463073250156",
    ),
    (-887271, "4295343490", "4295343489"),
];

#[test]
fn matches_reference_vectors_within_documented_error() {
    for (tick, expected, exact) in VECTORS {
        let got = sqrt(tick);
        assert_eq!(got, u(expected), "tick {tick}");
        // Relative error below 1e-9 (documented bound: < 2.1e-10).
        let exact = u(exact);
        let diff = if got > exact {
            got - exact
        } else {
            exact - got
        };
        assert!(diff * U256::new(1_000_000_000) <= exact, "tick {tick}");
    }
}

#[test]
fn endpoints_and_bounds() {
    assert_eq!(sqrt(MIN_TICK), MIN_SQRT_PRICE);
    assert_eq!(sqrt(MAX_TICK), MAX_SQRT_PRICE);
    assert_eq!(sqrt(0), U256::ONE << 96u32);
    // √P always fits in 160 bits.
    assert!(MAX_SQRT_PRICE < U256::ONE << 160u32);
    assert_eq!(
        ticks::sqrt_price_at_tick(MIN_TICK - 1),
        Err(TickError::TickOutOfRange)
    );
    assert_eq!(
        ticks::sqrt_price_at_tick(MAX_TICK + 1),
        Err(TickError::TickOutOfRange)
    );
}

// ---------------------------------------------------------------------------
// tick_at_sqrt_price
// ---------------------------------------------------------------------------

#[test]
fn inverse_at_the_edges() {
    assert_eq!(ticks::tick_at_sqrt_price(MIN_SQRT_PRICE), Ok(MIN_TICK));
    assert_eq!(
        ticks::tick_at_sqrt_price(MAX_SQRT_PRICE - U256::ONE),
        Ok(MAX_TICK - 1)
    );
    assert_eq!(ticks::tick_at_sqrt_price(U256::ONE << 96u32), Ok(0));
    assert_eq!(
        ticks::tick_at_sqrt_price((U256::ONE << 96u32) - U256::ONE),
        Ok(-1)
    );
    assert_eq!(
        ticks::tick_at_sqrt_price(MIN_SQRT_PRICE - U256::ONE),
        Err(TickError::SqrtPriceOutOfRange)
    );
    assert_eq!(
        ticks::tick_at_sqrt_price(MAX_SQRT_PRICE),
        Err(TickError::SqrtPriceOutOfRange)
    );
}

/// Checks the defining property `sqrtP(t) <= p < sqrtP(t + 1)`.
fn assert_brackets(p: U256) {
    let t = ticks::tick_at_sqrt_price(p).unwrap();
    assert!(sqrt(t) <= p, "sqrtP({t}) > {p}");
    assert!(t < MAX_TICK && p < sqrt(t + 1), "sqrtP({}) <= {p}", t + 1);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn sqrt_price_is_strictly_increasing(t in MIN_TICK..MAX_TICK) {
        prop_assert!(sqrt(t) < sqrt(t + 1));
    }

    #[test]
    fn tick_round_trips(t in MIN_TICK..MAX_TICK) {
        prop_assert_eq!(ticks::tick_at_sqrt_price(sqrt(t)), Ok(t));
    }

    #[test]
    fn tick_brackets_prices_inside_a_bin(t in MIN_TICK..MAX_TICK, frac in any::<u64>()) {
        // A price strictly inside bin t: sqrtP(t) + width · frac / 2^64.
        let lo = sqrt(t);
        let p = lo + (((sqrt(t + 1) - lo) * U256::from(frac)) >> 64u32);
        prop_assert_eq!(ticks::tick_at_sqrt_price(p), Ok(t));
    }

    #[test]
    fn tick_brackets_arbitrary_prices(hi in any::<u32>(), lo in any::<u128>(), bits in 32u32..=160) {
        // Log-uniform over the whole domain.
        let raw = U256::from_words(u128::from(hi), lo) >> (160 - bits);
        let p = raw.clamp(MIN_SQRT_PRICE, MAX_SQRT_PRICE - U256::ONE);
        assert_brackets(p);
    }
}

// ---------------------------------------------------------------------------
// Tick spacing and liquidity cap
// ---------------------------------------------------------------------------

#[test]
fn per_tick_liquidity_cap_prevents_overflow_for_every_spacing() {
    for s in 1..=ticks::MAX_TICK_SPACING {
        let (min, max) = (ticks::min_usable_tick(s), ticks::max_usable_tick(s));
        assert!(min >= MIN_TICK && max <= MAX_TICK && min % s == 0 && max % s == 0);
        // No usable tick is left outside the range.
        assert!(min - s < MIN_TICK && max + s > MAX_TICK);
        let n = u128::from(ticks::usable_tick_count(s));
        let cap = ticks::max_liquidity_per_tick(s);
        // n · L_max fits in u128, and L_max is the largest cap with that property.
        assert!(n.checked_mul(cap).is_some());
        assert!(n.checked_mul(cap + 1).is_none());
    }
}

#[test]
fn liquidity_caps_match_uniswap_v3() {
    assert_eq!(
        ticks::max_liquidity_per_tick(10),
        1_917_569_901_783_203_986_719_870_431_555_990
    );
    assert_eq!(
        ticks::max_liquidity_per_tick(60),
        11_505_743_598_341_114_571_880_798_222_544_994
    );
    assert_eq!(
        ticks::max_liquidity_per_tick(200),
        38_350_317_471_085_141_830_651_933_667_504_588
    );
}

#[test]
fn position_ranges_are_validated() {
    assert_eq!(ticks::check_range(-60, 60, 60), Ok(()));
    assert_eq!(ticks::check_range(60, 60, 60), Err(TickError::InvalidRange));
    assert_eq!(
        ticks::check_range(-61, 60, 60),
        Err(TickError::TickNotAligned)
    );
    assert_eq!(
        ticks::check_range(MIN_TICK - 1, 0, 1),
        Err(TickError::TickOutOfRange)
    );
    assert_eq!(
        ticks::check_range(0, 10, 0),
        Err(TickError::InvalidTickSpacing)
    );
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

fn tokens(env: &Env) -> (Address, Address) {
    let (a, b) = (Address::generate(env), Address::generate(env));
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

fn pool(env: &Env) -> ClAmmClient<'_> {
    let (t0, t1) = tokens(env);
    let id = env.register(ClAmm, (Address::generate(env), t0, t1, 3_000u32, 60i32));
    ClAmmClient::new(env, &id)
}

fn host_u256(env: &Env, v: U256) -> soroban_sdk::U256 {
    soroban_sdk::U256::from_parts(
        env,
        (v >> 192u32).as_u64(),
        (v >> 128u32).as_u64(),
        (v >> 64u32).as_u64(),
        v.as_u64(),
    )
}

#[test]
fn constructor_stores_config_with_liquidity_cap() {
    let env = Env::default();
    let c = pool(&env).config();
    assert_eq!((c.fee, c.tick_spacing), (3_000, 60));
    assert_eq!(c.max_liquidity_per_tick, ticks::max_liquidity_per_tick(60));
    assert!(c.token0 < c.token1);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn constructor_rejects_unsorted_tokens() {
    let env = Env::default();
    let (t0, t1) = tokens(&env);
    env.register(ClAmm, (Address::generate(&env), t1, t0, 3_000u32, 60i32));
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn constructor_rejects_fee_of_100_percent() {
    let env = Env::default();
    let (t0, t1) = tokens(&env);
    env.register(
        ClAmm,
        (Address::generate(&env), t0, t1, 1_000_000u32, 60i32),
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn constructor_rejects_invalid_tick_spacing() {
    let env = Env::default();
    let (t0, t1) = tokens(&env);
    env.register(ClAmm, (Address::generate(&env), t0, t1, 3_000u32, 0i32));
}

#[test]
fn initialize_sets_price_and_tick_once() {
    let env = Env::default();
    env.mock_all_auths();
    let p = pool(&env);
    // Price 1.0001^1000.5 lies inside bin 1000.
    let sqrt_price = sqrt(1000) + U256::ONE;
    assert_eq!(p.initialize(&host_u256(&env, sqrt_price)), 1000);
    let slot0 = p.slot0().unwrap();
    assert_eq!(slot0.tick, 1000);
    assert_eq!(slot0.sqrt_price_x96, host_u256(&env, sqrt_price));
    assert_eq!(
        p.try_initialize(&host_u256(&env, sqrt_price)).unwrap_err(),
        Ok(PoolError::AlreadyInitialized.into())
    );
}

#[test]
fn initialize_is_admin_only_and_range_checked() {
    let env = Env::default();
    let p = pool(&env);
    assert!(p
        .try_initialize(&host_u256(&env, U256::ONE << 96u32))
        .is_err());
    env.mock_all_auths();
    assert_eq!(
        p.try_initialize(&host_u256(&env, MAX_SQRT_PRICE))
            .unwrap_err(),
        Ok(PoolError::SqrtPriceOutOfRange.into())
    );
    assert!(p.slot0().is_none());
}

#[test]
fn contract_views_match_library() {
    let env = Env::default();
    let p = pool(&env);
    for t in [MIN_TICK, -1000, 0, 443636, MAX_TICK] {
        assert_eq!(p.sqrt_price_at_tick(&t), host_u256(&env, sqrt(t)));
    }
    assert_eq!(p.tick_at_sqrt_price(&host_u256(&env, sqrt(-1000))), -1000);
}
