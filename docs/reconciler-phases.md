# Modular Reconciler Phases

> **TL;DR** — `apply_stellar_node` and `cleanup_stellar_node` now track their
> progress through an explicit `Phase` state machine
> (`src/controller/phase.rs`) with a defined transition table. It's purely
> observational: it never changes reconciliation behavior, only names and
> logs where in the loop a given pass currently is.

---

## Motivation

The reconciler's core functions were a single long sequence of steps,
identified only by numbered comments (`// 1. Core infrastructure`, `// 5.
Create/update the Deployment`, `// 8. Disaster Recovery reconciliation`,
...). That made it hard to answer, from a log line or a metric alone,
"where did this reconcile get to before it failed / took a long time?"
without reading the surrounding source.

`Phase` gives that implicit structure an explicit, typed, testable name.

## The state machine

```text
Initializing -> Provisioning -> Configuring -> Observing -> Reconciling -> Finalizing -> Completed
Initializing -> Deleting -> Completed
```

| Phase | Covers |
|---|---|
| `Initializing` | Spec/security validation, network-safety checks, plugin `pre_reconcile` hooks |
| `Provisioning` | Core infrastructure: PVC, ConfigMap, managed database |
| `Configuring` | Suspension handling, mTLS certs, Deployment/StatefulSet, the canary state machine |
| `Observing` | Health checks, sync-state scaling, quorum analysis, archive pruning |
| `Reconciling` | Disaster recovery, cross-cloud failover, auto-remediation |
| `Finalizing` | Final status/condition patch and metrics emission |
| `Deleting` | The finalizer cleanup path (resource has a `deletionTimestamp`) |
| `Completed` | Terminal — this reconcile pass is done |

Each call to `apply_stellar_node` (create/update path) or
`cleanup_stellar_node` (deletion path) constructs its own `PhaseTracker`.
Phase state is per-invocation — it is not persisted on the CRD status and
does not survive across reconciles; it exists purely to make one pass
through the loop legible.

## Design principle: observational, not load-bearing

`PhaseTracker::transition()` **cannot fail**. If a transition is called out
of order — a programming error, e.g. skipping straight from `Initializing`
to `Observing` — it is logged as a `tracing::warn!` and the tracker still
records the move. It is deliberately impossible for a bug in phase
bookkeeping to alter control flow, change an error path, or block a
reconcile. This was a hard requirement for retrofitting phases into a large,
already-in-production reconciler: the existing behavior of
`apply_stellar_node` / `cleanup_stellar_node` is unchanged by this work —
phase transitions are additional statements layered on top of the existing
logic, not a restructuring of it.

## Where transitions are recorded

```rust
let mut reconcile_phase = PhaseTracker::new(format!("{namespace}/{name}"));
// ...
reconcile_phase.transition(Phase::Provisioning);
// 1. Core infrastructure (PVC and ConfigMap) always managed by operator
// ...
reconcile_phase.transition(Phase::Completed);
```

Every transition emits a `tracing::debug!` span with `object`, `from`, and
`to` fields, so phase progress is visible in structured logs without any
additional instrumentation.

## Extending it

Adding a new phase means:

1. Add the variant to `Phase` in `src/controller/phase.rs`.
2. Add it to `Phase::allowed_next()` for whichever phase(s) may precede it,
   and give it its own `allowed_next()` arm.
3. Add a `reconcile_phase.transition(Phase::YourPhase)` call at the point in
   `apply_stellar_node` / `cleanup_stellar_node` where that stage begins.
4. Add a test to `src/controller/phase.rs`'s `#[cfg(test)] mod tests` and a
   row to the tables above and in `docs/architecture.md`.

## Tests

`src/controller/phase.rs` (`#[cfg(test)] mod tests`) covers the full
happy-path sequence, the deletion sequence, that `Completed` is terminal,
and that an out-of-order transition logs but does not panic or block. Run
with:

```bash
cargo test --lib controller::phase
```
