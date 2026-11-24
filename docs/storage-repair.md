# Storage Corruption Recovery & Database Repair Playbook

Abrupt node failures, OOM kills mid-write, or underlying disk faults can leave a
Stellar-K8s node's database in a corrupted state — the pod goes into a crash
loop and ledger ingestion (Horizon) or consensus (`stellar-core`) halts. This
playbook covers diagnosing that condition and recovering the database
in-place, without a full resync/restore, when that's viable.

> **Before you start:** repairing a database in place is a **last resort**.
> If you have a recent [VolumeSnapshot](volume-snapshots.md) or
> [verified backup](backup-verification.md), restoring from it is safer
> and usually faster than manual repair. Use this playbook when no usable
> backup exists, or when the corruption is shallow enough that a targeted
> repair is clearly faster than a resync (e.g. a multi-hundred-GB Mainnet
> validator where a full history catchup would take days).

## Applies to

| Node type | Engine | Data path | Symptom source |
|---|---|---|---|
| `Validator` (`stellar-core`) | SQLite (bucket/ledger metadata DB) | `/opt/stellar/data` on the node's data PVC (`<node-name>-data`) | `stellar-node` container logs, `CrashLoopBackOff` |
| `Horizon` / `SorobanRpc` | PostgreSQL (via `DATABASE_URL`, either `spec.database` or `spec.managedDatabase`) | External to the node PVC — the Postgres StatefulSet/CNPG cluster's own PVC | Postgres pod logs, ingestion errors in `stellar-node` logs |

Horizon and Soroban RPC don't store their database on the node's own PVC —
they connect out to a Postgres instance. If that instance is a plain
StatefulSet (as used for [backup verification](backup-verification.md)),
the repair steps in this playbook apply directly to its PVC. If it's a
managed database (CNPG or an external provider), follow that system's own
recovery tooling instead — do not attach a debug pod to a PVC owned by
another operator.

## 1. Confirm it's storage corruption, not something else

Corruption symptoms are easy to mistake for config or network problems.
Rule those out first:

```bash
# Pod status and recent restarts
kubectl get pod -n <namespace> -l stellar.org/name=<node-name>

# Last logs before the crash
kubectl logs -n <namespace> <pod-name> -c stellar-node --previous
```

Look for:

- **SQLite**: `database disk image is malformed`, `file is not a database`,
  `database corruption at page N`, or `disk I/O error`.
- **PostgreSQL**: `invalid page in block N`, `could not read block N`,
  `PANIC: ... zero_damaged_pages`, or `unexpected chunk number` errors,
  usually accompanied by the Postgres pod itself refusing to reach `Ready`.

If instead you see connection refused, auth failures, or the pod scheduled
but pending on a PVC, that's a networking/RBAC/quota issue — this playbook
won't help; see [docs/troubleshooting/networking.md](troubleshooting/networking.md).

## 2. Stop writers to the affected PVC

A debug pod must never share a PVC with a running database process — mount
conflicts and concurrent writes will make corruption worse.

```bash
# Validator: scale the StatefulSet to zero
kubectl scale statefulset <node-name> -n <namespace> --replicas=0

# Horizon/SorobanRpc pointing at a separate Postgres StatefulSet
kubectl scale statefulset <postgres-statefulset-name> -n <namespace> --replicas=0
```

Wait for the pod to fully terminate before continuing:

```bash
kubectl wait --for=delete pod/<pod-name> -n <namespace> --timeout=120s
```

> **Note:** the Stellar-K8s operator will keep the StatefulSet's `spec.replicas`
> in sync with the `StellarNode`'s `suspended` field on the next reconcile. If
> you don't also set `spec.suspended: true` on the `StellarNode`, the operator
> may scale the StatefulSet back up while you're mid-repair. Suspend the node
> first:
> ```bash
> kubectl patch stellarnode <node-name> -n <namespace> --type=merge -p '{"spec":{"suspended":true}}'
> ```

## 3. Snapshot before you touch anything

Even a "just in case" copy is cheap compared to redoing a multi-day resync.
If your storage class supports CSI snapshots, take one now:

```bash
kubectl apply -f - <<EOF
apiVersion: snapshot.storage.k8s.io/v1
kind: VolumeSnapshot
metadata:
  name: <node-name>-pre-repair-$(date +%Y%m%d%H%M%S)
  namespace: <namespace>
spec:
  volumeSnapshotClassName: <your-volume-snapshot-class>
  source:
    persistentVolumeClaimName: <node-name>-data
EOF
```

If CSI snapshots aren't available, at minimum `tar` the data directory to a
scratch location from within the debug pod in step 4 before running any
repair command.

## 4. Launch a debug pod attached to the corrupted PVC

Use [`examples/debug/repair-pod.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/debug/repair-pod.yaml)
as a starting point. It mounts the node's existing PVC read-write and sleeps,
so you can `kubectl exec` into it and run repair commands interactively. Fill
in the placeholders (namespace, PVC name, storage class if relevant) before
applying, then:

```bash
kubectl apply -f examples/debug/repair-pod.yaml
kubectl wait --for=condition=Ready pod/db-repair -n <namespace> --timeout=60s
kubectl exec -it db-repair -n <namespace> -- sh
```

The pod's `alpine` base image ships without database clients to keep it
small; install what you need inside the pod once it's running (see below).
This avoids maintaining a separate bespoke image just for occasional repair
work.

## 5. Repair — SQLite (`stellar-core` validator data)

Inside the debug pod, install `sqlite3` and locate the database file (its
name depends on the `DATABASE` value in the node's `stellar-core.cfg`,
typically `stellar.db` under the mounted data directory):

```sh
apk add --no-cache sqlite

cd /repair-data
ls -la
```

**Diagnose first — always non-destructive:**

```sh
sqlite3 stellar.db "PRAGMA integrity_check;"
```

- If this returns `ok`, the database itself is fine and the crash has
  another cause — stop here and re-check step 1.
- Any other output lists the corrupted pages/tables.

**Attempt recovery, safest option first:**

```sh
# 1. Try the built-in recovery command (SQLite 3.29+) into a fresh file.
#    This reconstructs as much of the original data as possible without
#    trusting the damaged structures.
sqlite3 stellar.db ".recover" | sqlite3 stellar-recovered.db

# 2. Verify the recovered file
sqlite3 stellar-recovered.db "PRAGMA integrity_check;"
```

If `.recover` isn't available on the installed sqlite3 version, fall back to
dump/reload (slower, and will hard-fail if a page is unreadable rather than
just malformed):

```sh
sqlite3 stellar.db ".dump" | sqlite3 stellar-recovered.db
```

**Swap in the recovered file** only after the integrity check on
`stellar-recovered.db` comes back clean:

```sh
mv stellar.db stellar.db.corrupt.bak
mv stellar-recovered.db stellar.db
```

Exit the debug pod, then jump to step 6 to validate.

> If `stellar-core`'s ledger bucket files (not the SQLite catalog itself) are
> what's corrupted, in-place repair generally isn't viable — `stellar-core`
> validates bucket hashes against the ledger header on startup and will
> refuse to run on mismatched buckets. In that case, restore from a
> [VolumeSnapshot](volume-snapshots.md) or let the node resync via
> `historyArchiveUrls` instead of continuing this playbook.

## 6. Repair — PostgreSQL (Horizon / Soroban RPC database)

PostgreSQL page-level corruption is riskier to hand-repair than SQLite —
prefer restoring from a [verified backup](backup-verification.md)
(S3/pgBackRest/VolumeSnapshot) whenever one exists. The steps below are for
cases where no usable backup exists and the corruption is isolated to a
small number of pages/indexes.

Inside the debug pod:

```sh
apk add --no-cache postgresql16-client

# Start a scratch Postgres instance directly against the mounted PGDATA
# so you can run repair commands without going through the StatefulSet.
pg_ctl -D /repair-data start -o "-p 5433"
```

**Diagnose:**

```sh
# Table-level scan for damaged pages (safe, read-only)
psql -p 5433 -U postgres -c "SET zero_damaged_pages = off;" \
  -c "VACUUM (VERBOSE, DISABLE_PAGE_SKIPPING) horizon_history_ledgers;"
```

Repeat against each table logs point to, or script over
`information_schema.tables` to scan everything.

**If only a small number of pages in non-critical (rebuildable) tables or
indexes are affected**, the safest targeted fix is usually:

```sh
# Corrupted index: rebuild it, don't try to repair it in place
psql -p 5433 -U postgres -c "REINDEX INDEX CONCURRENTLY <index_name>;"

# Corrupted table page (last resort — zeroes the unreadable page,
# which means losing the rows on that page):
psql -p 5433 -U postgres -c "SET zero_damaged_pages = on;" \
  -c "VACUUM <table_name>;"
```

`zero_damaged_pages` silently discards data — after using it, re-run
Horizon's ingestion reingest for the affected ledger range rather than
trusting the table is complete:

```bash
kubectl exec -n <namespace> <horizon-pod> -c stellar-node -- \
  horizon db reingest range <start-ledger> <end-ledger>
```

Stop the scratch instance before exiting the debug pod:

```sh
pg_ctl -D /repair-data stop
```

## 7. Validate and resume

```bash
# Remove the debug pod
kubectl delete -f examples/debug/repair-pod.yaml

# Un-suspend and scale back up
kubectl patch stellarnode <node-name> -n <namespace> --type=merge -p '{"spec":{"suspended":false}}'
```

Watch the pod come back up clean:

```bash
kubectl get pod -n <namespace> -l stellar.org/name=<node-name> -w
kubectl logs -n <namespace> <pod-name> -c stellar-node -f
```

- **Validator**: confirm `stellar-core` reaches `Synced!` in its status
  (`kubectl exec ... -- curl -s localhost:11626/info`) and ledger sequence is
  advancing.
- **Horizon**: confirm `/health` returns healthy and `history_latest_ledger`
  advances (`kubectl exec ... -- curl -s localhost:8000/health`).

If the pod crash-loops again on the recovered data, don't keep repairing in
place — restore from the pre-repair snapshot taken in step 3 or a verified
backup, and treat further hand-repair attempts as data-loss risk.

## Safety summary

- Never attach a debug pod to a PVC while its owning StatefulSet pod is
  still running.
- Always snapshot or copy the data directory before running any repair
  command that isn't read-only.
- `PRAGMA integrity_check` / `VACUUM ... DISABLE_PAGE_SKIPPING` are
  diagnostics only — they don't modify data. Everything past that point does.
- `zero_damaged_pages` and dump/reload both **lose data** on unreadable
  pages; only use them when a resync/restore is more expensive than the
  loss, and always re-verify counts/ranges afterward.
- Delete the debug pod as soon as you're done — it mounts the raw data
  volume with no application-level access controls.
