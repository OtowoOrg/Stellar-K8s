# Reconciler Phases

The reconciler walks a fixed pipeline on every pass. Before issue #1047 that
pipeline was implicit — encoded only in statement order inside a ~2,000-line
`apply_stellar_node`, with numbered comments as the only signpost. Nothing
declared which stage was running, no log line named it, and a step
accidentally moved across a boundary produced no signal at all.

`src/controller/phases.rs` makes the pipeline explicit: a phase enum, an
authoritative transition table, and a per-pass state machine that records
every move with a timestamp, a reason, and how long the previous phase took.

## Phases

| Phase | What runs in it | Lifecycle phase |
|---|---|---|
| `Initializing` | Leader check, context and configuration resolution | `Pending` |
| `Validating` | Spec validation, security policy enforcement, network safety | `Pending` |
| `Finalizing` | Deletion path: finalizer-driven cleanup of owned resources | `Terminating` |
| `Provisioning` | PVCs, ConfigMaps, secrets, mTLS material | `Creating` |
| `Deploying` | Deployment/StatefulSet, Service, Ingress, PDB | `Creating` |
| `Scaling` | HPA/VPA, replica and disk scaling | `Creating` |
| `Observing` | Health checks, sync state, archive integrity | `Syncing` |
| `Remediating` | Automatic recovery from a detected failure | `Remediating` |
| `Publishing` | Status subresource and Kubernetes event publication | `Running` |
| `Succeeded` | Terminal: the pass completed | `Running` |
| `Failed` | Terminal: the pass aborted | `Failed` |

The rightmost column is `ReconcilePhase::lifecycle_phase()`, which maps onto
the existing `StellarNodeStatus.phase` vocabulary. The two are **not** the
same concept:

- `StellarNodeStatus.phase` (deprecated) describes the **resource** as an
  observer sees it.
- `ReconcilePhase` describes the **reconciler's own pipeline** during a single
  pass.

## Transition table

```
Initializing ─┬─→ Validating ─┬─→ Provisioning ──→ Deploying ─┬─→ Scaling ──→ Observing ─┐
              │               │                               │                ↑         │
              │               └─→ Publishing ←────────────────┼────────────────┼─────────┤
              │                        │                      └─→ Observing ───┘         │
              ├─→ Finalizing ──→ Succeeded                     Observing ⇄ Remediating ───┘
              │                        ↑
              └─→ Succeeded            └── Publishing ──→ Succeeded

  Any non-terminal phase ──→ Failed
```

Rules worth knowing:

- **Every non-terminal phase can reach `Failed`.** An error can surface
  anywhere, so no error path needs a special case.
- **Terminal phases accept no transitions.** `Succeeded` and `Failed` end the
  pass.
- **`Scaling` is skippable, not reorderable.** `Deploying → Observing` is legal
  (autoscaling not configured); `Deploying → Provisioning` never is.
- **`Remediating` loops back to `Observing`** so recovery can be confirmed
  before the status is published.
- **A validation failure can still publish.** `Validating → Publishing` exists
  so a rejected spec still gets its status written.

`ReconcilePhase::can_transition_to` is the single source of truth. Reordering
the pipeline means editing that function, which is what makes an accidental
reorder visible in review.

## Reading the trail in logs

Every pass emits one summary line:

```
INFO reconcile phases: initializing → validating → provisioning → deploying →
     scaling → observing → publishing → succeeded (total 812ms)
     node=validator-testnet namespace=stellar phase=succeeded
```

Individual transitions are logged at `DEBUG`:

```
DEBUG reconcile phase transition from=deploying to=scaling elapsed_ms=214
      reason="workload applied; reconciling elasticity"
```

To see the trail for a single node:

```bash
kubectl logs -n stellar-system deploy/stellar-operator \
  | grep "reconcile phases" | grep validator-testnet
```

## Failure behaviour

Phase bookkeeping is **observability, not control flow**. An illegal
transition is logged as a warning and reconciliation continues exactly as it
did before:

```rust
fn advance_phase(phases: &Arc<Mutex<PhaseMachine>>, to: ReconcilePhase, reason: &str) {
    // ... transition_to() errors are logged, never propagated
}
```

This is deliberate. A mistake in the transition table must not take the
operator down. The table is enforced strictly in tests instead, where a
regression is cheap to catch. `Error::PhaseTransitionError` (`SK8S-023`)
exists for callers that *do* want to treat an illegal move as fatal — such as
`PhaseMachine::run`, which refuses to execute a step it cannot legally enter.

## Using the machine

```rust
use stellar_k8s::controller::phases::{PhaseMachine, ReconcilePhase};

let mut machine = PhaseMachine::new();
machine.transition_to(ReconcilePhase::Validating, "spec validation")?;

// Illegal moves are typed errors that name the legal alternatives.
assert!(machine.transition_to(ReconcilePhase::Publishing, "skip").is_err());

// Run a step inside a phase; a failing body marks the pass Failed.
let replicas = machine
    .run(ReconcilePhase::Deploying, "rolling out workload", || async {
        ensure_deployment(&client, &node).await
    })
    .await?;

println!("{}", machine.summary());
// initializing → validating → deploying (total 412ms)
```

Useful accessors:

| Method | Purpose |
|---|---|
| `current()` | The phase the reconciler is in now |
| `history()` | Every transition, oldest first |
| `summary()` | One-line trace of the whole pass |
| `phase_durations()` | Time spent in each completed phase |
| `elapsed_in_phase_ms()` | Time in the current phase so far |
| `is_terminal()` | Whether the pass has ended |

## Verification

```bash
# The state machine's own tests: transition table, reachability, machine behaviour
K8S_OPENAPI_ENABLED_VERSION=1.30 cargo test --lib controller::phases

# The documented examples in this page and the module docs
K8S_OPENAPI_ENABLED_VERSION=1.30 cargo test --doc controller::phases
```

The suite asserts structural properties, not just individual edges: every
phase is reachable from `Initializing`, no phase self-loops, every
non-terminal phase has a forward edge (no dead ends) and can reach `Failed`,
and terminal phases accept nothing. Adding a phase without wiring it into the
table fails those tests immediately.
