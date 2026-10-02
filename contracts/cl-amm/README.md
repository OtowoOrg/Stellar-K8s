# Concentrated Liquidity AMM

A Uniswap-v3-style pool for Soroban. Liquidity providers place capital in
bounded price ranges made of discrete ticks.

## Status: first increment (tick-based pricing)

| Part | Status |
|------|--------|
| Tick ↔ √price conversion in Q64.96 fixed point (`ticks.rs`) | ✅ |
| Tick-spacing rules and per-tick liquidity cap | ✅ |
| Proofs of the tick and liquidity-cap formulas (module docs of [`src/ticks.rs`](src/ticks.rs)) | ✅ |
| Pool configuration and one-time price initialisation (`lib.rs`) | ✅ |
| Positions, swaps that cross ticks, and fee accrual | ⏳ |

## Pricing model

Tick `i` has price `1.0001^i`, so neighbouring ticks differ by one basis
point. The pool stores `√price · 2^96`.

| Function | Result |
|----------|--------|
| `sqrt_price_at_tick(i)` | Exact integer math with 256-bit intermediates, rounded up. Strictly increasing in `i`. |
| `tick_at_sqrt_price(p)` | The greatest `i` with `sqrt_price_at_tick(i) ≤ p`. Computed in O(1) with a fixed-point base-2 logarithm, then settled exactly with one extra evaluation. |
| `max_liquidity_per_tick(s)` | `⌊(2^128 − 1) / n(s)⌋`, where `n(s)` is the number of usable ticks. Active liquidity can therefore never overflow `u128`. |

The tick range is `±887272`, which gives √price values in
`[4295128739, 1461446703485210103287273052203988822378723970342]`. These
bounds and outputs are identical to Uniswap v3's `TickMath`.

## Verification

- **Reference values:** outputs match Uniswap v3 exactly. They stay within
  a relative 10⁻⁹ of values computed separately with 150-digit decimal
  arithmetic. The measured maximum error is 2.1·10⁻¹⁰, which is 5 orders of
  magnitude below the 10⁻⁴ gap between ticks.
- **Fuzz tests (`proptest`, 4,096 cases each):**
  - `sqrt_price_at_tick` is strictly increasing;
  - converting a tick to a price and back returns the same tick, for every
    tick sampled;
  - `sqrtP(t) ≤ p < sqrtP(t+1)` holds for prices inside a tick and for
    random prices across the whole range.
- **Tick spacing:** for every spacing from 1 to 16384, the cap satisfies
  `n · L_max ≤ u128::MAX`, and no larger cap does. The usable range is
  aligned to the spacing and as wide as the bounds allow.

```sh
cargo test
cargo build --target wasm32v1-none --release
```
