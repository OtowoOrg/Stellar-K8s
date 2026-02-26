# Zero-Downtime Ledger Snapshots via CSI VolumeSnapshots

Syncing a new Stellar Validator from a history archive can take hours or days. The Stellar-K8s operator supports **near-instant bootstrapping** of new nodes by taking live disk snapshots using Kubernetes CSI VolumeSnapshots and restoring from them.

## Overview

- **Snapshot source node**: Configure `snapshotSchedule` on a Validator StellarNode. The operator will create `VolumeSnapshot` resources targeting the node's data PVC on a schedule or when you set an annotation.
- **Restore target node**: Configure `restoreFromSnapshot` on a new Validator StellarNode. Its PVC will be created from the specified VolumeSnapshot so the node starts with existing ledger data.

Both features are **Validator-only** (they apply to the Stellar Core ledger data PVC).

## Prerequisites

1. **CSI driver with snapshot support**: Your cluster storage must support the [Kubernetes Volume Snapshot](https://kubernetes.io/docs/concepts/storage/volume-snapshots/) API. For example:
   - AWS EBS CSI driver
   - GCE PD CSI driver
   - Azure Disk CSI driver
   - OpenStack Cinder CSI
   - Other CSI drivers that implement the snapshot controller interface

2. **Snapshot controller and CRDs**: Install the [external-snapshotter](https://github.com/kubernetes-csi/external-snapshotter) components (Snapshot CRDs and snapshot-controller). Many managed Kubernetes offerings include these or provide a VolumeSnapshotClass.

3. **VolumeSnapshotClass**: At least one `VolumeSnapshotClass` must exist (or rely on the default for your driver).

## Snapshot schedule

Add `snapshotSchedule` to a Validator StellarNode to have the operator create VolumeSnapshots of its data PVC.

### Behavior

1. **Optional flush**: If `flushBeforeSnapshot` is `true`, the operator can attempt to flush/lock the Stellar database briefly before the snapshot for consistency. (Implementation may vary; many CSI drivers provide crash-consistent snapshots without application cooperation.)
2. **Create VolumeSnapshot**: The operator creates a `VolumeSnapshot` resource (API group `snapshot.storage.k8s.io/v1`) whose `source.persistentVolumeClaimName` is the node's data PVC (e.g. `<node-name>-data`).
3. **Resume**: Normal operations continue; the snapshot is taken by the storage layer without stopping the node.

### Triggering snapshots

- **On a schedule**: Set `schedule` to a cron expression (e.g. `0 2 * * *` for daily at 2 AM UTC). The operator evaluates the schedule on each reconciliation and creates a snapshot when the next run time has passed.
- **On demand**: Set the annotation `stellar.org/request-snapshot: "true"` on the StellarNode. The operator will create one snapshot and then clear the annotation. Example:
  ```bash
  kubectl annotate stellarnode my-validator stellar.org/request-snapshot=true -n stellar-nodes
  ```

### Retention

Set `retentionCount` to a positive number to keep only the N most recent snapshots per node. The operator lists VolumeSnapshots with label `stellar.org/snapshot-of=<node-name>` and deletes the oldest when over the limit.

### Example (schedule + on-demand)

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-primary
  namespace: stellar-nodes
spec:
  nodeType: Validator
  network: Testnet
  version: "21.0.0"
  storage:
    storageClass: standard-rwo
    size: "500Gi"
  snapshotSchedule:
    schedule: "0 2 * * *"          # daily at 2 AM UTC
    volumeSnapshotClassName: csi-gce-pd-snapshot-class  # optional
    flushBeforeSnapshot: false
    retentionCount: 7
  validatorConfig:
    seedSecretRef: validator-seed
    # ...
```

## Restore from snapshot

To bootstrap a **new** Validator from an existing snapshot:

1. Ensure the source VolumeSnapshot exists (created by the operator or manually) and is ready.
2. Create a new StellarNode with `restoreFromSnapshot` pointing to that snapshot. The node's PVC will be created with `dataSource` set to the VolumeSnapshot, so the volume is populated from the snapshot instead of being empty.

### Example

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-restored
  namespace: stellar-nodes
spec:
  nodeType: Validator
  network: Testnet
  version: "21.0.0"
  storage:
    storageClass: standard-rwo
    size: "500Gi"
  restoreFromSnapshot:
    volumeSnapshotName: validator-primary-data-20250224-020000  # from snapshot schedule or manual snapshot
  validatorConfig:
    seedSecretRef: validator-restored-seed
    # ...
```

The PVC `validator-restored-data` will be created from the given VolumeSnapshot. The new node can start with the ledger state from the snapshot and catch up from the network.

### Cross-namespace (optional)

If your cluster supports the CrossNamespaceVolumeDataSource feature and the snapshot is in another namespace, you can set `restoreFromSnapshot.namespace` to that namespace. Otherwise, the VolumeSnapshot must be in the same namespace as the StellarNode.

## Flow summary

| Step | Snapshot (source node) | Restore (new node) |
|------|------------------------|---------------------|
| 1 | Optional: flush DB (if configured) | — |
| 2 | Operator creates VolumeSnapshot targeting node's PVC | Operator creates PVC with dataSource = VolumeSnapshot |
| 3 | Node keeps running | Node starts with snapshot data |
| 4 | Retention prunes old snapshots (if retentionCount &gt; 0) | — |

## YAML examples

See [examples/validator-volume-snapshots.yaml](../examples/validator-volume-snapshots.yaml) for complete examples: a Validator with snapshot schedule, and a second Validator that restores from a snapshot.

## References

- [Kubernetes Volume Snapshots](https://kubernetes.io/docs/concepts/storage/volume-snapshots/)
- [CSI Volume Snapshots](https://kubernetes-csi.github.io/docs/snapshot-controller.html)
- [Volume Snapshot Classes](https://kubernetes.io/docs/concepts/storage/volume-snapshot-classes/)
