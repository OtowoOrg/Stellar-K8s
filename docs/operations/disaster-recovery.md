# Disaster Recovery & Quorum Loss Runbook

**Audience:** on-call operators responding to a live incident. **Goal:** stop the
bleeding without causing a ledger fork.

This runbook covers single-cluster, single-node incidents: a validator pod that
won't come back, a corrupted data volume, and a validator that has fallen so far
out of consensus it needs a full resync from history archives. For a
region-level outage where you're failing an entire quorum slice over to a
standby cluster, see [DR Failover Guide](../dr-failover.md) instead — that is a
different procedure with different risks (promoting a standby's quorum weight)
and is out of scope here.

**⚠️ The #1 rule: never let two copies of the same validator's seed key run
`stellar-core` at the same time.** A validator that double-signs at the same
slot from two processes is a slashable, network-visible SCP violation and (on
networks that treat it that way) can contribute to a fork. Every mitigation
below scales the validator to **zero** replicas before touching its disk or
signing key, and only scales back up once you've confirmed there is exactly one
copy.

## Contents

- [Prerequisites](#prerequisites)
- [Recovery Mode](#recovery-mode) — the shared pattern used by all three scenarios
- [Scenario 1: Complete Pod Failure](#scenario-1-complete-pod-failure)
- [Scenario 2: Corrupted PVC Recovery](#scenario-2-corrupted-pvc-recovery)
- [Scenario 3: Total Quorum Loss — Forcing a Sync from Archives](#scenario-3-total-quorum-loss-forcing-a-sync-from-archives)
- [Post-Incident](#post-incident)

## Prerequisites

- `kubectl` configured against the affected cluster, and the
  [`kubectl stellar` plugin](../kubectl-plugin.md) installed.
- Admin access to the `StellarNode` custom resource and to Pods/PVCs in its
  namespace.
- Know the node's name and namespace. Everywhere below, replace `<node>` and
  `<namespace>`; commands assume you've exported them:
  ```bash
  export NODE=my-validator
  export NS=stellar-nodes
  ```

## Recovery Mode

"Recovery Mode" isn't a single CRD flag — it's two `StellarNode` fields used
together, applied **in this order**:

1. **Relax the readiness probe first**, while the operator is still reconciling
   normally. Set generous [`spec.probes.readiness`](../api-reference.md)
   thresholds so Kubernetes doesn't kill or endlessly restart a pod that's
   legitimately busy doing a multi-hour catchup instead of failing:
   ```yaml
   spec:
     probes:
       readiness:
         initialDelaySeconds: 60
         periodSeconds: 30
         failureThreshold: 240 # ~2h grace before k8s gives up on this pod
   ```
   `kubectl apply -f -` (or `kubectl patch stellarnode`) this change and wait
   for `kubectl rollout status statefulset/$NODE -n $NS` — confirm the new
   thresholds landed on the live StatefulSet before proceeding:
   ```bash
   kubectl get statefulset $NODE -n $NS \
     -o jsonpath='{.spec.template.spec.containers[0].readinessProbe}'
   ```

2. **Then freeze the operator** with `maintenanceMode: true`:
   ```bash
   kubectl patch stellarnode $NODE -n $NS --type merge \
     -p '{"spec":{"maintenanceMode":true}}'
   ```
   This must be the *second* step, not combined with step 1 in one apply.
   `maintenanceMode` makes the operator return immediately on its next
   reconcile ("Manual maintenance mode active; workload management paused",
   `status.phase: Maintenance`) *before* it evaluates anything else about the
   node's spec, including a probe change you apply at the same time — so
   sequencing the probe relaxation first is what makes it actually take
   effect. While frozen, the operator will not scale, restart, or repatch
   anything for this node, so `kubectl exec`, `kubectl delete pod`, and manual
   `kubectl patch statefulset` calls all stick instead of being reconciled
   away underneath you.

3. **When the incident is resolved**, reverse the order: clear
   `maintenanceMode` first, confirm the operator has reconciled the node back
   to a healthy phase (`kubectl stellar status $NODE -n $NS`), *then* remove
   the probe override (or leave it — a generous-but-finite readiness
   threshold is harmless in steady state).

Ready-to-adapt manifests for both steps are at
[`examples/recovery-mode-1-relax-probes.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/recovery-mode-1-relax-probes.yaml)
and
[`examples/recovery-mode-2-maintenance-mode.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/recovery-mode-2-maintenance-mode.yaml) —
kept as two separate files on purpose, so there's no single `kubectl apply`
that accidentally applies both at once and skips the confirmation step
between them.

Two related fields, used inside the scenarios below rather than as "Recovery
Mode" itself:

- **`suspended: true`** — scales the StatefulSet to `0` replicas but keeps the
  Service alive (for peer discovery) and, if `storage.retentionPolicy` is
  `Retain`, keeps the PVC. The operator keeps reconciling everything else
  while suspended; only `maintenanceMode` freezes the operator entirely. This
  is the safe way to stop `stellar-core` on a node without deleting it.
- **`storage.retentionPolicy: Retain`** — only matters if you delete the
  `StellarNode` resource itself; the [finalizer](../adr/0003-kube-rs-finalizers.md)
  (`stellarnode.stellar.org/finalizer`) checks it before deleting the PVC. It
  does **not** stop you from deleting the PVC directly with `kubectl delete
  pvc` — that's the mechanism Scenario 2 uses on purpose.

## Scenario 1: Complete Pod Failure

**Symptom**

`kubectl stellar status $NODE -n $NS` shows the node stuck outside `Ready`
(`Pending`, `Creating`, or repeatedly flapping), and:

```bash
kubectl get pods -n $NS -l app.kubernetes.io/instance=$NODE,app.kubernetes.io/name=stellar-node
```

shows `CrashLoopBackOff`, `Error`, or a pod stuck in `Pending`.

**Diagnosis**

```bash
# Pod-level detail: events, last termination reason, exit code
kubectl describe pod -n $NS -l app.kubernetes.io/instance=$NODE

# Last known logs from the crashed container (main container is "stellar-node")
kubectl logs -n $NS -l app.kubernetes.io/instance=$NODE -c stellar-node --previous --tail=200
# or: kubectl stellar logs $NODE -n $NS --tail 200

# Is this a scheduling problem, not a crash? (Pending pods)
kubectl get events -n $NS --sort-by='.lastTimestamp' | grep -i "$NODE\|FailedScheduling\|Insufficient"
```

Common root causes and their signature:

| Signature in logs/events | Likely cause | Go to |
|---|---|---|
| `FailedScheduling`, `Insufficient cpu/memory` | Node pool out of capacity | Scale the node pool; not a data issue |
| `sqlite3.OperationalError`, `database disk image is malformed`, repeated crash immediately after DB open | Corrupted data volume | [Scenario 2](#scenario-2-corrupted-pvc-recovery) |
| `Out of sync with the network`, no new SCP messages for a long window, `ERROR SCP ... quorum` | Quorum/consensus failure | [Scenario 3](#scenario-3-total-quorum-loss-forcing-a-sync-from-archives) |
| `OOMKilled` in `kubectl describe pod` | Undersized memory limit | Raise `spec.resources.limits.memory`; not covered here |
| Pod `Running`, container never `Ready` | Slow catchup, or probe too strict | Check `kubectl stellar logs $NODE -c stellar-node -f` for catchup progress; consider [Recovery Mode](#recovery-mode) if it's just slow |

**Mitigation**

If it's a plain crash loop with no data corruption (e.g. a transient panic, a
bad env change that's since been reverted, a node drain that raced the pod):

```bash
# Force a clean restart — the StatefulSet controller recreates the pod
kubectl delete pod -n $NS -l app.kubernetes.io/instance=$NODE

# Watch it come back
kubectl get pods -n $NS -l app.kubernetes.io/instance=$NODE -w
```

If the pod is `Pending` on a drained/cordoned node and nothing is
rescheduling it, check for a stuck `PodDisruptionBudget` (see
[pod-disruption-budget.md](../pod-disruption-budget.md)) before forcing
anything.

**Resolution**

```bash
kubectl stellar status $NODE -n $NS
```

Confirm `Ready=True` and, for a validator, tail the core logs for consensus
participation (there's no `/health` endpoint on validators — see
[health-checks.md](../health-checks.md)):

```bash
kubectl stellar logs $NODE -n $NS -c stellar-node -f | grep -i "SCP\|ledger closed"
```

If the pod keeps crashing after a clean restart, it isn't a transient
failure — move to Scenario 2 or 3 based on the log signature above.

## Scenario 2: Corrupted PVC Recovery

**Symptom**

The container repeatedly crashes on startup with a storage-layer error
(`database disk image is malformed`, `I/O error`, bucket file checksum
failures), and a clean pod restart (Scenario 1's mitigation) doesn't help —
the new pod hits the same disk and crashes the same way.

**Diagnosis**

```bash
# Confirm it's the disk, not the process: exec a shell and inspect directly
# (only works if the pod is still up long enough to exec into; if it's
# crash-looping, apply Recovery Mode's probe relaxation first so it stays up,
# or exec during the brief Running window before the crash)
kubectl exec -n $NS -it $(kubectl get pod -n $NS -l app.kubernetes.io/instance=$NODE \
  -o jsonpath='{.items[0].metadata.name}') -c stellar-node -- df -h /data

kubectl stellar sql $NODE -n $NS "PRAGMA integrity_check;"
```

If you suspect the corruption itself is evidence of misbehavior (not just a
bad disk) rather than routine hardware failure, capture a forensic snapshot
**before** you destroy anything — see
[forensic-snapshot.md](../forensic-snapshot.md):

```bash
kubectl annotate stellarnode $NODE -n $NS \
  stellar.org/request-forensic-snapshot=true --overwrite
# Watch status.forensicSnapshotPhase go Capturing -> Complete before continuing
```

**Mitigation**

The PVC is corrupted, not the `StellarNode` spec, so this recovers the volume
in place rather than deleting the `StellarNode` resource (which would also
tear down its Service and any dependent config).

```bash
# 1. Stop stellar-core cleanly. suspended scales to 0 but the operator keeps
#    the PVC and Service around, and keeps reconciling everything else.
kubectl patch stellarnode $NODE -n $NS --type merge -p '{"spec":{"suspended":true}}'

# Wait for the pod to actually terminate before touching the PVC
kubectl wait --for=delete pod -l app.kubernetes.io/instance=$NODE -n $NS --timeout=120s

# 2. Delete only the corrupted PVC. This does NOT need retentionPolicy:
#    Retain or finalizer involvement — you're deleting the PVC object
#    directly, not the StellarNode.
kubectl delete pvc $NODE-data -n $NS

# 3. Let the operator recreate an empty PVC on its next reconcile
#    (it re-creates on 404, same as it would for a brand-new node).
kubectl get pvc $NODE-data -n $NS -w
# Ctrl-C once STATUS is Bound

# 4. Resume. The pod comes back with an EMPTY data volume, so stellar-core
#    starts a full catchup from historyArchiveUrls configured on the node —
#    this is the same archive-sync path as Scenario 3.
kubectl patch stellarnode $NODE -n $NS --type merge -p '{"spec":{"suspended":false}}'
```

If the catchup is going to take a long time (it will — a full-history replay
can run for hours), apply [Recovery Mode](#recovery-mode) now so Kubernetes
doesn't cycle the pod for taking too long to become ready.

**Faster alternative**: if this node has `snapshotSchedule` configured and a
recent `VolumeSnapshot` exists (`kubectl get volumesnapshots -n $NS -l
stellar.org/snapshot-of=$NODE`), restoring the PVC from that snapshot instead
of an empty volume skips the archive replay almost entirely. That requires
recreating the `StellarNode` with `restoreFromSnapshot` set rather than
patching the PVC in place — see
[volume-snapshots.md](../volume-snapshots.md) for the full procedure and
trade-offs (you lose whatever ledger history postdates the snapshot, made up
by a much shorter catchup afterward).

**Resolution**

```bash
kubectl stellar logs $NODE -n $NS -c stellar-node -f
```

Look for catchup progress messages advancing toward the current ledger, then:

```bash
kubectl stellar status $NODE -n $NS
```

Once `Ready=True` and logs show `Ledger closed` at a current sequence, the
node is fully recovered. If you left `maintenanceMode` set as part of Recovery
Mode, clear it now (see [Recovery Mode](#recovery-mode) step 3).

## Scenario 3: Total Quorum Loss — Forcing a Sync from Archives

**Symptom**

The validator can't reach consensus at all — not "slow", but stuck: no new
ledgers close, and core logs show it never has enough of its quorum set
online/agreeing to form SCP consensus (`Out of sync`, quorum-not-met
warnings that never resolve). This is the scenario where the local ledger
state itself may be unrecoverable through normal peer sync and the node needs
to be reseeded from a trusted history archive instead.

**Diagnosis**

```bash
# Confirm the network state, not just this node
kubectl stellar logs $NODE -n $NS -c stellar-node --tail=200 | grep -i "quorum\|SCP\|out of sync"

# Check which archive(s) this validator is configured to trust
kubectl get stellarnode $NODE -n $NS \
  -o jsonpath='{.spec.validatorConfig.historyArchiveUrls}'

# From inside the pod, confirm the archive is actually reachable
kubectl exec -n $NS -it $(kubectl get pod -n $NS -l app.kubernetes.io/instance=$NODE \
  -o jsonpath='{.items[0].metadata.name}') -c stellar-node -- \
  curl -fsI "$(kubectl get stellarnode $NODE -n $NS -o jsonpath='{.spec.validatorConfig.historyArchiveUrls[0]}')/.well-known/stellar-history.json"
```

Rule out a lower-level network problem first — Submariner/BGP peering issues
can look identical to genuine quorum loss; see
[peer-discovery.md](../peer-discovery.md) and
[metallb-bgp-anycast.md](../metallb-bgp-anycast.md).

**Mitigation**

This wipes the node's locally-persisted SCP/ledger state and forces a replay
from the configured history archive. **Only do this on a node that has
genuinely lost quorum** — never on a node that's simply catching up normally,
and never while a second copy of the same seed key might still be running
(see the rule at the top of this doc).

```bash
# 1. Apply Recovery Mode (probe relaxation, then maintenanceMode) — see
#    "Recovery Mode" above. This keeps the pod alive and un-cycled while you
#    work inside it, and stops the operator from reconciling around you.

# 2. Confirm exactly one pod, then exec in
kubectl get pods -n $NS -l app.kubernetes.io/instance=$NODE
kubectl exec -n $NS -it $(kubectl get pod -n $NS -l app.kubernetes.io/instance=$NODE \
  -o jsonpath='{.items[0].metadata.name}') -c stellar-node -- bash

# --- inside the pod ---
# Stop core if it's the foreground process of this container, or send it a
# clean shutdown signal per your image's process supervisor before touching
# the DB/buckets — never run new-db against a live core process.

# Reinitialize the local ledger/SCP state
stellar-core new-db

# Replay history from the archive up to the current network ledger. Use
# 'current/0' to catch up fully to the archive's latest checkpoint with no
# lookback trimming.
stellar-core catchup current/0

# Sanity-check before handing back to the supervisor
stellar-core http-command 'info'
# --- exit the pod ---

# 3. Let the container's normal entrypoint take over (restart the pod so
#    core comes back up under supervision instead of as your manual exec):
kubectl delete pod -n $NS -l app.kubernetes.io/instance=$NODE
```

**Resolution**

```bash
kubectl stellar logs $NODE -n $NS -c stellar-node -f | grep -i "SCP\|ledger closed\|quorum"
```

Confirm the node forms SCP consensus with its quorum set again and ledgers
are closing at the current network sequence. Then reverse Recovery Mode (see
[Recovery Mode](#recovery-mode) step 3): clear `maintenanceMode` first, verify
`kubectl stellar status $NODE -n $NS` reports `Ready=True`, then remove the
probe override.

## Post-Incident

- Generate an artifact bundle for the incident window (operator logs, pod
  logs, events) with the built-in tool rather than hand-collecting them:
  ```bash
  stellar-operator incident-report --namespace $NS --since 2h --output incident.zip
  # or: kubectl stellar incident-report --namespace $NS --since 2h --output incident.zip
  ```
- File a post-mortem using the repo's
  [post-mortem template](../incident-response/post-mortem.md).
- If this incident revealed a gap in this runbook (a command that didn't
  work as documented, a missing scenario), update this document — it's meant
  to stay accurate under pressure, not just at review time.

## References


