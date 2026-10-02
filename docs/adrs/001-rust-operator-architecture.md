# ADR-001: Rust and kube-rs as the Operator Foundation

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-09-30 |
| **Supersedes** | [ADR-0002 Choice of Rust Programming Language](../adr/0002-rust-language-choice.md), [ADR-0003 Use of kube-rs Finalizers](../adr/0003-kube-rs-finalizers.md) |
| **Related** | [ADR-002 CRD Versioning Strategy](002-crd-versioning-strategy.md), [ADR-0001 Wasm Admission Webhook](../adr/0001-wasm-admission-webhook.md) |
| **Deciders** | Core maintainers, DevOps leads |

## Context

Kubernetes operators are overwhelmingly written in Go. The client libraries the
Kubernetes project maintains (`client-go`, `controller-runtime`, `kubebuilder`,
`operator-sdk`) are Go-first, most published operators are Go, and most
operator contributors know Go. Choosing anything else is a departure that has
to be justified in engineering terms, not taste.

Stellar-K8s manages Stellar Core validators, Horizon API servers, and Soroban
RPC nodes. These are financial infrastructure. A validator holds a signing seed
that participates in Stellar Consensus Protocol voting; a Horizon deployment
fronts a ledger database; a Soroban RPC node executes smart-contract preflight
requests from untrusted clients. The operator is in the trust boundary of all
three. It reads and rotates secrets, terminates mTLS, parses admission requests
from the API server, runs third-party validation plugins, and decides when a
persistent volume holding ledger history is deleted.

That places three requirements on the operator process that a typical
"deploy a stateless web app" operator does not have:

1. **Memory-safety defects must be a compile-time class of bug, not a runtime
   one.** A crash in a reconciliation loop is tolerable. Memory corruption in a
   process that holds validator seeds and TLS private keys is not.
2. **Latency and memory behaviour must be predictable under load.** The
   documented scaling target is hundreds of `StellarNode` resources per
   operator instance, each triggering a multi-step reconcile that fans out to
   Deployments, StatefulSets, Services, PVCs, PodDisruptionBudgets, HPAs,
   VPAs, ServiceMonitors, NetworkPolicies and CNPG database clusters.
3. **The runtime footprint must be small.** The operator ships as several
   binaries in one distroless image and also runs as sidecars
   (`stellar-sidecar`, `stellar-watcher`, `stellar-fork-detector`) next to
   validator pods, where every megabyte of memory competes with Stellar Core.

## Decision

Stellar-K8s is implemented in **Rust**, using **kube-rs** as the Kubernetes
client and controller framework and **Tokio** as the asynchronous runtime.

The pinned stack, taken from `Cargo.lock` and the CI toolchain gate:

| Component | Version | Role |
|---|---|---|
| Rust toolchain | 1.92 minimum (CI preflight gate), 1.93 in the release `Dockerfile` | Language |
| `kube` / `kube-runtime` / `kube-core` | 0.94.2 | API client, `Controller`, watchers, finalizer helpers, CRD derive |
| `k8s-openapi` | 0.22.0, `v1_30` feature | Typed Kubernetes API objects |
| `tokio` | 1.52.1, `full` feature set | Async runtime, signals, timers, sync primitives |
| `schemars` | 0.8.22 | OpenAPI v3 schema generation for CRDs |
| `serde` / `serde_json` / `serde_yaml` | 1.x / 1.x / 0.9 | Serialisation |
| `thiserror` / `anyhow` | 1.x | Error types |
| `tracing` + OpenTelemetry | 0.1 / 0.21 | Structured logging and distributed tracing |

The rest of this document explains why, compares the alternative honestly, and
records the implementation patterns that follow from the decision so that new
contributors can recognise them in the code.

## Rationale

### 1. Memory safety as a compile-time guarantee

Rust's ownership and borrowing rules, enforced by the compiler, rule out the
following defect classes in safe code:

| Defect class | How Rust prevents it | Where it matters in Stellar-K8s |
|---|---|---|
| Null pointer dereference | There is no null. Absence is `Option<T>` and must be matched before use. | Every optional CRD field (`validator_config`, `horizon_config`, `soroban_config`, `managed_database`, …) is an `Option`. The reconciler cannot forget to check for absence; the code will not compile. |
| Buffer overflow / out-of-bounds read | Slices and `Vec` are bounds-checked; unchecked indexing requires `unsafe`. | Parsing admission review payloads, Wasm plugin bytes, history-archive JSON, and snapshot tar streams from network or object storage. |
| Use-after-free, double free, dangling references | The borrow checker rejects them statically. Lifetimes are part of the type. | Long-lived `Arc<ControllerState>` shared across reconcile tasks, background daemons, and the REST API. |
| Data races | `Send` and `Sync` are enforced by the type system. Sharing mutable state across tasks requires `Mutex`, `RwLock`, or atomics. | The leader flag, reconcile counters, and status caches are `Arc<AtomicBool>`, `Arc<AtomicU64>`, or `Arc<tokio::sync::Mutex<_>>`. The compiler rejects an unsynchronised share. |
| Uninitialised memory | Values must be initialised before use. | Struct construction for CRD types and Kubernetes objects. |

The operator crate contains **no `unsafe` blocks** (`grep -rn "unsafe " src/`
matches only a doc comment). The `deny.toml` and `cargo audit --deny unsound`
gates in `make audit` extend the same standard to dependencies.

Go is memory-safe in the garbage-collection sense (no use-after-free), but two
of the classes above remain runtime failures in Go: nil-pointer dereference
panics, and data races, which Go detects only with the race detector enabled
and only on the code paths a test happens to exercise. Both have caused
production outages in widely used Go operators. In a process that holds
validator signing material, we want those excluded by construction.

### 2. Predictable performance: garbage collection versus ownership

This is the comparison that needs the most care, because Go's garbage collector
is good and the naive "Go has GC pauses, Rust doesn't" framing overstates the
case.

**What Go's collector actually costs.** Since Go 1.8 the collector is
concurrent and stop-the-world phases are typically in the tens to low hundreds
of microseconds, not milliseconds. The real costs are elsewhere:

- **CPU.** The collector runs concurrently on up to 25% of `GOMAXPROCS`, and
  mutator goroutines are drafted into "assist" work when allocation outpaces
  collection. Under a reconcile storm, a Go operator spends a variable, hard
  to budget fraction of its CPU on collection.
- **Memory headroom.** With the default `GOGC=100`, the heap is allowed to grow
  to roughly twice the live set before a cycle starts. An operator whose live
  set is 200 MB will routinely occupy 400 MB. Kubernetes resource limits are
  set on the peak, so the container request has to include that headroom.
- **Tail latency under allocation pressure.** Assist work lands on whichever
  goroutine is allocating. In a controller that is decoding hundreds of watch
  events and building hundreds of Kubernetes objects at once, that is the
  reconcile path.
- **Non-determinism.** When a collection happens is a function of the
  allocation rate, not of program structure. Two identical reconciles can have
  different latency profiles.

**What Rust does instead.** Memory is freed deterministically when the owner
goes out of scope. There is no collector thread, no assist work, no heap
headroom multiplier, and the memory ceiling of a reconcile is a function of
what the reconcile allocates. The "zero-cost abstraction" principle means the
high-level constructs the codebase leans on (iterators, `Option`/`Result`
combinators, `async`/`await` state machines, generics) compile down to the same
code a hand-written loop would, with no runtime dispatch or allocation unless
the programmer asks for it.

**What Rust costs.** It would be dishonest to present this as free:

- Reference counting (`Arc`) is used pervasively to share state across Tokio
  tasks. Each clone is an atomic increment. This is cheap but not zero, and it
  is paid on the reconcile path.
- `async` state machines can be large. The reconcile future is boxed
  (`BoxFuture`) precisely because its state is too big and too recursive to
  live on the stack.
- The compiler cannot always prove `Send` for complex higher-ranked closures.
  `src/controller/reconciler.rs` manages the finalizer by hand instead of
  through the `kube::runtime::finalizer` helper for exactly this reason (the
  comment reads "Manual finalizer logic to avoid HRTB Send issues with the
  helper closure"). This is a real ergonomic tax.
- Compile times are long. A clean release build of this crate is measured in
  minutes, which is why the `Dockerfile` uses `cargo-chef` to cache dependency
  layers.

**Measured outcome.** The project's published comparison
([docs/scalability.md](../scalability.md#comparison-with-go-based-operators))
reports roughly 0.9 MB of resident memory per managed `StellarNode` against
2 to 3 MB for a comparable Go controller, sub-second cold start, and a p99
reconcile of about 120 ms at 100 nodes. Those figures come from the
project's own benchmark harness (`benches/`, `benchmarks/`,
`make benchmark`) and should be re-run rather than quoted when the reconcile
pipeline changes materially.

### 3. Footprint

The release image is `gcr.io/distroless/cc-debian12:nonroot` with stripped,
statically-linked Rust binaries copied in. The `Dockerfile` documents the
target as a 15 to 20 MB total image; the README quotes the operator binary at
roughly 15 MB. There is no interpreter, no runtime, no shell, and no package
manager in the image, which is both a size and an attack-surface decision. A
comparable Go operator image built on distroless is usually 30 to 50 MB
because Go binaries embed the runtime, the scheduler, and the collector.

Contributors should treat the 15 MB figure as a budget, not a constant. It
grows with every optional feature compiled in (`rest-api`, `metrics`,
`admission-webhook` with `wasmtime`, `kafka` with `rdkafka`). The feature flags
in `Cargo.toml` exist so that deployments that do not need a subsystem can
leave it out of the binary entirely.

### 4. Tokio for the reconciliation loop

kube-rs is built on Tokio, so the runtime choice follows from the framework
choice, but it also fits the workload. An operator is I/O bound: it waits on
watch streams, API round-trips, health-check HTTP calls, and timers. Tokio's
work-stealing scheduler multiplexes those waits across a small thread pool
without a thread per reconcile.

The patterns in `src/main.rs`, `src/commands/operator.rs`, and
`src/controller/reconciler.rs` are the ones contributors will encounter:

- **One runtime, entered once.** `#[tokio::main]` on `main`. Everything else
  is a task on that runtime.
- **Leader election gates the controller.** `main` starts a Lease-based
  election (`src/controller/leader.rs`, lease `stellar-operator-leader`,
  15-second duration, 10-second renewal) and awaits `wait_until_leader()`
  before starting the operator. `tokio::select!` races the operator future
  against `wait_until_lost()`; losing the lease ends the process so a replica
  can take over.
- **The controller is a stream.** `Controller::new(stellar_nodes, Config::default())`
  is extended with `.owns::<Deployment>()`, `.owns::<StatefulSet>()`,
  `.owns::<Service>()`, `.owns::<PersistentVolumeClaim>()`,
  `.owns::<PodDisruptionBudget>()` and `.watches::<Secret>()`, then
  `.shutdown_on_signal().run(reconcile, error_policy, state)`. Changes to any
  owned child re-enqueue the parent `StellarNode`. The resulting stream is
  folded into a `BatchSummaryReport` that logs a summary every 50 results.
- **Reconcile is a boxed future that returns an `Action`.** A pass ends with
  `Action::requeue(duration)` or `Action::await_change()`. In steady state a
  `Ready` node is requeued at `operator_config.reconciler.requeue_interval`;
  a node in any other phase is requeued at a quarter of that. A non-leader
  replica returns `Action::requeue(5s)` without doing work.
- **Errors map to a retry budget, not a crash.** `error_policy` inspects
  `Error::is_retriable()` (true for `KubeError`, `FinalizerError`,
  `RemediationError`) and requeues after the retriable budget (15 s default)
  or the non-retriable budget (60 s default). Both are CLI flags.
- **Long-running work is a spawned task, not part of reconcile.** Peer
  discovery, the feature-flag ConfigMap watcher, the audit worker, the
  quorum optimiser, the database compaction daemon, the key-rotation daemon,
  and the metrics heartbeat are each `tokio::spawn`ed once from
  `run_controller` or `run_operator`, and communicate with reconcile through
  `Arc<ControllerState>`.
- **Shutdown is cooperative.** `wait_for_shutdown_signal()` listens for
  `SIGTERM` and `SIGINT` via `tokio::signal`, flips the leader flag, releases
  the Lease, and flushes telemetry before exit.
- **Each pass has an explicit phase machine.** `src/controller/phases.rs`
  names the stages (`Initializing`, `Validating`, `Finalizing`,
  `Provisioning`, `Deploying`, `Scaling`, `Observing`, `Remediating`,
  `Publishing`, `Succeeded`, `Failed`) and validates transitions. An illegal
  transition is logged, never fatal. See [docs/reconciler-phases.md](../reconciler-phases.md).

### 5. Typed CRDs from Rust structs

`kube-derive`'s `#[derive(CustomResource)]` plus `schemars::JsonSchema` make
the Rust struct the single source of truth for the CRD. The
`StellarNodeSpec` struct in `src/crd/stellar_node.rs` generates the OpenAPI
schema, the printer columns, the short name, and the status subresource
declaration. The `crdgen` binary emits the YAML; CI fails if the committed
YAML drifts from the struct. A misspelled field or wrong type is a compile
error in the reconciler, not a runtime deserialisation failure discovered in a
cluster. [ADR-002](002-crd-versioning-strategy.md) covers how that schema is
allowed to change.

### 6. Resource cleanup: owner references plus a finalizer

Kubernetes offers two cleanup mechanisms and the operator uses both, for
different jobs.

**Owner references for garbage collection.** Every child object the operator
creates carries an `OwnerReference` to its `StellarNode` with
`controller: true` and `blockOwnerDeletion: true`
(`src/controller/resources.rs`, `owner_reference`). If the operator is down
when a `StellarNode` is deleted, the Kubernetes garbage collector still removes
the in-namespace children. This is the safety net.

**A finalizer for ordered, policy-aware cleanup.** The safety net is not
enough on its own, for two reasons. First, some children must be removed in a
specific order (the CNPG database cluster before the workload that depends on
it, the HPA before the Deployment it scales). Second, the most important
child, the PersistentVolumeClaim holding ledger history, must be deleted or
retained according to `spec.storage.retentionPolicy`, and the garbage collector
knows nothing about that policy.

So the reconciler adds the finalizer `stellarnode.stellar.org/finalizer`
(`src/controller/finalizers.rs`) on first sight of a live `StellarNode`. When
`metadata.deletionTimestamp` is set, the pass enters the `Finalizing` phase and
runs `cleanup_stellar_node`, which deletes in this order:

1. Managed database resources (CNPG `Cluster` and `Pooler`)
2. Alerting rules
3. VerticalPodAutoscaler
4. HorizontalPodAutoscaler
5. ServiceMonitor
6. Ingress
7. NetworkPolicy
8. MetalLB LoadBalancer Service and address configuration
9. Service mesh resources (Istio or Linkerd)
10. PodDisruptionBudget
11. Service
12. Workload (Deployment or StatefulSet)
13. ConfigMap
14. PersistentVolumeClaim, **only if** `retentionPolicy` is `Delete`. With
    `Retain` (the non-default), the PVC is left in place and the log says so.

Each step logs and continues on failure so one missing child cannot wedge the
others. Only when the whole sequence returns `Ok` does the reconciler patch the
finalizer off the object and return `Action::await_change()`; the API server
then removes the `StellarNode`. If cleanup returns an error, the finalizer
stays, `error_policy` requeues, and the object remains `Terminating` until a
later pass succeeds. Every step honours the operator's `--dry-run` flag, and a
`FinalizerCleanupStarted` Kubernetes event is emitted so operators can see
cleanup begin. `src/controller/orphan_audit.rs` exists to find anything the
sequence missed.

The finalizer is added and removed with explicit merge patches rather than the
`kube::runtime::finalizer` helper (see the `Send` note above), but the
project's `Error` type still implements `From<kube::runtime::finalizer::Error>`
so the helper can be adopted for other controllers where the closure shape
allows it.

## Alternatives considered

### Go with controller-runtime / kubebuilder

The default choice, and the one that would have been easiest to staff.

- **For:** first-party client libraries; `controller-runtime` handles caches,
  informers, and leader election out of the box; kubebuilder and operator-sdk
  scaffold CRDs, RBAC, webhooks, and OLM bundles; the largest pool of operator
  contributors; fast compile times.
- **Against:** nil dereferences and data races are runtime failures; the
  collector imposes a memory headroom multiplier and variable CPU cost that
  matter when running as a sidecar next to Stellar Core; larger images;
  `interface{}`-heavy unstructured handling in places where we want static
  types.
- **Why rejected:** the safety and predictability requirements in the Context
  section were weighted above staffing ease. The Go ecosystem advantages are
  real and are the main ongoing cost of this decision (see Consequences).

### Java / Kotlin with the Fabric8 or Java Operator SDK

Mature, well-supported in enterprises that already run the JVM.

- **Against:** JVM warm-up and memory floor are incompatible with the sidecar
  use case; images are several hundred megabytes; garbage collection
  characteristics are a larger version of the Go concern.

### Python with kopf

Fast to write.

- **Against:** the GIL serialises CPU work; dynamic typing gives up the
  compile-time CRD guarantees; the interpreter and dependencies dominate the
  image; performance at hundreds of resources is a known problem.

### Rust without kube-rs (raw HTTP client)

Considered briefly.

- **Against:** reimplementing watch bookmarks, reflector caches, backoff, and
  the controller queue is exactly the undifferentiated work kube-rs exists to
  do. kube-rs is a CNCF sandbox project with a stable release cadence and a
  responsive maintainer group.

## Consequences

### Positive

- Whole classes of memory-safety and concurrency defects are excluded at
  compile time in a process that handles validator seeds and TLS keys.
- Memory usage is a function of live data, not of the allocator's heuristics.
  Resource requests can be set close to actual usage.
- One small distroless image with no runtime dependencies; the same binary
  serves as operator, CLI plugin, and sidecar.
- CRDs, API reference documentation, JSON schemas, and shell completions are
  all generated from the Rust types, so drift is caught in CI rather than in
  a cluster.
- `Result`-based error handling with a central `Error` enum
  (`src/error.rs`, codes `SK8S-001` onwards) forces every failure path to be
  handled or explicitly propagated, and maps cleanly to CLI exit codes and
  retry policy.
- The `reconciler-fuzz` feature, property tests, and the formal-verification
  directory are practical because the reconcile function is a pure
  `async fn(Arc<StellarNode>, Arc<ControllerState>) -> Result<Action>`.

### Negative

- **Contributor pool.** Fewer engineers have both Rust and Kubernetes operator
  experience. Onboarding is slower. This ADR and the module-level docs exist to
  offset that.
- **Ecosystem gap.** There is no kube-rs equivalent of kubebuilder scaffolding
  or operator-sdk's Go plugins. OLM bundle generation is done with
  `operator-sdk generate` against Helm-rendered manifests rather than natively.
  Conversion webhooks, if ever needed, are hand-written (see ADR-002).
- **Compile times and binary bloat with many features.** Mitigated by
  `cargo-chef` layer caching and feature flags, but not eliminated.
- **`async` ergonomics.** Higher-ranked trait bound and `Send` inference
  limitations occasionally force boxing or manual code where a Go closure
  would just work. The manual finalizer patching is the visible instance.
- **CRD generation edge cases.** `kube-core`'s schema hoisting has a known
  panic on certain enum shapes; the CI `crd-drift` job tolerates a `crdgen`
  failure and falls back to the committed YAML as the source of truth. This is
  tracked and is a cost of relying on a smaller ecosystem.

### Neutral

- Go's collector improvements do not change this decision, because the
  primary driver was safety, not the millisecond-scale pause difference.
- kube-rs tracks Kubernetes releases closely; the `k8s-openapi` version
  feature (`v1_30`) and the compatibility matrix test
  (`tests/compat_matrix.rs`, Kubernetes 1.27 to 1.30) must be bumped together.

## Compliance and review

- New code must not introduce `unsafe`. If a dependency requires it, the
  justification goes in the PR and `deny.toml` is updated.
- New long-running work must be a spawned task with its own error logging,
  never an unbounded loop inside `reconcile`.
- Any new child resource type must set the owner reference and be added to
  `cleanup_stellar_node` in the correct position in the order above.
- Reconcile must remain idempotent: every step is a server-side apply or
  merge patch that can be repeated safely, because the controller will
  repeat it.

## References

- `src/main.rs`, `src/commands/operator.rs`, `src/controller/reconciler.rs`,
  `src/controller/finalizers.rs`, `src/controller/phases.rs`,
  `src/controller/leader.rs`, `src/controller/resources.rs`, `src/error.rs`
- [docs/reconciler-phases.md](../reconciler-phases.md),
  [docs/leader-election.md](../leader-election.md),
  [docs/scalability.md](../scalability.md),
  [docs/docker-build-optimization.md](../docker-build-optimization.md)
- kube-rs: <https://kube.rs/> and <https://docs.rs/kube/0.94.2/>
- Tokio: <https://tokio.rs/>
- Kubernetes finalizers: <https://kubernetes.io/docs/concepts/overview/working-with-objects/finalizers/>
- Kubernetes garbage collection and owner references: <https://kubernetes.io/docs/concepts/architecture/garbage-collection/>
- Go garbage collector guide (for the comparison above): <https://go.dev/doc/gc-guide>
