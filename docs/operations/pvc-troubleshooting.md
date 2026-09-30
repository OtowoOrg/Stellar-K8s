# Persistent Volume Corruption and Storage Recovery Playbook

This playbook is for SREs recovering Stellar Core validator pods and Horizon
pods when the underlying PersistentVolumeClaim contains a corrupted SQLite
database, PostgreSQL data directory, bucket list, or ledger archive cache.

Use this document after you have ruled out ordinary pod scheduling, image pull,
network, and quorum issues. For a broader node failure runbook, see
[Disaster Recovery](./disaster-recovery.md). For snapshot restore behavior, see
[Volume Snapshots](../volume-snapshots.md).

> **Warning: recovery commands can destroy local state.**
> Do not delete a `StellarNode` custom resource as part of storage recovery.
> Stop the workload, attach a maintenance pod, copy evidence, and prefer a
> snapshot restore before resetting databases or deleting files.

## Quick decision matrix

| Symptom | Most likely damaged area | First non-destructive check | Recovery path |
|---|---|---|---|
| `database disk image is malformed` in Stellar Core logs | SQLite database file | `sqlite3 stellar.db "PRAGMA integrity_check;"` | Recreate Core DB with `stellar-core new-db`, then catch up |
| Core exits during bucket apply or logs bucket hash mismatch | Bucket list or history cache | `stellar-core offline-info` and bucket file listing | Restore bucket directory from snapshot or reset local state |
| Horizon returns 500s and ingest stalls | Horizon PostgreSQL database | `pg_amcheck`, `REINDEX`, and Horizon logs | Reindex, dump/restore, or rebuild Horizon ingest DB |
| PVC mounts read-only or files disappear | Filesystem or volume layer | pod events, `dmesg`, `fsck` from detached volume | Restore from CSI snapshot or cloud disk snapshot |
| Archive verification fails for local archive volume | Local history archive files | `stellar-core self-check` / checkpoint listings | Restore archive files or republish from a healthy node |

## Assumptions and placeholders

Commands use these shell variables. Set them once per incident and paste the
expanded values into the incident notes.

```bash
export NS=stellar
export NODE=validator-primary
export POD=validator-primary-0
export PVC=validator-primary-data
export CONFIG=/etc/stellar/stellar-core.cfg
export CONFIGMAP=validator-primary-config
export CORE_DATA=/mnt/stellar-data
export HORIZON_DATA=/mnt/stellar-data
export DATABASE_URL='postgresql://horizon:REDACTED@horizon-postgres-rw:5432/horizon'
```

Confirm the actual names before touching storage:

```bash
kubectl get stellarnode "$NODE" -n "$NS" -o wide
kubectl get pod,pvc -n "$NS" -l "app.kubernetes.io/instance=$NODE"
kubectl get pvc "$PVC" -n "$NS" -o jsonpath='{.spec.volumeName}{"\n"}'
```

Expected output:

```text
pvc-3b36f20d-5c3b-4e90-93b0-4f4d7f7b6d21
```

If the PVC name is not obvious, use the operator naming reference in
[Disaster Recovery](./disaster-recovery.md#naming-reference-so-commands-below-arent-guesswork).

## Phase 1: Stop writes and preserve evidence

1. Freeze the operator and stop the workload:

   ```bash
   kubectl patch stellarnode "$NODE" -n "$NS" --type=merge \
     -p '{"spec":{"maintenanceMode":true,"suspended":true}}'
   kubectl wait pod "$POD" -n "$NS" --for=delete --timeout=180s
   ```

   Expected output:

   ```text
   pod/validator-primary-0 condition met
   ```

   If the pod has already disappeared, `kubectl wait` can return `NotFound`.
   That is acceptable after confirming no replacement pod is running:

   ```bash
   kubectl get pods -n "$NS" -l "app.kubernetes.io/instance=$NODE"
   ```

2. Capture Kubernetes evidence:

   ```bash
   mkdir -p "incident-$NODE"
   kubectl get stellarnode "$NODE" -n "$NS" -o yaml > "incident-$NODE/stellarnode.yaml"
   kubectl get pvc "$PVC" -n "$NS" -o yaml > "incident-$NODE/pvc.yaml"
   kubectl get events -n "$NS" --sort-by=.lastTimestamp > "incident-$NODE/events.txt"
   kubectl describe pvc "$PVC" -n "$NS" > "incident-$NODE/pvc-describe.txt"
   ```

3. Create a CSI snapshot if the storage class supports it:

   ```bash
   cat > "incident-$NODE/snapshot.yaml" <<EOF
   apiVersion: snapshot.storage.k8s.io/v1
   kind: VolumeSnapshot
   metadata:
     name: ${PVC}-pre-recovery
     namespace: ${NS}
   spec:
     source:
       persistentVolumeClaimName: ${PVC}
   EOF
   kubectl apply -f "incident-$NODE/snapshot.yaml"
   kubectl wait volumesnapshot "${PVC}-pre-recovery" -n "$NS" \
     --for=jsonpath='{.status.readyToUse}'=true --timeout=10m
   ```

   Expected output:

   ```text
   volumesnapshot.snapshot.storage.k8s.io/validator-primary-data-pre-recovery condition met
   ```

> **Warning: do not run repair tools against a mounted live database.**
> The workload must be stopped before running `sqlite3`, `pg_resetwal`,
> `REINDEX`, filesystem repair, or `stellar-core new-db`.

## Phase 2: Attach a maintenance pod

Create the debug manifest from
[`examples/debug/maintenance-pod.yaml`](../../examples/debug/maintenance-pod.yaml),
set `metadata.namespace`, set `volumes[].persistentVolumeClaim.claimName` to
the affected PVC, set the optional ConfigMap volume to the node's generated
`$CONFIGMAP`, and apply it:

```bash
kubectl apply -f examples/debug/maintenance-pod.yaml
kubectl wait pod stellar-maintenance-debug -n "$NS" --for=condition=Ready --timeout=180s
```

Expected output:

```text
pod/stellar-maintenance-debug created
pod/stellar-maintenance-debug condition met
```

Open a shell in the container that matches the task:

```bash
kubectl exec -n "$NS" -it stellar-maintenance-debug -c core-tools -- bash
kubectl exec -n "$NS" -it stellar-maintenance-debug -c postgres-tools -- bash
kubectl exec -n "$NS" -it stellar-maintenance-debug -c filesystem-tools -- sh
```

Record the mounted data shape before changing anything:

```bash
find /mnt/stellar-data -maxdepth 3 -type f | sort | sed -n '1,120p'
du -sh /mnt/stellar-data
df -h /mnt/stellar-data
```

Expected output:

```text
3.2G    /mnt/stellar-data
Filesystem      Size  Used Avail Use% Mounted on
/dev/nvme1n1    500G  3.2G  497G   1% /mnt/stellar-data
```

## Phase 3: Diagnose Stellar Core SQLite corruption

Use this path when the validator uses SQLite, usually visible as
`DATABASE="sqlite3://..."` in `stellar-core.cfg`.

1. Locate the config and database:

   ```bash
   grep -n '^DATABASE=' "$CONFIG"
   find /mnt/stellar-data -name '*.db' -o -name '*.sqlite' -o -name 'stellar.db'
   ```

   Expected output:

   ```text
   12:DATABASE="sqlite3:///opt/stellar/data/stellar.db"
   /mnt/stellar-data/stellar.db
   ```

2. Run SQLite integrity checks:

   ```bash
   sqlite3 /mnt/stellar-data/stellar.db "PRAGMA quick_check;"
   sqlite3 /mnt/stellar-data/stellar.db "PRAGMA integrity_check;"
   ```

   Healthy output:

   ```text
   ok
   ok
   ```

   Corrupt output examples:

   ```text
   *** in database main ***
   Page 714: btreeInitPage() returns error code 11
   database disk image is malformed
   ```

3. Try a read-only logical dump into a clean file. This is useful only when the
   corruption is limited and SQLite can still scan the tables:

   ```bash
   cd /mnt/stellar-data
   sqlite3 stellar.db ".recover" > /tmp/stellar-recovered.sql
   sqlite3 /tmp/stellar-recovered.db < /tmp/stellar-recovered.sql
   sqlite3 /tmp/stellar-recovered.db "PRAGMA integrity_check;"
   ```

   Expected output:

   ```text
   ok
   ```

4. If recovery succeeded, keep the original and replace atomically:

   ```bash
   mv stellar.db "stellar.db.corrupt.$(date -u +%Y%m%dT%H%M%SZ)"
   install -m 0600 /tmp/stellar-recovered.db stellar.db
   ```

5. If recovery failed, reset local Stellar Core state and let Core catch up
   from configured history archives:

   ```bash
   stellar-core new-db --conf "$CONFIG"
   stellar-core catchup recent --conf "$CONFIG"
   stellar-core offline-info --conf "$CONFIG"
   ```

   Expected output includes a fresh ledger database and an offline info payload:

   ```text
   Content-Length: ...
   Content-Type: application/json
   ```

`stellar-core new-db` initializes or resets the local database and bucket state.
Use it only after the pre-recovery snapshot and evidence capture are complete.
For the difference between initializing a fresh volume and resetting a damaged
one, see [Database Schema Initialization](./db-schema-init.md).
`stellar-core catchup` runs archive catchup without joining the peer network.
`stellar-core offline-info` confirms the local database can be opened offline.

Some older internal runbooks refer to this reset-and-catchup step as "offline
instantiation." Do not assume the deployed image has an `offline-instantiate`
subcommand. Check the actual image first:

```bash
stellar-core --help offline-instantiate
```

If the command is absent, use the documented sequence above:
`stellar-core new-db`, `stellar-core catchup`, then `stellar-core offline-info`.

## Phase 4: Diagnose Stellar Core bucket or ledger archive corruption

Use this path when logs mention bucket hash mismatches, missing bucket files, or
history checkpoint verification failures.

1. Inspect Core's offline state:

   ```bash
   stellar-core offline-info --conf "$CONFIG"
   ```

   Healthy output includes JSON with current ledger state. Failure examples:

   ```text
   ERROR BucketListDB: failed to load bucket
   ERROR HistoryArchive: checkpoint hash mismatch
   ```

2. List bucket and history files:

   ```bash
   find /mnt/stellar-data -maxdepth 4 -type f \
     \( -path '*bucket*' -o -path '*history*' -o -name 'history*.json' \) \
     -printf '%s %TY-%Tm-%TdT%TH:%TM %p\n' | sort -n | tail -50
   ```

3. Verify configured history archives before resetting local state:

   ```bash
   grep -n 'HISTORY\\|CATCHUP' "$CONFIG"
   stellar-core report-last-history-checkpoint --conf "$CONFIG"
   stellar-core self-check --conf "$CONFIG"
   ```

   Expected output from a reachable archive includes the latest checkpoint
   ledger. Network, DNS, or 404 errors mean local repair is not enough.

4. Restore the bucket/history directory from a known-good snapshot when one is
   available. If no clean copy exists, reset the local DB and buckets:

   ```bash
   mv /mnt/stellar-data /mnt/stellar-data.corrupt.$(date -u +%Y%m%dT%H%M%SZ)
   mkdir -p /mnt/stellar-data
   stellar-core new-db --conf "$CONFIG"
   stellar-core catchup recent --conf "$CONFIG"
   ```

> **Warning: never hand-edit bucket files.**
> Bucket hashes are part of ledger state verification. Editing or deleting
> individual bucket files can create a node that appears to run but follows
> invalid local state. Restore the whole directory or rebuild from archives.

## Phase 5: Diagnose Horizon PostgreSQL corruption

Horizon normally stores ingest and API state in PostgreSQL. The PVC can be a
CloudNativePG cluster volume, a standalone PostgreSQL pod volume, or an external
database volume. Run PostgreSQL repair steps from a maintenance shell that has
network access to the database service, or from a detached PostgreSQL pod that
mounts the affected data directory.

1. Confirm connectivity and server state:

   ```bash
   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c 'select version();'
   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c \
     "select datname, pg_database_size(datname) from pg_database order by 2 desc;"
   ```

2. Run index and heap checks. `pg_amcheck` uses PostgreSQL's `amcheck`
   extension to detect relation corruption:

   ```bash
   pg_amcheck --jobs=4 --verbose "$DATABASE_URL"
   ```

   Healthy output ends without relation corruption errors. Corrupt output can
   name a damaged index or heap page:

   ```text
   error: bt_index_check failed for index "history_transactions_pkey"
   ```

3. For isolated index corruption, rebuild indexes first:

   ```bash
   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c 'REINDEX DATABASE horizon;'
   pg_amcheck --jobs=4 --verbose "$DATABASE_URL"
   ```

4. For a single table with readable rows, copy data out and rebuild the table
   under DBA supervision:

   ```bash
   pg_dump "$DATABASE_URL" --format=custom --file=/tmp/horizon-pre-repair.dump
   pg_restore --list /tmp/horizon-pre-repair.dump | sed -n '1,80p'
   ```

5. If PostgreSQL will not start because WAL is damaged, restore from the latest
   clean backup or snapshot. `pg_resetwal` is a last-resort data-loss tool:

   ```bash
   pg_controldata /var/lib/postgresql/data
   pg_resetwal --dry-run /var/lib/postgresql/data
   ```

> **Warning: do not run `pg_resetwal -f` as a normal repair step.**
> It can make an inconsistent cluster start by discarding WAL continuity.
> Use it only after a snapshot, with DBA approval, and only to extract data
> that will be dumped into a newly initialized cluster.

6. If Horizon ingest data is corrupt but Stellar Core and archives are healthy,
   the safest recovery is often a clean Horizon database rebuild:

   ```bash
   kubectl scale deploy "$NODE" -n "$NS" --replicas=0
   pg_dump "$DATABASE_URL" --schema-only --file=/tmp/horizon-schema.sql
   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c 'drop schema public cascade; create schema public;'
   kubectl scale deploy "$NODE" -n "$NS" --replicas=1
   ```

   Horizon should re-run migrations and reingest from Stellar Core according to
   its configured ingest range. Do not drop a production database until backups
   and retention requirements are confirmed.

## Phase 6: Filesystem and cloud volume checks

Use this path when Kubernetes reports mount errors, the kernel remounts the
filesystem read-only, or database tools fail with I/O errors.

1. Check Kubernetes and node events:

   ```bash
   kubectl describe pod "$POD" -n "$NS"
   kubectl describe pvc "$PVC" -n "$NS"
   kubectl get events -n "$NS" --sort-by=.lastTimestamp | tail -80
   ```

   Look for:

   ```text
   MountVolume.MountDevice failed
   I/O error
   EXT4-fs error
   volume attachment is being deleted
   ```

2. Confirm the PVC is mounted read/write in the maintenance pod:

   ```bash
   mount | grep /mnt/stellar-data
   touch /mnt/stellar-data/.stellar-k8s-write-test
   rm /mnt/stellar-data/.stellar-k8s-write-test
   ```

3. If filesystem repair is required, detach the PVC from all pods and use your
   storage provider's documented offline disk repair workflow. For ext4 on a
   detached block device, the shape is:

   ```bash
   e2fsck -f -n /dev/disk/by-id/<volume-id>
   e2fsck -f -y /dev/disk/by-id/<volume-id>
   ```

   `-n` is read-only diagnosis. `-y` writes repairs and must not be used until a
   cloud disk snapshot exists.

## Phase 7: Restart and verify recovery

1. Remove the maintenance pod:

   ```bash
   kubectl delete pod stellar-maintenance-debug -n "$NS"
   ```

2. Resume the node:

   ```bash
   kubectl patch stellarnode "$NODE" -n "$NS" --type=merge \
     -p '{"spec":{"maintenanceMode":false,"suspended":false}}'
   ```

3. Watch the pod and application logs:

   ```bash
   kubectl get pod -n "$NS" -l "app.kubernetes.io/instance=$NODE" -w
   kubectl stellar logs "$NODE" -n "$NS" --tail 200 -f
   ```

4. Confirm Stellar Core health:

   ```bash
   kubectl stellar status "$NODE" -n "$NS"
   kubectl exec -n "$NS" "$POD" -c stellar-core -- \
     stellar-core http-command info --conf /etc/stellar/stellar-core.cfg
   ```

   Expected output:

   ```text
   STATUS: CatchingUp
   ...
   STATUS: Synced
   ```

5. Confirm Horizon health when recovering Horizon:

   ```bash
   kubectl logs -n "$NS" deploy/"$NODE" --tail=200 | grep -i 'ingest\|migrate\|error'
   kubectl port-forward -n "$NS" deploy/"$NODE" 8000:8000
   curl -sf http://127.0.0.1:8000/ | jq .
   ```

## Local validation recipe

Use this reproducible lab path before changing production procedures.

1. Create a disposable namespace and apply a testnet validator.
2. Wait for the validator to initialize its SQLite database.
3. Suspend the node and mount the PVC with the maintenance pod.
4. Corrupt a copy of the database, not the only production file:

   ```bash
   cp /mnt/stellar-data/stellar.db /mnt/stellar-data/stellar.db.lab
   printf 'corrupt-page' | dd of=/mnt/stellar-data/stellar.db.lab bs=1 seek=4096 conv=notrunc
   sqlite3 /mnt/stellar-data/stellar.db.lab "PRAGMA integrity_check;"
   ```

   Expected output:

   ```text
   database disk image is malformed
   ```

5. Run `.recover`, verify the recovered database, then repeat with
   `stellar-core new-db` and `stellar-core catchup recent` to confirm both
   documented paths are executable in your cluster image.

6. Save a terminal recording or script log with:

   ```bash
   script -a "incident-$NODE/recovery-validation.typescript"
   # run the playbook commands
   exit
   ```

## Cleanup checklist

- [ ] Pre-recovery snapshot or cloud disk snapshot exists.
- [ ] Evidence bundle contains `stellarnode.yaml`, `pvc.yaml`, events, and logs.
- [ ] Maintenance pod is deleted.
- [ ] `spec.maintenanceMode` and `spec.suspended` are back to `false`.
- [ ] Stellar Core reaches `Synced` or Horizon resumes ingest.
- [ ] Corrupt files are retained only as long as incident policy requires.
- [ ] The final incident note records exact commands, timestamps, and outputs.

## References

- Stellar Core command reference:
  <https://developers.stellar.org/docs/validators/admin-guide/commands>
- Stellar Core environment preparation and `new-db` guidance:
  <https://developers.stellar.org/docs/validators/admin-guide/environment-preparation>
- PostgreSQL `pg_amcheck`:
  <https://www.postgresql.org/docs/16/app-pgamcheck.html>
- PostgreSQL `amcheck`:
  <https://www.postgresql.org/docs/17/amcheck.html>
- Maintenance pod manifest:
  [`examples/debug/maintenance-pod.yaml`](../../examples/debug/maintenance-pod.yaml)
