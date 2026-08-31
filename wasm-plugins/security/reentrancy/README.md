# Reentrancy Guard Sub-Contract Middleware for Soroban

A native reentrancy guard that can be enforced through the Stellar-K8s custom
validation (Wasm) layer. It tracks cross-contract call stacks during transaction
execution and **reverts nested, mutating invocations that target the same state
variable**, while never false-positiving legitimate non-mutating read callbacks.

This crate lives at `wasm-plugins/security/reentrancy/` and is intentionally a
standalone (non-workspace) Rust crate so it can be compiled to the
`wasm32-unknown-unknown` target and built/tested in isolation — matching the
pattern used by the other Wasm plugins under `examples/plugins/`.

Design reference: OpenZeppelin's `ReentrancyGuard`, adapted to Soroban's
explicit cross-contract execution model (see [ADR 0005](../../../../docs/adr/0005-reentrancy-guard-middleware.md)).

## Why a guard is needed

Soroban contracts call each other via cross-contract invocations
(`env.invoke_contract`). A malicious or buggy target can, before settling its own
accounting, **re-enter** a caller's mutating function and mutate the same state a
second time. Because the caller's state is not yet committed, the nested call
observes stale state and can drain funds or corrupt invariants.

## How it works

The middleware maintains a **write-lock stack** keyed by a state-variable
[`SlotId`](src/guard.rs):

- `enter(slot, Write)` pushes the slot and succeeds **only if** it is not already
  on the current stack. A sparse re-entry of the same slot is detected and
  reverted (`GuardError::ReentrancyDetected`).
- `enter(slot, Read)` never locks and never fails → **zero false positives** on
  legitimate read callbacks, even while an ancestor write is in flight.
- `exit(slot, Write)` pops the matching lock, so sequential (non-nested)
  mutation of the same variable remains fully allowed.

This is strictly stronger than a single boolean "entered" flag: it permits
re-entrancy for reads and for *different* state variables while still reverting
the only actually-unsafe pattern — a nested mutation of the same variable.

The state machine is **storage agnostic** ([`GuardStorage`](src/guard.rs)):

| Backing store | Where |
|---|---|
| Soroban host instance storage | [`host`](src/host.rs) (`soroban` feature) |
| In-memory store (tests/demo) | [`mem`](src/mem.rs) |
| Operator admission-webhook input | [`config`](src/config.rs) |

## Scope & instruction budget

- A single instance-storage read/write plus a linear stack scan bounded by
  [`MAX_DEPTH`](src/guard.rs) = 8 → **overhead < 500 Wasm instructions**.
- ConfigMap-driven scoping (per namespace or contract ID) via
  [`ReentrancyGuardConfig`](src/config.rs) — the operator can selectively enable
  the guard without rebuilding or redeploying.

## Build & test

Default (stable `std`, zero external deps beyond `serde`/`serde_json`):

```bash
cargo test --manifest-path wasm-plugins/security/reentrancy/Cargo.toml
cargo clippy --manifest-path wasm-plugins/security/reentrancy/Cargo.toml --all-targets
```

Soroban `no_std` Wasm guest (requires `rustup target add wasm32-unknown-unknown`):

```bash
cargo build --manifest-path wasm-plugins/security/reentrancy/Cargo.toml \
    --features soroban --target wasm32-unknown-unknown --release
```

## Validation

`tests/security.rs` proves the *Definition of Done*:

1. **`unguarded_mock_contract_is_exploitable`** — the deliberately vulnerable
   mock vault (`vuln.rs`) pays out `200` from a `100` deposit and leaves the
   ledger corrupted.
2. **`guarded_mock_contract_blocks_reentrancy`** — same attack wrapped by the
   middleware reverts with `GuardError::ReentrancyDetected`, moves nothing, and
   remains usable afterwards (no lock leak / DoS).
3. **`read_callback_is_never_a_false_positive`** — non-mutating reads produce
   zero false positives.
4. **`cross_contract_stack_tracks_multiple_distinct_slots`** and
   **`distinct_state_variables_do_not_interfere`** — legitimate nested writes to
   distinct slots are never rejected.
5. **`configmap_selects_scope_without_false_positives`** — per-namespace /
   contract-ID scoping behaves correctly.

## Configuration

See [`config/reentrancy-guard-configmap.yaml`](../../../../config/reentrancy-guard-configmap.yaml)
for a deployable `ConfigMap` example.

## License

Apache 2.0
