# Staking Vault Contract

A decentralized staking and continuous yield distribution engine for the Stellar/Soroban ecosystem. Implements the Synthetix/Uniswap `StakingRewards` algorithmic model adapted for Soroban smart contracts.

## Features

- **Continuous reward accrual** using the per-token accumulator pattern to guarantee zero rounding drift across any number of stakers.
- **Deposit / Withdraw** staked principal with automatic reward checkpoint on every state transition.
- **Claim** accrued rewards at any time.
- **Compound** rewards back into stake (when staking token == reward token).
- **Emergency Withdraw** — bypasses reward calculation entirely when the contract is paused. Guarantees capital recovery during emergency stops.
- **Admin pause / unpause** for operational safety.

## Modules

| File | Purpose |
|------|---------|
| `src/lib.rs` | Contract entry-points: initialize, deposit, withdraw, claim_reward, compound, emergency_withdraw, set_paused, view functions |
| `src/reward.rs` | Pure reward math: `compute_reward_per_token`, `compute_earned`, `compute_new_reward_rate` |
| `src/test.rs` | Unit tests covering proportional distribution, solvency invariants, zero-stake edge cases |

## Algorithm

Reward tracking uses the standard accumulator:

```
reward_per_token += (Δt × rate × PRECISION) / total_staked
user_earned      += user_stake × (current_reward_per_token - user_paid_reward_per_token) / PRECISION
```

`REWARD_PRECISION = 1e18` eliminates precision loss for small stake weights or short block durations.

## Building

```bash
cd contracts/staking-vault
cargo build --target wasm32-unknown-unknown --release
```

## Testing

```bash
cd contracts/staking-vault
cargo test
```
