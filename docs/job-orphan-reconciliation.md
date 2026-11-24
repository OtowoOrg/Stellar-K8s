# Job and CronJob Orphan Detection with Ownership Reconciliation

Epic #1503. Reclaims batch-workload debris (terminal Jobs, completed Pods and
deleted-owner remnants) that Kubernetes garbage collection cannot clean up,
because the owner chain was broken by a partial deletion, a namespace move, or
a CronJob that was deleted before its Jobs finished.

## Policy CRD

`JobRetentionPolicy` (`jrp`) is namespaced. The reconciler reads *only* these
values — there is no hardcoded TTL anywhere in the codebase.

```yaml
apiVersion: stellar.org/v1alpha1
kind: JobRetentionPolicy
metadata:
  name: job-retention
  namespace: stellar
spec:
  completedJobRetentionSeconds: 3600  # keep a Succeeded Job for an hour
  failedJobRetentionSeconds: 900     # minimum retention of a Failed Job
  stuckJobGraceSeconds: 3600          # floor: never reclaim a just-failed Job
  podRetentionSeconds: 1800           # keep a terminal Pod for 30 minutes
  deleteOrphanedJobs: true
  deleteOrphanedPods: true
  reconcileOwnerReferences: true      # re-point broken ownerReferences
  reclaimAfterNamespaceMove: true
  workloadLabel: app.kubernetes.io/managed-by
  workloadLabelValue: stellar-operator
  previousNamespaceAnnotation: stellar.org/previous-namespace
  gracePeriodSeconds: 30
  reconcileIntervalSeconds: 300
  dryRun: false
```

`stuckJobGraceSeconds` is a floor, not a separate window: a `Failed` Job is
reclaimed once it is older than `max(failedJobRetentionSeconds,
stuckJobGraceSeconds)`. This guarantees a Job that has just exhausted its
backoff is never deleted before its owner can react.

## Orphan classes

| Class | Detection | Action |
|---|---|---|
| `deleted_cron_job` | Job's `ownerReference` CronJob is absent from the namespace | delete |
| `broken_owner_reference` | owner name exists but the UID is stale (owner recreated) | re-point the `ownerReference` at the live UID |
| `stuck_failed_job` | Job is `Failed` past the effective retention | delete |
| `completed_pod` | terminal Pod of a terminal Job, past the pod window | delete |
| `namespace_move` | artifact carries `stellar.org/previous-namespace` and its owner is gone | delete |
| `deleted_owner_remnant` | no controller owner at all, in the configured sweep scope | delete |

## Safety properties

- An `Active`, `Pending` or `Suspended` Job is never deleted.
- A `Pending` or `Running` Pod is never deleted, and neither is a terminal Pod
  whose owning Job is still running (a Job may finish some pods while others
  run).
- Artifacts with no controller owner are only reclaimed when they carry
  `spec.workloadLabel=spec.workloadLabelValue`, so hand-written Jobs and Pods
  are untouched.
- Classification is a pure function of one listed snapshot, so a CronJob spec
  change that lands mid-cycle cannot produce a half-applied decision; it is
  picked up by the next sweep. Deleting something that changed in the meantime
  returns `404`, which is treated as success.
- `spec.dryRun` plans and reports without mutating anything.

## Metrics

- `stellar_job_orphans_reclaimed_total{namespace, kind, class}` — counter of
  reclaimed artifacts.
- `stellar_job_orphan_pods_outstanding{namespace}` — orphan Job pods still
  pending at the end of a sweep; must fall to zero (acceptance criterion).

## Status

Each sweep patches `status`: `reclaimedJobs`, `reclaimedPods`,
`repairedOwnerReferences`, a per-class `reclaimedByClass` breakdown, `retained`,
`dryRun`, `lastReconciled` and a `Ready` condition (`Swept` /
`PartiallySwept`).

## Code

- `src/crd/job_retention.rs` — the `JobRetentionPolicy` CRD.
- `src/controller/job_orphan_reconciler.rs` — pure classification/planning plus
  the `kube` reconcile path.
