# Synthetix Core — Synthetic Asset Issuance & Debt Tracking

Closes #259.

A Synthetix-style synthetic-asset engine for Stellar/Soroban. Users lock
protocol tokens as collateral and mint synthetic assets (`sBTC`, `sXLM`, …)
that track external oracle prices. Debt is tracked in a shared, index-based
pool so that re-pricing the pool costs `O(1)` in the number of minters, and
under-collateralised accounts are resolved by a permissionless, *localized*
liquidation engine rather than a global sweep.

Implements issue #259.

## The three properties this contract guarantees

### 1. A dynamic global debt pool — `O(1)`, not `O(minters)`

Debt is held as **debt shares**. The pool keeps two global counters per
synthetic currency (`total_debt`, `total_shares`) and a minter's debt is a pure
function of them:

```text
debt_per_share = total_debt / total_shares            O(1)
debt_of(s)     = total_debt * s / total_shares        O(1)
```

A position is **marked** at

```text
marked_debt = max(indexed_debt, synth_units_held * current_price)
```

so an oracle price move re-prices every position at once with **no write at
all** — no minter record is read or written during a revaluation. `DebtPool::mutations`
makes the property observable on chain: it counts global-record rewrites and
stays constant per user action regardless of how many minters are active.

The `max` is monotone in the safe direction. A price **surge** marks a position
up immediately and can push it under the threshold; a price **fall** can never
shrink a recorded debt and hand collateral back to a minter.

The index itself is stationary by design: minting and retiring are both pro rata
against it, so `debt_per_share` only ever moves by the rounding surplus a `ceil`
share surrender cannot give back. `high_water_debt_per_share` tracks the peak so
that drift is observable rather than silent.

### 2. 300%+ collateralisation on every mint

`mint_synth` recomputes the minter's **total** debt across every registered
currency against their locked collateral and refuses the mint below 300%. The
floor is a safety property, so the ratio ratchets: governance can tighten it but
never loosen it, and a request at or below the ratio in force is rejected
outright rather than silently ignored.

The currency registry is a capped, admin-curated allow-list (`MAX_CURRENCIES`),
which is what keeps `total_debt` and the collateralisation check `O(currencies)`
rather than `O(minters)`.

### 3. A localized liquidation engine

`liquidate` lets anyone redeem part of a shortfall account's debt using the
synthetic units they already hold, and hands them that account's collateral plus
a `penalty_bps` surcharge. It is *localized* in two senses:

* only the liquidator's and the target's positions are touched — no other
  minter's shares are read or written; and
* the trade is asserted non-fraternal: the liquidator's post-trade ratio may not
  fall below `min(ratio before, target ratio after)`.

**Why the liquidator is checked relatively, not absolutely.** Positions are
marked at `max(indexed debt, synths × price)`, so after a price surge *every*
synth holder is already below the threshold. An absolute "stay above 300%" gate
on the liquidator would therefore make liquidation impossible exactly when it is
needed — the classic liquidation-deadlock failure mode. The relative bound holds
by construction (collateral strictly increases, effective debt strictly
decreases) and is enforced as a post-condition, so any future change to the
accounting that breaks it reverts the trade.

The penalty does **not** leave the protocol. The target hands over
`debt_to_cover` *plus* the penalty as collateral, and the tokens are still held,
so the forfeited value accrues to `surplus()` as extra backing for the debt that
remains — exactly as in Synthetix. `total_collateral` is unchanged by a
liquidation, so the sum of every minter's balance keeps equalling the protocol
total.

**Why there is no batch liquidation.** Synthetix's period-end batch liquidation
and its "all synths" backstop mode are global sweeps that walk every minter —
precisely the cost model this contract exists to avoid — and both can
concentrate control of the whole pool in a single actor. Shortfalls here are
resolved by whoever is willing to take the penalised collateral, which is
permissionless, bounded and incentive-compatible.

## Interface

| Group | Calls |
|-------|-------|
| Init / config | `initialize`, `add_currency`, `set_price`, `set_collateral_price`, `set_min_ratio_bps`, `set_penalty_bps`, `set_paused` |
| Collateral | `lock_collateral`, `withdraw_collateral` |
| Synthetic | `mint_synth`, `burn_synth` |
| Shortfall | `flag_account`, `is_flagged`, `flagged_at`, `liquidate` |
| Views | `debt_of`, `indexed_debt_of`, `total_debt`, `shares_of`, `synth_units_of`, `collateral_of`, `collateral_value_of`, `collateral_ratio_bps`, `pool`, `total_collateral`, `surplus`, `currencies`, `min_ratio_bps`, `penalty_bps`, `collateral_price`, `admin`, `required_collateral_for`, `synth_units_for`, `max_price` |

Collateral withdrawal stays open while the contract is paused, so a user is never
trapped by the emergency stop. `flag_account` is permissionless; it is advisory
only, because `liquidate` re-checks the ratio on chain.

## Storage layout

| Key | Type | Storage | Purpose |
|-----|------|---------|---------|
| `ADMIN` | `Address` | Instance | Privileged caller (admin / oracle) |
| `COLLTRL` | `Address` | Instance | Token accepted as collateral |
| `CPRICE` | `u128` | Instance | Collateral price, 1e9 fixed point |
| `RATIO` | `i128` | Instance | Minimum collateralisation ratio, bps |
| `PENALTY` | `i128` | Instance | Liquidation penalty, bps |
| `TOTCOL` | `i128` | Instance | Protocol-wide locked collateral |
| `SURPLUS` | `i128` | Instance | Value forfeited by liquidated accounts |
| `CURS` | `Vec<Address>` | Instance | Registered synthetic currencies |
| `PAUSED` | `bool` | Instance | Emergency stop |
| `POOL` | `DebtPool` | Persistent | Per-currency global debt counters |
| `PRICE` | `u128` | Persistent | Per-currency oracle price |
| `SHARES` | `u128` | Persistent | Minter's debt shares for a currency |
| `SYNTH` | `i128` | Persistent | Minter's synthetic-unit balance |
| `COLL` | `i128` | Persistent | Minter's locked collateral |
| `FLAG` | `bool` | Persistent | Under-collateralisation flag |
| `FLAGTS` | `u64` | Persistent | Ledger timestamp the flag was set at |

Every key a user can touch is persistent and has its TTL extended on each access,
so an active position can never be archived out from under a minter.

## Fixed-point conventions

* Prices: `u128`, `PRICE_PRECISION = 1e9` (e.g. `50_000_000_000` for $50.00).
* Amounts: `i128`, capped at `MAX_POOL_AMOUNT = 1e18` and `MAX_COLLATERAL = 1e24`.
* Ratios: basis points, `MIN_COLLATERAL_RATIO_BPS = 30_000` (300%).
* The index: `u128`, `PRECISION = 1e18`.

All arithmetic is done in `u128` with every narrowing back to `i128` checked, so
a pool can never be driven into a silent wrap-around. Widest intermediate is
`1e18 × 1e18 = 1e36`, well inside `u128::MAX ≈ 3.4e38`.

## Tests

```bash
cargo test                      # 40 unit + integration tests
cargo test --release -- --ignored   # 10,000-minter stress run (a few minutes)
```

`src/test.rs` covers the 300% boundary, the price surge and its aftermath,
burn/burden-shifting, permissionless flagging, the full liquidation engine
(penalty, surplus, non-fraternity, localization), admin guards and the emergency
stop.

`tests/stress_debt_pool.rs` is the issue's 10,000-mintter requirement. It asserts
that the sum of all 10,000 indexed debts equals the pool's global counter to the
base unit, that a surge rewrites the global record exactly once while re-marking
all 10,000 positions, that liquidating one minter leaves the other 9,999 records
byte-for-byte identical, and — by measuring CPU instructions — that the cost of a
price update and of a position read is the same in a 10,000-minter pool as in a
single-minter pool.

These two are `#[ignore]`d because enrolling 10,000 minters is 20,000 contract
invocations, which would make an ordinary `cargo test` run take minutes.

## Local build notes

The crate is a standalone Soroban workspace, independent of the root
Stellar-K8s operator workspace. Building the `cdylib` for wasm is the deployment
path; the `cdylib` target does not link under the `x86_64-pc-windows-gnu` toolchain
(`ld: export ordinal too large`), so on Windows use `crate-type = ["rlib"]` or
test on a Linux/wasm toolchain.

## Relationship to the Ethereum reference implementation

Synthetix V2 re-values each account's `debtBalance` *lazily*, on that account's
next touch, and leaves the global `synthTotalSupply` untouched. This contract
re-derives debt on every read instead. The consequences:

* Debt is always current, so there is no staleness window to exploit.
* The marginal cost of servicing an extra minter is zero.
* Retiring debt always benefits the remaining holders, so rounding never
  confiscates value from a position that did nothing wrong.
