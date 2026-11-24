# Disaster Recovery & Backup Verification Automation

A successful backup job is not evidence that the backup can be restored.
This runbook explains how to run **nightly recovery tests** that restore a
snapshot into an isolated namespace, check SQL integrity and ledger hashes,
report pass/fail, and clean up every temporary resource — including on
failure.

It complements the operator-native `spec.backupVerification` scheduler
documented in [Automated Backup Verification](../backup-verification.md).
Use that CRD field when the operator should own the loop. Use the CronJob
in [`examples/cronjobs/backup-verifier.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/cronjobs/backup-verifier.yaml)
when you want an independent nightly drill that does not reconcile
production `StellarNode` objects.

**Related:** [Volume Snapshots](../volume-snapshots.md) ·
[DR Failover](../dr-failover.md) ·
[Quorum Loss Runbook](disaster-recovery.md) ·
[Byzantine / Slack / PagerDuty patterns](../byzantine-monitoring.md)

---

## Why backup completion is not enough

The operator can report that a CSI `VolumeSnapshot` or an S3/pgBackRest
upload finished without error. That only proves the **write path** worked.
It does not prove:

- The snapshot is restorable onto a new PVC
- Horizon tables are complete and queryable
- Stellar Core bucket / ledger hashes match a known-good checkpoint
- Restore finishes inside your RTO
- The backup is new enough for your RPO

Corruption, incomplete flushes, wrong snapshot class, expired credentials,
and cross-namespace restore gaps only appear when you **actually restore**.
Treat "backup job succeeded" as a necessary signal, not a sufficient one.

The in-tree verifier (`src/backup/verification.rs`) already follows this
rule: it creates a temporary namespace, restores, runs SQL checks, then
deletes the namespace. Nightly CronJobs apply the same discipline outside
the operator process so a stuck reconciler cannot silently skip drills.

---

## Architecture: nightly recovery testing

```
  CronJob backup-verifier (stellar-backup-verify)
            │
            ├─ 1. Resolve latest Ready VolumeSnapshot (read-only)
            ├─ 2. Create namespace verify-backup-<unix>
            │      labels: stellar.org/backup-verification=true
            │             stellar.org/ephemeral=true
            ├─ 3. Apply ResourceQuota + NetworkPolicy (egress limited)
            ├─ 4. Create PVC dataSource=VolumeSnapshot (test ns only)
            ├─ 5. Run validation Job (SQL and/or ledger hash)
            ├─ 6. Emit pass/fail report + Slack / PagerDuty
            └─ 7. trap EXIT: delete the test namespace and children
  CronJob backup-verifier-gc
            └─ delete leftover verify-backup-* namespaces past TTL
```

Components (all in `examples/cronjobs/backup-verifier.yaml`):

| Object | Purpose |
|---|---|
| `ServiceAccount` `backup-verifier` | Identity for the job |
| `ClusterRole` / `ClusterRoleBinding` | Create ephemeral namespaces; **get/list** snapshots only |
| `ConfigMap` `backup-verifier-scripts` | Orchestrator + SQL/hash templates |
| `CronJob` `backup-verifier` | Nightly restore + validate + report |
| `CronJob` `backup-verifier-gc` | Orphan namespace reaper |

The controller-based path uses `BackupVerificationScheduler` and the same
cleanup-after-verify pattern. Do not enable both against the **same**
snapshot on the same night without staggering schedules — they contend for
storage IOPS, not for production PVCs.

---

## Isolated namespaces and production safety

Every live run creates a new namespace named
`verify-backup-<unix-seconds>`. The orchestrator refuses to continue if:

1. The computed name does not start with `verify-backup-`
2. The name (or `SNAPSHOT_NAMESPACE`) is in `PROTECTED_NAMESPACES`
3. `VERIFY_MODE=live` would create a PVC in a protected namespace

Default protected list:

```text
stellar,stellar-nodes,stellar-system,kube-system,kube-public,kube-node-lease
```

Production is touched **only** with `get`/`list` on `VolumeSnapshot`
objects. The ClusterRole does **not** grant `update`, `patch`, or `delete`
on `StellarNode`, StatefulSets, or production PVCs. Restore always uses a
new PVC in the ephemeral namespace via `spec.dataSource` (see
[Volume Snapshots](../volume-snapshots.md)).

Optional extra guards:

- Set `SNAPSHOT_NAMESPACE` to the namespace that already holds snapshots
- Leave `CrossNamespaceVolumeDataSource` off unless your CSI stack
  supports it; otherwise copy the snapshot into the test namespace first
- `ResourceQuota` in the test namespace caps CPU, memory, and PVC count
- `NetworkPolicy` default-deny ingress; allow DNS + optional DB egress

---

## Snapshot restoration steps

These steps match CSI restore as implemented for Validators
(`restoreFromSnapshot` → PVC `dataSource`) and the verifier's
`BackupSource::VolumeSnapshot`.

1. Confirm the source snapshot is `Ready`:

   ```bash
   kubectl get volumesnapshot -n stellar-nodes
   kubectl get volumesnapshot SNAPSHOT_NAME -n SNAPSHOT_NAMESPACE \
     -o jsonpath='{.status.readyToUse}{"\n"}'
   ```

2. Record the production ledger hash **before** you rely on the snapshot
   (Validator) or row-count baseline (Horizon). Store them in a Secret or
   ConfigMap referenced by the CronJob — not in the manifest.

3. Create an isolated namespace (the CronJob does this):

   ```bash
   kubectl create namespace verify-backup-$(date +%s)
   kubectl label namespace verify-backup-... \
     stellar.org/backup-verification=true \
     stellar.org/ephemeral=true
   ```

4. Create a **new** PVC that restores from the snapshot. Never attach the
   snapshot to a production pod:

   ```yaml
   apiVersion: v1
   kind: PersistentVolumeClaim
   metadata:
     name: restored-data
     namespace: verify-backup-1710000000
   spec:
     accessModes: ["ReadWriteOnce"]
     storageClassName: standard-rwo
     dataSource:
       name: validator-primary-data-20250224-020000
       kind: VolumeSnapshot
       apiGroup: snapshot.storage.k8s.io
     resources:
       requests:
         storage: 500Gi
   ```

5. Start a throwaway workload that mounts `restored-data` (Postgres for
   Horizon, or a Core debug mount for ledger files). Do not set
   `NODE_IS_VALIDATOR=true` and do not register the pod in a production
   quorum set.

6. Run the SQL and/or ledger-hash checks below.

7. Delete the entire namespace. Namespace deletion garbage-collects PVCs,
   pods, Jobs, and ConfigMaps in that namespace.

S3 and pgBackRest sources follow the same isolation rules: restore into
the test namespace only. See
[examples/backup-verification-example.yaml](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/backup-verification-example.yaml)
for the CRD-shaped equivalents.

---

## SQL integrity check templates

These queries are the same families used by
`BackupVerificationScheduler::run_integrity_checks` in
`src/backup/verification.rs`. Run them against the **restored** database
only.

### Connectivity (required)

```sql
SELECT 1;
```

### Table existence (required for Horizon)

```sql
SELECT table_name
FROM information_schema.tables
WHERE table_schema = 'public'
  AND table_name IN (
    'accounts', 'ledgers', 'transactions', 'operations'
  );
```

Fail if any of the four names is missing.

### Row counts (standard / full)

```sql
SELECT COUNT(*) AS ledgers FROM ledgers;
SELECT COUNT(*) AS transactions FROM transactions;
SELECT COUNT(*) AS operations FROM operations;
SELECT COUNT(*) AS accounts FROM accounts;
```

Fail if any required table returns `0` when the source backup is known to
contain history (set `MIN_LEDGER_ROWS`, default `1`).

### Sample / monotonic ledgers (full)

```sql
SELECT sequence, ledger_hash
FROM ledgers
ORDER BY sequence DESC
LIMIT 10;
```

Fail if `sequence` is not strictly decreasing in that result, or if
`ledger_hash` is null/empty.

A ready-to-run `psql` wrapper lives in the CronJob ConfigMap key
`sql-integrity.sh`.

---

## Ledger hash verification (restored volumes)

Validator snapshots store Core state under `/opt/stellar/data` (see
[Quorum Loss Runbook](disaster-recovery.md)). After the PVC is bound,
verify hashes **without** joining production consensus.

### 1. Known-good hash from the snapshot epoch

At snapshot time, capture:

```bash
# Production read-only — Core info endpoint or kubectl stellar logs
curl -sf "http://my-validator.stellar-nodes.svc:11626/info" \
  | jq -r '.info.ledger | "\(.num) \(.hash)"'
```

Store `EXPECTED_LEDGER_SEQ` and `EXPECTED_LEDGER_HASH` in the verifier
Secret. The job compares the restored view to those values.

### 2. Hash files on the restored volume

```bash
# Mounted at /restore (example)
find /restore -type f \( -name '*.xdr' -o -name 'HAS-*' -o -name '*.db' \) \
  | sort \
  | xargs sha256sum \
  > /tmp/restored.sha256

# Optional: compare to a manifest written when the snapshot was taken
sha256sum -c /manifest/ledger-files.sha256
```

Fail if any file listed in the snapshot-time manifest is missing or has a
different digest.

### 3. Live Core `/info` on a catchup-disabled test pod (optional)

If you start `stellar-core` in the test namespace with a non-publishing
config (`NODE_IS_VALIDATOR=false`), poll `/info` and compare
`.info.ledger.hash` to `EXPECTED_LEDGER_HASH`. The same `/info` hash is
what `src/fork_detector` compares across peers; do that compare on the
restored volume only, never by patching a production Validator.

The CronJob ConfigMap key `ledger-hash.sh` implements steps 1–2.

---

## Pass / fail conditions

| Result | When | Job exit | Slack | PagerDuty |
|---|---|---|---|---|
| **PASS** | All required checks passed and cleanup ran | `0` | yes (success) | no |
| **FAIL** | Any required check failed, restore error, timeout, or protected-namespace guard | `1` | yes (failure) | yes (if routing key set) |

Required checks (any failure → **FAIL**):

- Snapshot is `readyToUse=true` (live mode)
- Ephemeral namespace created with verification labels
- PVC reaches `Bound` (live mode)
- SQL: `SELECT 1` and expected tables present (Horizon / `BACKUP_KIND=horizon`)
- SQL: `MIN_LEDGER_ROWS` satisfied when counts run
- Ledger: expected hash matches, or manifest `sha256sum -c` succeeds
  (`BACKUP_KIND=validator` or `both`)
- Cleanup attempted (failure to delete the test namespace is itself a FAIL)

`VERIFY_MODE=dry-run` evaluates configuration and prints a synthetic PASS
report without creating cluster objects. `VERIFY_MODE=fail-fixture` prints
a synthetic FAIL report (safe; no restore). Use those modes to prove
notification wiring.

---

## Reporting and observability

Each run writes a JSON report to stdout and, when
`REPORT_CONFIGMAP` is set, to a ConfigMap in `stellar-backup-verify`
(the controller-owned path can also upload to S3 via `reportStorage`).

```json
{
  "timestamp": "2026-08-29T03:00:00Z",
  "mode": "live",
  "status": "pass",
  "snapshot": "validator-primary-data-20250829-020000",
  "namespace": "verify-backup-1756436400",
  "checks": [
    {"name": "SnapshotReady", "passed": true},
    {"name": "RestorePvcBound", "passed": true},
    {"name": "SqlIntegrity", "passed": true},
    {"name": "LedgerHash", "passed": true},
    {"name": "Cleanup", "passed": true}
  ],
  "durationSeconds": 842
}
```

Watch:

| Signal | How |
|---|---|
| CronJob last schedule / last success | `kubectl get cronjob -n stellar-backup-verify` |
| Job logs | `kubectl logs job/backup-verifier-<id> -n stellar-backup-verify` |
| Operator metrics (CRD path) | `stellar_operator_backup_verifications_total` |
| Orphan namespaces | `kubectl get ns -l stellar.org/backup-verification=true` |
| Ledger-hash divergence (fleet) | `monitoring/fork-detector-alerts.yaml` when that stack is deployed |

Alert if there is **no successful verification in 36 hours** (nightly
schedule + one missed window). That is more important than paging on a
single flake if `backup-verifier-gc` is healthy and the next run is soon;
page immediately on hash mismatch or restore failure.

---

## Slack notifications

Do **not** put webhook URLs in Git. Create a Secret and mount it:

```bash
kubectl create secret generic backup-verifier-notify \
  -n stellar-backup-verify \
  --from-literal=slack-webhook-url='https://hooks.slack.com/services/...'
```

The CronJob reads `SLACK_WEBHOOK_URL` from that key. On completion it
POSTs an Incoming Webhook payload:

```json
{
  "text": "Backup verification PASS",
  "blocks": [
    {
      "type": "section",
      "text": {
        "type": "mrkdwn",
        "text": "*Backup verification PASS*\nsnapshot=`…` namespace=`…`"
      }
    }
  ]
}
```

Both PASS and FAIL send Slack messages so a silent channel means "the job
did not run", not "everything is fine". If the webhook is unset, the job
logs a warning and continues (Slack is best-effort; PagerDuty is the
actionable path for FAIL).

---

## PagerDuty integration

Page only on **actionable FAIL** (restore broken, hash mismatch, SQL
integrity, cleanup failed). Do not page on dry-run or fail-fixture unless
you are testing the integration.

```bash
kubectl create secret generic backup-verifier-notify \
  -n stellar-backup-verify \
  --from-literal=pagerduty-routing-key='R0…'
```

The job sends Events API v2 (`https://events.pagerduty.com/v2/enqueue`)
with `event_action=trigger`, severity `error`, and the report JSON as
custom details. Dedup key:

```text
stellar-k8s/backup-verification/${SNAPSHOT_NAME}
```

so repeated nightly failures update one incident. A later PASS does **not**
auto-resolve unless you set `PAGERDUTY_RESOLVE_ON_PASS=true` (off by
default — a human should confirm the backup is trusted again).

Routing key stays in the Secret. The API URL is the public PagerDuty
endpoint, not a tenant secret.

---

## Credentials and secrets

| Secret | Keys | Used for |
|---|---|---|
| `backup-verifier-notify` | `slack-webhook-url`, `pagerduty-routing-key` | Chat + paging |
| `backup-verifier-expected` | `expected-ledger-hash`, `expected-ledger-seq` | Hash compare |
| CSI / cloud credentials | provider-specific | Only if restore needs object-storage access |

Rules:

- Reference Secrets with `secretKeyRef` / `envFrom`. Never inline tokens.
- RBAC: the verifier ServiceAccount can `get` those Secrets in
  `stellar-backup-verify` only.
- Snapshot source credentials (if any) stay in the **production**
  namespace; the job does not copy them into the test namespace.
- Rotate webhook and routing keys with your normal Secret workflow
  ([Secret Rotation](../secret-rotation.md)).

---

## Failure handling and operator troubleshooting

### Job failed, test namespace still exists

The orchestrator uses `trap cleanup EXIT`. If the kube-apiserver was
unreachable during cleanup, `backup-verifier-gc` deletes namespaces with
`stellar.org/backup-verification=true` older than `ORPHAN_TTL_SECONDS`
(default 6 hours).

```bash
kubectl get ns -l stellar.org/ephemeral=true
kubectl delete ns -l stellar.org/backup-verification=true
```

### Snapshot not Ready

```bash
kubectl describe volumesnapshot -n stellar-nodes SNAPSHOT_NAME
```

Check CSI snapshot-controller logs and the VolumeSnapshotClass. Do not
point the job at a production PVC as a workaround.

### PVC stays Pending

Storage class, volume size (must be ≥ snapshot size), and
cross-namespace data-source support are the usual causes. Confirm the
CronJob `STORAGE_CLASS` and `STORAGE_SIZE` match
[volume-snapshots.md](../volume-snapshots.md).

### SQL checks fail after a Bound PVC

The volume may be Core ledger data, not Postgres. Set `BACKUP_KIND` to
`validator` or `both` only when the snapshot matches that data. For
Horizon, restore the database backup (S3/pgBackRest) as in
`docs/backup-verification.md`, not a Validator PVC.

### Ledger hash mismatch

Treat as **do not promote this snapshot for DR**. Take a new snapshot
from a healthy Validator (`stellar.org/request-snapshot=true`), then
re-run. See [Quorum Loss Runbook](disaster-recovery.md) — do not hand-edit
files under `/opt/stellar/data`.

### Slack/PagerDuty 4xx

Almost always a bad Secret or a revoked webhook. The job still exits
non-zero on FAIL even if notify fails; fix the Secret and re-run.

### Manual one-shot

```bash
kubectl create job -n stellar-backup-verify backup-verifier-manual \
  --from=cronjob/backup-verifier
kubectl logs -n stellar-backup-verify job/backup-verifier-manual -f
```

Set `VERIFY_MODE=dry-run` or `fail-fixture` on the Job to test reporting
without a snapshot.

---

## Deploy and validate

```bash
# 1. Namespace + RBAC + CronJobs (edit SNAPSHOT_* env first)
kubectl apply --dry-run=client -f examples/cronjobs/backup-verifier.yaml
kubectl apply -f examples/cronjobs/backup-verifier.yaml

# 2. Notify secrets (values from your secret manager, not Git)
kubectl create secret generic backup-verifier-notify \
  -n stellar-backup-verify \
  --from-literal=slack-webhook-url="$SLACK_WEBHOOK_URL" \
  --from-literal=pagerduty-routing-key="$PAGERDUTY_ROUTING_KEY"

# 3. Reporting dry-run / fail-fixture (no restore)
kubectl create job -n stellar-backup-verify verify-dry \
  --from=cronjob/backup-verifier
# then: kubectl set env job/verify-dry VERIFY_MODE=dry-run -n stellar-backup-verify
```

Local syntax checks (no cluster required):

```bash
yamllint -c .yamllint.yml examples/cronjobs/backup-verifier.yaml
python3 -c "import yaml,sys; list(yaml.safe_load_all(open(sys.argv[1])))" \
  examples/cronjobs/backup-verifier.yaml
```

---

## References

- [`src/backup/verification.rs`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/backup/verification.rs) — SQL checks, temp namespace, cleanup
- [`src/controller/csi_snapshot.rs`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/csi_snapshot.rs) — CSI VolumeSnapshot create/retain
- [`src/fork_detector/detector.rs`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/fork_detector/detector.rs) — live ledger-hash comparison
- [`examples/cronjobs/backup-verifier.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/cronjobs/backup-verifier.yaml)
- [`examples/backup-verification-example.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/backup-verification-example.yaml)
- [`examples/validator-volume-snapshots.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/validator-volume-snapshots.yaml)
