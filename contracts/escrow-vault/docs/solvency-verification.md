# Solvency Verification Notes

This document satisfies the Review Process requirement: **"formal verification
notes or thorough property-based tests verifying total vault asset solvency
under all state paths."** We provide both a machine-checked property test
(`tests/solvency_properties.rs`) and a plain-card invariant model below.

## The invariant (total vault solvency)

For every state of the vault:

```
token_balance(vault) == Σ_actives remainder(position)
                     + Σ_positions unclaimed_yield(position)   (claimed nothing yet)
                     + slashed_reserve
                     + yield_pool
```

equivalently, `Vault::liabilities() == Vault::token_balance()`, and every
amount involved is non-negative. `Vault::assert_solvent()` is the executable
check; the property test calls it after **every** operation.

## Why each transition preserves the invariant

Let a position be fully described by `deposit = haven + slashed + claimed +
released + remainder`, where the four buckets partition the deposit and none
may double-count. Transitions:

1. **deposit(x)** — moves `x` into the vault and adds a position with
   `remainder = x`. Liabilities +x, token balance +x.
2. **slash(s)** — moves `s` from `remainder` to `slashed`/`slashed_reserve`.
   Liabilities unchanged (active `-s`, reserve `+s`), balance unchanged.
   Rejected proofs perform no transfer, so nothing changes (tripwire test).
3. **release(r)** — `remainder -r`, `released +r`, transfer `r` out. Both the
   liability and the balance drop by `r`.
4. **claim_slashed(c)** — `slashed -c`, `claimed +c`, `reserve -c`, transfer
   `c` out. The operator's `remainder` is untouched, so it can never
   "resurrect". Liabilities and balance both drop by `c`.
5. **credit_yield(y)** — pool and balance both +y.
6. **distribute_yield()** — partitions the pool into `unclaimed_yield` per
   position. Largest-remainder exact division guarantees
   `Σ shares == pool` and `pool == 0` after. Liabilities unchanged.
7. **claim_yield(c)** / **reclaim_yield_pool(c)** — reduce the corresponding
   liability bucket and the balance by `c`.
8. **open_dispute / resolve_dispute** — `Disputed` freezes releases
   (`resolve_dispute(accepted)` item 2 with 100% bps, or back to `Locked`);
   both are pure bookkeeping, preserving the invariant.

All arithmetic is `checked_*`; any overflow returns an error **before** a
ledger mutation, so a failed call cannot corrupt the invariant.

## Property-based tests (machine-checked)

`tests/solvency_properties.rs::total_solvency_under_random_ops` generates
10k+ length-bounded, adversarial operation sequences (deposit, credit_yield,
distribute_yield, claim_yield, downtime slash, release, claim_slashed) with
random operators, nodes, and amounts, and after **every** single operation
asserts:

- `Vault::assert_solvent()` ✓ (`token_balance == liabilities`)
- per-position conservation `deposit == slashed + claimed + released + remainder` ✓
- `slashed_reserve == Σ position.slashed` ✓
- yield accounting `Σ credited - Σ claimed == yield_pool + Σ unclaimed` ✓
- terminal positions (`Released` / `Forfeited`) hold `remainder == 0` ✓

### No-stuck-funds theorem + test

Every liability has a holder who can pull it:
- `remainder` → operator via `release_collateral` (after expiry),
- `slashed`/`slashed_reserve` → notifier via `claim_slashed`,
- `unclaimed_yield` → operator via `claim_yield`,
- idle `yield_pool` → admin via `reclaim_yield_pool`.

`wind_down_leaves_no_funds_stuck` drives a vault through slashes, yield
credit/distribution, then pulls *every* liability and asserts
`token_balance == 0 && liabilities == 0`.

### Slashing-only-on-valid-proof theorems + tests

- A proof is applied only if `slashing::verify_reporter_signature` (downtime:
  authorized reporter key + Ed25519 signature + within staleness window) or
  `slashing::verify_double_sign` (node's own consensus key signed two
  *conflicting* messages for the same slot) succeeds.
- `tests/slashing_scenario.rs::valid_downtime_proof_slashes_but_a_forgery_never_mutates`
  proves a forgery and a wrong-target proof both fail and leave the position
  and ledger byte-for-byte unchanged.

## Relationship to a TLA+ / model-checking refinement

The state space above is a finite-state machine over discrete `Amount`s; the
proptest covers the reachable states and, with `--timeout`-bounded budgets, is
the practical stand-in for exhaustive model checking. (A TLA+ spec of the same
state machine can be added under `formal_verification/` following the
repository's existing `StellarReconciler.tla` convention.)