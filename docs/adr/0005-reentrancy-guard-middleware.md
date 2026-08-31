# ADR 0005: Native Reentrancy Guard Middleware for Soroban

## Status

Accepted

## Context

Soroban contracts interact through cross-contract invocations
(`env.invoke_contract`). These calls share a single transaction and, until the
transaction commits, a callee's view of the caller's storage is the caller's
*latest uncommitted* state. A malicious or buggy target contract can exploit this
by **re-entering** a mutating function on a caller before the caller's accounting
is settled, mutating the same state variables a second time against a stale
snapshot.

This is the classic reentrancy vulnerability, and Stellar-K8s operates
high-value `StellarNode` deployments where an exploited smart contract can move
user funds. We need a way to globally enforce execution safety on those
deployments, driven through the operator's existing Wasm custom-validation
layer so that operators can opt-in per namespace or contract without rebuilding
the operator.

Rather than demanding every contract author hand-roll a guard, we provide a
**reusable sub-contract middleware** that wraps a mutating function and reverts
the unsafe nested invocation, exactly as OpenZeppelin's `ReentrancyGuard` does
for EVM, but adapted to Soroban's explicit cross-contract execution model.

## Decision

Implement a native reentrancy-guard middleware under
`wasm-plugins/security/reentrancy/` and expose its configuration through
`ConfigMap`s the operator admission webhook already consumes.

### Key modules

- `wasm-plugins/security/reentrancy/` — the middleware crate
  (`stellar-soroban-reentrancy-guard`).
- The operator admission webhook integration, which passes a
  [`ReentrancyGuardConfig`] loaded from a `ConfigMap` data key
  (e.g. `reentrancy-guard.json`) into the middleware before a guarded invocation.

### Locking model (OpenZeppelin-flavoured, Soroban-adapted)

State variables are addressed by a `SlotId` (a strongly-typed 32-byte
identifier). The guard keeps a **write-lock stack**:

- `enter(slot, Write)` pushes the slot and returns normally only if it is not
  already present anywhere on the current cross-contract call stack. If it is
  already present — i.e. an ancestor invocation is currently mutating the same
  state variable — the guard returns `GuardError::ReentrancyDetected` and the
  middleware **reverts** the nested invocation.
- `enter(slot, Read)` never locks and never fails. Legitimate, non-mutating read
  callbacks therefore produce **zero false positives**, even when an ancestor
  write is in flight on the same slot.
- `exit(slot, Write)` pops the matching lock once the invocation completes, so
  sequential (non-nested) mutation of the same variable remains allowed.

A plain boolean "entered/not-entered" flag is deliberately **not** used: a read
callback is a legitimate re-entry, and different state variables may safely be
mutated concurrently by nested calls. Only a nested mutation of the *same* state
variable is unsafe, and only that is reverted.

### Storage-agnostic core

The state machine is expressed against a minimal `GuardStorage` trait so it can
be:

- unit- and integration-tested exhaustively on stable Rust with no SDK
  dependency, and
- bound to Soroban host instance storage through the optional `soroban` feature
  (compiled as a `no_std` alloc Wasm guest), or to the operator's admission
  input.

The stack is persisted as `u32 length || slot0 (32) || slot1 (32) || ...` under a
single fixed storage key. A malformed stack is treated as corruption
(`GuardError::CorruptedState`) so an attacker cannot launder state by tampering
with the stack encoding.

### Instruction budget

Every guarded invocation performs a constant number of operations — one
instance-storage read, a linear stack scan bounded by `MAX_DEPTH` (8), and one
instance-storage write — keeping the overhead **well below 500 Wasm
instructions** as required.

## Consequences

### Positive

1. **Zero false positives on reads**: non-mutating read callbacks are always
   allowed, satisfying the issue's central constraint.
2. **Selective enforcement**: ConfigMap-driven scoping lets operators guard only
   high-value namespaces/contracts, or explicitly opt out, without redeploys.
3. **Reusable**: any Soroban sub-contract can wrap its mutating entry points with
   the same middleware instead of maintaining bespoke guard code.
4. **Stronger than a boolean lock**: allows re-entrancy for reads and distinct
   slots while still reverting the only unsafe pattern.
5. **Testable & auditable**: the storage-agnostic core is fully unit-tested, and
   a deliberately vulnerable mock contract proves both that the attack drains
   funds *without* the guard and that the guard *blocks* it.
6. **Within budget**: bounded stack scan keeps overhead < 500 instructions.

### Negative

1. **Integration effort**: existing contracts must be adapted to enter/exit the
   guard around mutating functions (or be rewritten with the middleware in mind).
2. **Two code paths**: a single function must be both serializable for operator
   config and `no_std` for the Soroban guest, adding a small build-feature split.
3. **Not a substitute for audit**: the guard mitigates reentrancy specifically;
   it does not address other contract vulnerabilities.

### Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Corrupted/attacker-tampered stack | Length + structure validated on decode; corruption reverts |
| Lock leaks deny service after a revert | Guard releases locks on the failure path; tests assert reusability |
| Stack scan grows unbounded | `MAX_DEPTH = 8` bounds the scan and the instruction budget |
| False positives on legitimate nested writes | Only the same slot is reverted; distinct slots are never rejected |
| Read callbacks flagged as reentry | Read access never locks or fails (zero false positives) |

## Alternatives Considered

### 1. Single global boolean flag (classic OpenZeppelin)

**Pros**: simplest to implement. **Cons**: incorrectly blocks legitimate
cross-contract *read* callbacks (a false positive on the issue's key scenario)
and provides no way to track depth or distinguish slots. **Verdict**: rejected —
too coarse for Soroban's call model.

### 2. Full per-call "check-effects-interactions" rewrite of every contract

**Pros**: idiomatic, no middleware. **Cons**: high developer burden, easy to get
wrong, not enforceable globally through the operator. **Verdict**: rejected as
the sole mechanism; the middleware makes enforcement consistent.

### 3. Rely solely on Soroban's single-transaction atomicity

**Pros**: no new code. **Cons**: atomicity does not prevent a nested invocation
from observing and mutating the caller's uncommitted state within the same
transaction — precisely the attack. **Verdict**: rejected, reentrancy is a real
bounded-deferral bug atomicity does not resolve.

## Implementation Notes

```bash
# Build the Soroban Wasm middleware (no_std alloc guest)
cargo build --manifest-path wasm-plugins/security/reentrancy/Cargo.toml \
    --features soroban --target wasm32-unknown-unknown --release

# Run the security test suite (default std build)
cargo test --manifest-path wasm-plugins/security/reentrancy/Cargo.toml
```

See [`wasm-plugins/security/reentrancy/README.md`](../../wasm-plugins/security/reentrancy/README.md)
for the full walkthrough and
[`config/reentrancy-guard-configmap.yaml`](../../config/reentrancy-guard-configmap.yaml)
for a deployable `ConfigMap` example.

## References

- [Stellar Soroban docs](https://developers.stellar.org/docs/smart-contracts)
- [OpenZeppelin ReentrancyGuard](https://docs.openzeppelin.com/contracts/4.x/api/security#ReentrancyGuard)
- Wasm admission webhook: [ADR 0001](0001-wasm-admission-webhook.md) and
  [docs/wasm-webhook.md](../wasm-webhook.md)

## Decision Makers

- Sulamoney222 (Contributor)

## Date

2026-08-31
