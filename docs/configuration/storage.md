# Storage

How the operator provisions and mounts persistent storage for a `StellarNode`,
and how that relates to the paths you configure on the node itself.

## The data volume

Every node gets exactly one `PersistentVolumeClaim` named `<node>-data`, built
from `spec.storage`. It is attached to the pod as the volume `data`.

| Node type | PVC claim | Volume | Mount path | Access |
| --- | --- | --- | --- | --- |
| Validator | `<node>-data` | `data` | `/opt/stellar/data` | Read-write |
| Horizon | `<node>-data` | `data` | `/data` | Read-write |
| SorobanRpc | `<node>-data` | `data` | `/data` | Read-write |

`/opt/stellar/data` is the path that matters for validator configuration. Any
file the node writes — the SQLite database, BucketList checkpoints, and a
`LOG_FILE_PATH` log if you set one — belongs under it. Everything else in the
container is read-only: the pod sets `readOnlyRootFilesystem: true`, and the
`config` `ConfigMap` volume is mounted read-only at `/config`.

## `spec.storage` fields

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `mode` | `PersistentVolume` \| `Local` | `PersistentVolume` | `Local` additionally honours `nodeAffinity` to pin to a storage node |
| `storageClass` | string | `standard` | Passed through as the PVC's `storageClassName` |
| `size` | string | see below | Resize the field to expand; see [Expansion](#expansion) |
| `retentionPolicy` | `Delete` \| `Retain` | `Delete` | `Retain` keeps the PVC when the CR is deleted |
| `annotations` | map | none | Applied to the PVC |
| `nodeAffinity` | object | none | Only meaningful when `mode: Local` |
| `snapshotRef` | object | none | Bootstrap from a snapshot or compressed backup |

Access mode is always `ReadWriteOnce`. A `StellarNode` runs as a single replica
per node, so a shared-filesystem access mode would not buy you anything.

### Sizing

If `spec.storage.size` is empty, the operator falls back to a default derived
from the history mode:

| `historyMode` | Default size |
| --- | --- |
| `Full` | `1500Gi` |
| `Recent` | `100Gi` |

Set `spec.storage.size` explicitly. Relying on the fallback couples your
capacity to an unrelated field, so changing `historyMode` on a node that never
set a size would silently request a different amount of storage.

Sizing must cover the ledger, BucketList checkpoints, and — if you set one — the
`LOG_FILE_PATH` log file. See
[Configuration](index.md#log-output-and-rotation) for why the log file is worth
budgeting for.

## Expansion

To grow a volume, edit `spec.storage.size` and apply. The operator reconciles
the new request onto the existing PVC.

This only works if the `StorageClass` has `allowVolumeExpansion: true`. With the
default `standard` class this is usually set, but many cloud and on-premises
classes ship with it disabled — check before you plan an expansion:

```bash
kubectl get storageclass standard -o jsonpath='{.allowVolumeExpansion}{"\n"}'
```

If it reads `false` or is empty, expansion is rejected by the API server and you
need a different class or a migration. See
[PVC Auto Expansion](../pvc-auto-expansion.md) and
[Proactive Disk Scaling](../proactive-disk-scaling.md).

Filesystem expansion only becomes visible inside the container after the pod
restarts. **The operator does not restart it for you.** The pod template
references the PVC by the stable name `<node>-data`, so changing only
`spec.storage.size` does not alter the pod spec and does not trigger a rollout.
Restart the workload yourself once the PVC reports the new capacity:

```bash
# Validator — runs as a StatefulSet, so pods are <node>-0, <node>-1, ...
kubectl -n <namespace> delete pod <node>-0

# Horizon / SorobanRpc — run as a Deployment
kubectl -n <namespace> rollout restart deployment/<node>
```

Either way the replacement pod comes up on the same PVC, so the expanded
filesystem is already there when it starts.

The workload kind is not configurable — it follows from `nodeType`:

| `nodeType` | Workload |
| --- | --- |
| `Validator` | `StatefulSet` |
| `Horizon` | `Deployment` |
| `SorobanRpc` | `Deployment` |

## Bootstrap from a snapshot

`spec.storage.snapshotRef` lets a new node start from existing data instead of
catching up from genesis. Two mechanisms are supported, and they are mutually
exclusive — set only one of `volumeSnapshotName` or `backupUrl`:

| Field | Mechanism | Behaviour |
| --- | --- | --- |
| `volumeSnapshotName` | CSI `VolumeSnapshot` | Set as the PVC's `dataSource`; volume clone, near-instant, no init container |
| `backupUrl` | Compressed archive in S3 or on a PVC | Injects an init container that downloads and extracts the archive before the node starts |

`snapshotRef.volumeSnapshotName` takes precedence over
`spec.restoreFromSnapshot.volumeSnapshotName` when both are set.

The archive-restore init container is idempotent: it checks whether the data
directory is already populated and exits immediately if it is, so a re-run does
not overwrite live data.

For the surrounding procedure see
[Backup and Disaster Recovery Runbook](../backup-disaster-recovery-runbook.md)
and [Volume Snapshots](../volume-snapshots.md).

## Interaction with node configuration

Two consequences follow from the single-volume layout, both covered in more
detail in [Configuration](index.md):

- **Only `/opt/stellar/data` is writable and persistent.** A `LOG_FILE_PATH` or
  `BUCKET_DIR_PATH` pointing elsewhere either fails to open (read-only root
  filesystem) or lands in a container filesystem that is discarded when the pod
  is replaced.
- **Node state and log files share one allocation.** Anything written under
  `/opt/stellar/data` draws on the same `spec.storage.size` as the ledger.

## Related

- [Configuration](index.md) — generated ConfigMap keys, `LOG_FILE_PATH`, `BUCKET_DIR_PATH`
- [CRD Reference](crd-reference.md) — every `StellarNode` field
- [Proactive Disk Scaling](../proactive-disk-scaling.md)
- [PVC Auto Expansion](../pvc-auto-expansion.md)
- [Volume Snapshots](../volume-snapshots.md)
- [Archive Pruning](../archive-pruning.md)
