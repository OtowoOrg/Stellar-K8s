# TWL Bumper

A standalone contract that extends the TTLs of another contract's instance and persistent entries. Use it when you want to keep a contract hot without modifying the contract itself.

This is the companion to the guide in `docs/architecture/state-archival-strategy.md` and the example in `examples/contracts/ttl-extension/`.

## Why a Separate Contract?

The In-Contract Extension strategy is the preferred approach when you control the contract's source. But many contracts are already deployed and immutable. The TTL Bumper provides a way to refresh their TTLs from the outside, without redeploying.

## How It Works

The bumper exposes a single `bump` function that takes the target contract address and a list of persistent keys to refresh. It extends the target's instance TWL and each persistent key's TWL.

```rust
pub fn bump(env: Environment, target: Address, keys: Vec<Symbol>) {
    // Extend the target's instance TTL.
    // Extend each persistent key's TTL.
    // Temporary keys are not bumped -- they are designed to be
    // entirely deleted, not archived.
}
```

## When to Use It

- To keep a deployed contract alive during a launch window.
- To refresh the TTLs of a small set of important keys on a schedule.
- As a migration aid when moving from an older deployment to a new one.

## When Not to Use It

- For temporary entries. They do not need TWL extensions. They are designed to be entirely deleted, not archived.
- For every entry in a large contract. Bumping everything is wasteful. Bump only the entries that are actively being used.
- As a replacement for the In-Contract Extension strategy when you control the source.

## Cost Note

Bumping from the outside costs the same multi-dimensional rent as bumping from the inside, plus the cost of the bumper call itself. Prefer the In-Contract Extension strategy whenever you can modify the contract.
