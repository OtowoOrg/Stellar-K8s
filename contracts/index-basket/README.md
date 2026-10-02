# Index Basket & Factory

Soroban contracts for deploying diversified, fully collateralised **basket
tokens** (index funds) over multiple SEP-41 assets, such as 50% XLM / 30% USDC /
20% sBTC.

| Crate | Path | Role |
|-------|------|------|
| `index-basket` | [`src/`](src) | The basket. It is also the SEP-41 share token. |
| `index-basket-factory` | [`factory/`](factory) | Deploys baskets from an uploaded WASM hash and keeps a registry of them. |

## Basket lifecycle

1. **Create.** The factory's `create_basket(creator, salt, config)` deploys a
   basket with `creator` as its admin. The config lists each component's
   token, `units` (base units backing one whole basket token at bootstrap), and
   target `weight_bps`. The weights must sum to 10 000. If the config is
   invalid, the deployment aborts and nothing is registered.
2. **Issue.** `issue(user, shares, max_amounts)` pulls every underlying asset in
   the ratio the basket currently holds and mints `shares`.
3. **Redeem.** `redeem(user, shares, min_amounts)` burns `shares` and releases
   the pro-rata amount of every underlying asset.
4. **Rebalance.** After prices drift, allow-listed arbitrageurs call
   `rebalance(arb, asset_in, amount_in, asset_out, min_amount_out)`. They
   deposit an underweight asset and withdraw an overweight one at SEP-40 oracle
   prices, plus an optional premium capped at 1%.

## Guarantees

- **Perfect collateralisation.** Deposits round up and withdrawals round down,
  whatever each token's precision. A round trip can never extract value. The
  last redeemer receives all remaining reserves, including rounding dust.
- **Donation-proof accounting.** Reserves are tracked internally. Sending tokens
  directly to the basket does not change its share price.
- **Atomic multi-asset transfers.** Issue, redeem and rebalance validate every
  amount, commit state, and then perform all transfers in a single invocation.
  If any transfer fails (for example, a frozen trustline), the whole call
  reverts.
- **Rebalancing can only improve the basket's allocation.** A trade must deposit
  an underweight asset and withdraw an overweight one. After the trade, neither
  leg may be past its target. Valuations use exact 256-bit arithmetic,
  normalised across token precisions (0–18 decimals).
  `quote_rebalance(asset_in, asset_out)` returns the largest trade allowed for a
  pair.
- **Oracle safety.** Prices must be positive and no older than `max_price_age`.

Using SEP-41 `burn` destroys shares without releasing collateral. That
collateral goes to the remaining holders. To withdraw assets, use `redeem`.

## Build & test

Requires the `wasm32v1-none` target (`rustup target add wasm32v1-none`). The
factory tests deploy the compiled basket WASM, so build it first:

```sh
make test   # builds the WASMs, then runs `cargo test --workspace`
make lint   # rustfmt check + clippy (host and wasm) with -D warnings
```

The test suites cover:

- tokens with 0, 6, 7, 8, 9 and 18 decimals, and basket precisions of 0, 7, 9
  and 18
- rounding and round-trip invariants
- atomic redemption when one of the transfers fails
- SEP-41 behaviour
- every rebalance guard
- the issue's validation scenario: a 5-asset basket whose prices drift is
  rebalanced back to 20% weights, both natively and through the
  factory-deployed WASM
