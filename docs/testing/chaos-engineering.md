# Chaos Mesh Chaos Engineering Strategy

Issue #295. Standardized methodology for proving that `stellar-operator`
survives deliberate infrastructure faults — network partitions, pod failures,
disk I/O errors — injected with [Chaos Mesh](https://chaos-mesh.org) into a
staging Stellar cluster.

## Overview

Stellar nodes are stateful, quorum-dependent workloads; the operator that
manages them must converge `StellarNode` state back to healthy after any
single (or combined) infrastructure fault, without human intervention.
Chaos engineering is how we prove that claim.

This repo verifies operator resilience in three complementary layers:

| Layer | Question it answers | Tooling | Where |
|-------|--------------------|---------|-------|
| Deterministic unit/property tests | "Are the reconcile state-machine transitions correct?" | `cargo test` | `tests/` |
| Reconciler fuzzing | "Can any input sequence drive the reconciler into an illegal state?" | [`cargo fuzz` / property tests](../fuzzing.md) | `docs/fuzzing.md` |
| Chaos Mesh experiments | "Does the *real cluster* recover after deliberate infrastructure faults?" | Chaos Mesh CRs | [`tests/chaos/`](../../tests/chaos/README.md) + this document |

Monthly disaster-recovery *drills* (manual, runbook-style) are scheduled in
[`docs/chaos-drills.md`](../chaos-drills.md). This document covers the
automated, Kubernetes-native side: Chaos Mesh controller integration,
reusable experiment manifests, and the recovery behaviors the operator is
expected to exhibit during active injection.

## Architecture: Chaos Mesh on a staging cluster

```
┌───────────────────────────────────────────────────────────────┐
│ staging kind / k8s cluster                                    │
│                                                               │
│  ┌───────────────┐   injects into (RBAC-bounded)   ┌────────┐ │
│  │  chaos-mesh   │ ───────────────────────────────▶│ stellar│ │
│  │  (controller, │                                 │ system │ │
│  │  daemon, fq)  │   experiments live in           └────────┘ │
│  └───────────────┘   ┌─────────────────┐         operator pods │
│                      │ chaos-testing   │         validator /   │
│                      │ (NetworkChaos,  │         horizon /     │
│                      │  PodChaos, ...) │         soroban-rpc   │
│                      └─────────────────┘         StellarNodes  │
└───────────────────────────────────────────────────────────────┘
```

The reference installation is exactly what CI runs nightly in
[`chaos-tests.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/chaos-tests.yml):

```bash
# 1. Chaos Mesh control plane (pinned chart version, containerd runtime)
helm repo add chaos-mesh https://charts.chaos-mesh.org --force-update
helm repo update
kubectl create namespace chaos-mesh
helm install chaos-mesh chaos-mesh/chaos-mesh \
  --namespace chaos-mesh \
  --version 2.6.3 \
  --set chaosDaemon.runtime=containerd \
  --set chaosDaemon.socketPath=/run/containerd/containerd.sock \
  --wait --timeout=5m

# 2. Experiment namespace (all Chaos Mesh CRs live here, nowhere else)
kubectl create namespace chaos-testing \
  --dry-run=client -o yaml | kubectl apply -f -

# 3. Grant the chaos controller authority over the workload namespace ONLY
kubectl apply -f - <<'EOF'
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: chaos-mesh-stellar-system
  namespace: stellar-system
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: chaos-mesh-chaos-controller-manager-target-namespace
subjects:
  - kind: ServiceAccount
    name: chaos-controller-manager
    namespace: chaos-mesh
EOF
```

Pod selectors used by every experiment target the labels the operator itself
puts on workload pods (see
[`src/controller/resources.rs`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)):

| Selector | Matches |
|----------|---------|
| `app=stellar-operator` | the operator pod |
| `app.kubernetes.io/name=stellar-node` | all managed node pods |
| `app.kubernetes.io/component=<validator\|horizon\|sorobanrpc>` | pods of one node type |
| `app.kubernetes.io/instance=<StellarNode name>` | pods of one `StellarNode` |
| `stellar.org/node-type=<Validator\|Horizon\|SorobanRpc>` | typed alias of component |

## Safety: namespace isolation constraints

These are **hard requirements** for every experiment in this repo — chaos that
cannot be namespace-isolated must not be run on a shared cluster:

1. **Experiment CRs are only ever created in `chaos-testing`.**
   The Chaos Mesh controller executes them; no human applies chaos into a
   workload namespace.
2. **Every `spec.selector` pins `namespaces:` to the intended victims**
   (`stellar-system` for node/operator pods). A selector without an explicit
   namespace list is rejected in review.
3. **RBAC is scoped per target namespace** via the
   `chaos-mesh-chaos-controller-manager-target-namespace` RoleBinding (step 3
   above). Grant bindings only for namespaces under chaos testing.
4. **Every experiment sets `duration:`** — chaos self-destructs even if the
   cleanup step is skipped. The CI runner additionally `kubectl delete`s each
   manifest right after its observation window.
5. **`mode: all` requires justification.** Prefer `mode: one` (single pod)
   unless the scenario is specifically "total outage of this role"
   (e.g. `examples/chaos/pod-kill-horizon.yaml`).
6. `kube-system`, `chaos-mesh` and any namespace hosting shared infrastructure
   are **never** valid selector targets.

## Experiment catalog

Shipped, CI-verified experiments live in [`tests/chaos/`](../../tests/chaos/README.md)
and are applied with `kubectl apply -f <file> -n chaos-testing`:

| # | Manifest | Kind / action | Severity | SLO |
|---|----------|---------------|----------|-----|
| 01 | `01-operator-pod-kill.yaml` | PodChaos `pod-kill` (operator) | Critical | 180s |
| 02 | `02-network-partition.yaml` | NetworkChaos `partition` (operator ↔ API) | Critical | 180s |
| 03 | `03-api-latency.yaml` | NetworkChaos `delay` | High | 600s |
| 04 | `04-validator-peer-partition.yaml` | NetworkChaos `partition` (SCP peers) | High | 300s |
| 05 | `05-disk-fill.yaml` | PodChaos `pod-failure` / disk fill | Medium | 120s |
| 06 | `06-cpu-stress.yaml` | StressChaos cpu | Medium | 300s |
| 07 | `07-memory-pressure.yaml` | StressChaos memory | Medium | 300s |
| 08 | `08-validator-pod-kill.yaml` | PodChaos `pod-kill` (validators) | High | 300s |
| 09 | `09-cascading-failure.yaml` | Workflow: pod kill + partition | Critical | 600s |
| 10 | `10-io-stress.yaml` | StressChaos io | Medium | 300s |
| 11 | `11-stellar-core-crash-recovery.yaml` | PodChaos `container-kill` | Critical | 120s |
| 12 | `12-ledger-reconnection.yaml` | NetworkChaos partition ↔ world | Critical | 180s |
| 13 | `13-resource-exhaustion-recovery.yaml` | StressChaos cpu + memory | High | 300s |

> Note: the local runner `tests/chaos/run-chaos-tests.sh` currently executes
> 01–10; 11–13 are CI-only (they run in chaos-tests.yml group D). The SLO
> values above come from `default_slo_secs` in
> [`src/controller/chaos_engineering.rs`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/chaos_engineering.rs).

Documentation-focused, reusable starting points live in `examples/chaos/`:

| Manifest | Scenario | Validates |
|----------|----------|-----------|
| [`examples/chaos/network-partition.yaml`](../../examples/chaos/network-partition.yaml) | validator ↔ core quorum set partition | consensus reconnection; status honesty |
| [`examples/chaos/pod-kill-horizon.yaml`](../../examples/chaos/pod-kill-horizon.yaml) | Horizon pod killed mid-sync | operator pod re-creation + re-sync with **zero human intervention** |

## Scenario 1 — partition a node from its core quorum set

`examples/chaos/network-partition.yaml` severs all traffic between one
validator pod and every other validator for 120s:

```yaml
spec:
  action: partition
  mode: one
  selector:
    namespaces: [stellar-system]
    labelSelectors:
      app.kubernetes.io/name: stellar-node
      app.kubernetes.io/component: validator
      app.kubernetes.io/instance: validator-testnet   # edit me
  direction: both
  to:
    mode: all
    selector:
      namespaces: [stellar-system]
      labelSelectors:
        app.kubernetes.io/name: stellar-node
        app.kubernetes.io/component: validator
  duration: "120s"
```

The isolated node keeps its `spec.quorumSet` (a TOML string on the
`StellarNode` CR) but can no longer reach the validators listed in it —
exactly the "node loses quorum" failure that a disaster runbook cares about.

**What each observer should see during injection:**

| Observer | Expected signal | Where to look |
|----------|----------------|---------------|
| stellar-core (isolated pod) | SCP falls below quorum; ledger progress stops; peers show `disconnected` | `stellar-core --conf ... 'quorum-info'`, pod logs |
| Operator status | Validator pods stay `Ready=True` — **the operator has no SCP peer visibility** (validators are treated healthy while the pod runs, `src/controller/health.rs`) | `kubectl get stellarnodes -o yaml` |
| Horizon behind it | `/health` sync stalls → `Ready=False`, `Progressing=True`, reason `Syncing` | conditions on the `StellarNode` |
| Events | `QuorumUnderThreshold` / `NodeStale` style warnings when degradation is detected | `kubectl get events -n stellar-system` |

!!! important "Detection asymmetry is part of the assertion"

    This asymmetry is intentional and is *what we assert*: a quorum partition
    alone must NOT trigger destructive remediation (no pod eviction, no PVC
    deletion, `ClearAndResync` must not fire just because consensus stalled).
    Recovery of consensus is stellar-core's job; the operator's job is to keep
    the workload running and report state honestly.

**Post-injection assertions** (SLO 180s, mirrors CI experiment 12):

1. stellar-core re-establishes peer connections without a restart.
2. Ledger sequence resumes advancing.
3. Every affected `StellarNode` reaches condition `Ready=True`.
4. No operator panic; at most `WARN` reconciliation retries in logs.

## Scenario 2 — PodKill a syncing Horizon node (issue validation)

This is the acceptance check required by issue #295: kill a Horizon node that
is still syncing and prove the operator restarts and reconnects it with no
human intervention.

**Setup** — a Horizon `StellarNode` in `stellar-system` that is deliberately
still catching up (fresh database ⇒ long sync window). The CI workflow uses
this node (see `chaos-tests.yml` "Deploy test StellarNode"):

```bash
kubectl apply -f - <<'EOF'
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: chaos-test-horizon
  namespace: stellar-system
spec:
  nodeType: Horizon
  network: Testnet
  version: "v21.0.0"
  replicas: 1
  horizonConfig:
    databaseSecretRef: horizon-db-secret
    enableIngest: false
    stellarCoreUrl: "http://localhost:11626"
    ingestWorkers: 1
    enableExperimentalIngestion: false
    autoMigration: false
EOF

# Confirm it is syncing, not yet Ready:
kubectl -n stellar-system get stellarnode chaos-test-horizon \
  -o jsonpath='{.status.conditions[?(@.type=="Ready")].reason}'
# => NodeSyncing (or pod still Pending/Creating)
```

**Inject** four kill waves over 60s, then wait for recovery:

```bash
kubectl apply -f examples/chaos/pod-kill-horizon.yaml -n chaos-testing
sleep 75   # duration 60s + scheduler tail
kubectl delete -f examples/chaos/pod-kill-horizon.yaml -n chaos-testing --ignore-not-found

# Operator must converge the node back on its own (SLO 180s after last kill):
kubectl -n stellar-system wait --for=condition=Ready \
  stellarnode/chaos-test-horizon --timeout=180s
```

**Expected recovery sequence** (each step is produced by real operator code
paths; line refs at time of writing):

| Phase | Operator behavior | Source |
|-------|-------------------|--------|
| Kill detected | Pod disappears; StatefulSet recreates it immediately (normal Kubernetes mechanics) | `src/controller/reconciler.rs` |
| During rebuild | Health observation returns `Waiting for pod to be ready`; status phase `Creating`, `Ready=False` | `src/controller/health.rs` |
| Back during sync | Horizon `/health` reachable but not caught up → phase `Syncing`, conditions `Ready=False/NodeSyncing`, `Progressing=True/Syncing`, `Available=True` | `src/controller/reconciler.rs`, `src/controller/conditions.rs` |
| Caught up | `Ready=True/SyncComplete`, `Progressing=False`, `Degraded` removed | same |
| If it never catches up | 15-min stale-ledger watchdog escalates: level 1 `Restart` (operator deletes pods for clean recreation, event `AutoRemediationRestart`), level 2 `ClearAndResync` (event `AutoRemediationClearAndResync`), with a 10-minute cooldown between actions | `src/controller/remediation.rs` |
| Captive Core (Soroban-RPC/Horizon embedded core) | Restarted **in-container** by the supervisor: liveness probe every 5s, declared hung after 15s, SIGTERM with 5s grace, stale `/var/lib/stellar/core.lock` cleaned before respawn | `src/controller/captive/supervisor.rs` |

**Evidence to attach to the PR** (review requirement: "logs demonstrating the
operator successfully reconciling state during an active chaos injection"):

```bash
# 1. Operator logs across the injection window
kubectl logs -n stellar-system -l app=stellar-operator --tail=1000 \
  > exp-podkill-operator-logs.txt
# 2. Final CR state (conditions + timestamps)
kubectl -n stellar-system get stellarnode chaos-test-horizon -o yaml \
  > exp-podkill-stellarnode.yaml
# 3. Timing record
echo "{\"experiment_id\":\"horizon-pod-kill\",\"recovery_time_secs\":$RECOVERY,\"slo_secs\":180,\"recovered\":true}" \
  > exp-podkill-meta.json
```

Healthy runs look like this in the operator log (canonical patterns from
[`tests/chaos/README.md`](../../tests/chaos/README.md)):

```
WARN reconciliation error ... connection refused
INFO Reconciling StellarNode stellar-system/chaos-test-horizon
INFO Applied StellarNode: stellar-system/chaos-test-horizon
INFO Reconciled: ObjectRef { name: "chaos-test-horizon" ... }
```

Red flags that mean the experiment FAILED (from the same list): operator in
`CrashLoopBackOff` after chaos ends, `StellarNode` stuck in a failed/Creating
state past the timeout, duplicate Services/Deployments, stuck finalizers, or
any `panic` in operator logs.

## Running the suite

### Locally (kind)

```bash
chmod +x tests/chaos/run-chaos-tests.sh
./tests/chaos/run-chaos-tests.sh                    # experiments 01-10
EXPERIMENTS="01 02" ./tests/chaos/run-chaos-tests.sh  # subset
SKIP_SETUP=true RECOVERY_TIMEOUT=600 ./tests/chaos/run-chaos-tests.sh
kind delete cluster --name stellar-chaos            # teardown
```

Requires Docker, `kubectl`, `helm`, `kind`, `python3`. The script creates the
`stellar-chaos` cluster, installs Chaos Mesh 2.6.3, deploys the operator, and
writes per-run artifacts to `tests/chaos/results/<run-id>/`.

### In CI

`.github/workflows/chaos-tests.yml` runs nightly (02:00 UTC) and on demand via
`workflow_dispatch` (with an optional `skip_experiments` input). All 13
experiments run in 4 parallel kind clusters (groups A–D). It never gates pull
requests; see [Review workflow](#review-workflow-and-ci-parity) below.

### Scoring and reports

`tests/chaos/generate_report.py` aggregates the run into
`resilience-report.{md,json}`. Scoring (mirrored in
`compute_score` in `src/controller/chaos_engineering.rs`): recovered within SLO
= 100; late recovery = 70–99 (proportional overshoot, −30·overshoot/SLO capped
at 30); never recovered = 0. Overall score is severity-weighted (Critical 3×,
High 2×, Medium 1×). Recovery monitoring polls pod `Ready=True` every 5s with
a 10-minute hard ceiling (`monitor_recovery`).

## Review workflow and CI parity

A docs/manifest change to chaos tooling is merged only with evidence:

1. **PR description** must attach (or link to a workflow artifact containing)
   the log/status/meta triple from Scenario 2 for any change that alters
   recovery behavior, and a `resilience-report.md` for suite changes.
2. **Schema gates** — `examples/chaos/*.yaml` and `tests/chaos/*.yaml` are
   linted by `yamllint -c .yamllint.yml` and structurally validated by
   `scripts/validate-yaml-manifests.py` (both run on every PR).
   `tests/chaos/` additionally requires the Apache license header
   (`scripts/check-license-headers.py`); `examples/` is exempt.
3. **Nightly trend** — a new experiment is considered proven after it appears
   in at least one green nightly `chaos-tests.yml` run; the run's artifacts
   (`expNN-operator-logs.txt`, `expNN-stellarnode.yaml`, `expNN-meta.json`)
   are the durable evidence.

## Extending the suite — authoring checklist

- Copy `examples/chaos/network-partition.yaml` as the template; keep the
  header comment (Scenario / Success criteria / SLO / Usage).
- Use the `chaos.stellar.org/*` label convention and the `chaos-testing`
  namespace for new CRs.
- Pick an SLO from `default_slo_secs` or add the new experiment type there.
- Obey every rule in [Safety](#safety-namespace-isolation-constraints).
- Wire the manifest into `chaos-tests.yml` (its group) and, if it should run
  locally too, into `ALL_EXPERIMENTS` in `tests/chaos/run-chaos-tests.sh`.
- Add the doc→source mapping to `doc-coverage.toml` so the stale-docs gate
  keeps this file honest when the reconciler changes.

## See also

- [`tests/chaos/README.md`](../../tests/chaos/README.md) — experiment details
  and log interpretation
- [`docs/chaos-drills.md`](../chaos-drills.md) — monthly DR drill schedule
- [`docs/fuzzing.md`](../fuzzing.md) — reconciler state-machine fuzzing
- [`docs/reconciler-phases.md`](../reconciler-phases.md) — reconcile phase
  transition rules the chaos runs assert against
- [`docs/fmea-stellarnode.md`](../fmea-stellarnode.md) — failure mode and
  effects analysis that motivates the fault models above
- [Chaos Mesh documentation](https://chaos-mesh.org/docs/simulate-network-chaos-on-kubernetes/)
