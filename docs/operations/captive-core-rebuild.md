# Captive Core State Corruption: Rebuild Guide

**Audience:** On-call operators responding to a Horizon API outage caused by
Captive Core ledger state corruption.  
**Goal:** Safely wipe and rebuild only the ephemeral Captive Core state,
restore Horizon API availability, and avoid any data loss in the Horizon
PostgreSQL database.

> **Related:** [Disaster Recovery](./disaster-recovery.md) ·
> [PVC Troubleshooting](./pvc-troubleshooting.md) ·
> [Backup Verification](./backup-verification.md) ·
> [Operations Index](./index.md)

---

## ⚠️ Critical Safety Warning

**Do NOT delete the Horizon PostgreSQL database.**

The Horizon PostgreSQL database holds all ingested ledger data, account history,
and API state. It is irreplaceable without a full re-ingest from network history
archives (which can take many hours to days). This runbook targets **only** the
ephemeral Captive Core state directory (`/var/lib/stellar` or
`/mnt/stellar-data/captive-core`), which is fully rebuildable from the network.

| Component | Safe to wipe | Rebuild time |
|-----------|:---:|---|
| Captive Core state directory (`/var/lib/stellar`) | ✅ Yes | ~15–60 min catchup |
| Captive Core lock file (`stellar-core.lock`) | ✅ Yes | Immediate |
| Captive Core bucket directory (`buckets/`) | ✅ Yes | Rebuilt during catchup |
| **Horizon PostgreSQL database** | ❌ **NEVER** | Hours to days |
| **Validator seed secrets** | ❌ **NEVER** | Manual re-provision |

---

## Contents

- [What is Captive Core state?](#what-is-captive-core-state)
- [Symptoms of corruption](#symptoms-of-corruption)
- [Diagnostic log patterns](#diagnostic-log-patterns)
- [Prerequisites](#prerequisites)
- [Phase 1: Confirm corruption](#phase-1-confirm-corruption)
- [Phase 2: Freeze the workload](#phase-2-freeze-the-workload)
- [Phase 3: Wipe Captive Core state](#phase-3-wipe-captive-core-state)
- [Phase 4: Restart and verify catchup](#phase-4-restart-and-verify-catchup)
- [Phase 5: Confirm Horizon API health](#phase-5-confirm-horizon-api-health)
- [Post-incident actions](#post-incident-actions)
- [Validation: simulating corruption](#validation-simulating-corruption)

---

## What is Captive Core state?

Horizon uses **Captive Core** to run a Stellar Core process in-process (or as a
subprocess) to ingest ledger data. Captive Core maintains a local on-disk state
directory containing:

- **SQLite databases** — Captive Core metadata and overlay state
- **Bucket list** — compressed ledger state snapshots
- **Lock file** — `stellar-core.lock` prevents multiple Core instances
- **Ledger database** — a local working copy of ledger entries

This state directory is **ephemeral**. Unlike a full Validator's data directory,
Captive Core can rebuild it entirely from network history archives on the next
startup. The Horizon PostgreSQL database, however, contains the full ingested
history and is authoritative — it must never be deleted.

```mermaid
graph LR
    A[Network History Archives] -->|catchup| B[Captive Core State\n/var/lib/stellar]
    B -->|ingest events| C[Horizon PostgreSQL\nHorizon DB]
    C -->|serve| D[Horizon REST API]

    style B fill:#ffe0b2,stroke:#e65100
    style C fill:#e8f5e9,stroke:#2e7d32
    style A fill:#e3f2fd,stroke:#1565c0
```

---

## Symptoms of corruption

The following conditions indicate unrecoverable Captive Core state. All of them
block Horizon API functionality while the Horizon PostgreSQL database remains
intact.

| Symptom | Severity |
|---------|----------|
| Horizon returns HTTP 503 with `"problem": "Service Unavailable"` | High |
| Horizon pods stuck in `CrashLoopBackOff` | High |
| `stellar-core` subprocess exits with non-zero code immediately on start | High |
| Lock file present but no Core process running | Medium |
| Ledger ingest stalls at a fixed sequence number for > 5 minutes | Medium |
| Horizon liveness probe fails repeatedly | Medium |

---

## Diagnostic log patterns

Run these commands to confirm the error is Captive Core corruption and not a
network, quorum, or PostgreSQL problem.

### Check Horizon container logs

```bash
kubectl logs "$POD" -n "$NS" -c horizon --tail=100
```

**Signs of Captive Core lock corruption:**

```text
ERRO[2026-10-01T09:15:42Z] error starting captive core: open /var/lib/stellar/stellar-core.lock: permission denied
ERRO[2026-10-01T09:15:42Z] Failed to start Captive Core subprocess
FATA[2026-10-01T09:15:42Z] cannot start ingestion: captive core exited unexpectedly
```

**Signs of SQLite state corruption:**

```text
ERRO[2026-10-01T09:15:44Z] captive core subprocess error: database disk image is malformed
ERRO[2026-10-01T09:15:44Z] CaptiveCore: stellar-core process exited with status 1
ERRO[2026-10-01T09:15:44Z] ingestion service stopping: unrecoverable captive core error
```

**Signs of bucket list corruption:**

```text
ERRO[2026-10-01T09:15:50Z] captive core: could not apply bucket: bucket hash mismatch
ERRO[2026-10-01T09:15:50Z] CaptiveCore crashed: expected <hash_a>, got <hash_b>
FATA[2026-10-01T09:15:50Z] horizon shutting down: captive core is in an unrecoverable state
```

**Signs of a stale lock (Core crashed mid-write):**

```text
ERRO[2026-10-01T09:15:55Z] stellar-core lock file exists but process is not running
ERRO[2026-10-01T09:15:55Z] remove /var/lib/stellar/stellar-core.lock: operation not permitted
```

### Confirm Horizon PostgreSQL is healthy (it should be)

Before wiping Core state, confirm the Horizon database itself is responsive —
this distinguishes a Captive Core issue from a broader database failure:

```bash
kubectl exec "$POD" -n "$NS" -c horizon -- \
  psql "${DATABASE_URL}" -c "SELECT COUNT(*) FROM history_ledgers;"
```

Expected output (any non-zero count confirms the DB is intact):

```text
  count
---------
 9874532
(1 row)
```

If this query fails, stop here. You have a PostgreSQL problem, not a Captive
Core corruption. See [Disaster Recovery](./disaster-recovery.md) instead.

---

## Prerequisites

Install [`kubectl stellar`](../kubectl-plugin.md) and confirm access:

```bash
kubectl stellar status -n "$NS"
```

Export these variables at the start of your incident session. Replace values
with your actual deployment names:

```bash
export NS=stellar                        # Kubernetes namespace
export HORIZON_DEPLOY=horizon            # Deployment name for the Horizon pod
export POD=$(kubectl get pod -n "$NS" \
  -l "app.kubernetes.io/name=horizon" \
  --field-selector=status.phase=Running \
  -o jsonpath='{.items[0].metadata.name}')
export CAPTIVE_CORE_DIR="/var/lib/stellar"   # or /mnt/stellar-data/captive-core
export DATABASE_URL="postgresql://horizon:REDACTED@horizon-postgres-rw:5432/horizon"
```

> **Tip:** If you are unsure of the Captive Core directory path, check the
> Horizon ConfigMap:
> ```bash
> kubectl get configmap -n "$NS" -l "app.kubernetes.io/name=horizon" \
>   -o jsonpath='{.items[0].data.CAPTIVE_CORE_STORAGE_PATH}'
> ```

---

## Phase 1: Confirm corruption

Exec into the Horizon pod and inspect the Captive Core state directory:

```bash
kubectl exec "$POD" -n "$NS" -c horizon -- ls -lah "${CAPTIVE_CORE_DIR}/"
```

Expected output showing stale state:

```text
total 1.2G
drwxr-xr-x 4 horizon horizon 4.0K Oct  1 09:12 .
drwxr-xr-x 8 root    root    4.0K Oct  1 07:00 ..
drwxr-xr-x 3 horizon horizon 4.0K Oct  1 09:12 buckets
-rw-r--r-- 1 horizon horizon 512M Oct  1 09:11 stellar.db
-rw-r--r-- 1 horizon horizon   11 Oct  1 09:11 stellar-core.lock
-rw-r--r-- 1 horizon horizon 256M Oct  1 09:11 stellar-core-meta.db
```

Check the lock file content:

```bash
kubectl exec "$POD" -n "$NS" -c horizon -- \
  cat "${CAPTIVE_CORE_DIR}/stellar-core.lock"
```

A stale lock from a dead process looks like this (the PID no longer exists):

```text
12847
```

Verify the process is gone:

```bash
kubectl exec "$POD" -n "$NS" -c horizon -- \
  sh -c 'kill -0 $(cat '"${CAPTIVE_CORE_DIR}/stellar-core.lock"') 2>&1 || echo "STALE LOCK: process not found"'
```

Expected output confirming the lock is stale:

```text
STALE LOCK: process not found
```

---

## Phase 2: Freeze the workload

### 2.1 Scale down Horizon to zero replicas

> **Warning:** This takes Horizon offline. Notify users and monitoring teams
> before proceeding. Typically takes 30–90 seconds for in-flight requests to
> drain.

```bash
kubectl scale deployment "$HORIZON_DEPLOY" -n "$NS" --replicas=0
kubectl wait deployment "$HORIZON_DEPLOY" -n "$NS" \
  --for=jsonpath='{.status.availableReplicas}'=0 \
  --timeout=120s
```

Expected output:

```text
deployment.apps/horizon scaled
deployment.apps/horizon condition met
```

Confirm all Horizon pods are gone:

```bash
kubectl get pod -n "$NS" -l "app.kubernetes.io/name=horizon"
```

Expected output:

```text
No resources found in stellar namespace.
```

### 2.2 If managed by StellarNode CRD, enable maintenance mode

If your Horizon instance is managed by the Stellar-K8s operator via a
`StellarNode` resource, place it in maintenance mode to prevent the operator
from interfering with the scale-down:

```bash
kubectl patch stellarnode horizon -n "$NS" --type=merge \
  -p '{"spec":{"maintenanceMode":true}}'
```

Wait for the status to reflect maintenance:

```bash
kubectl get stellarnode horizon -n "$NS" -o jsonpath='{.status.phase}'
```

Expected output:

```text
Maintenance
```

---

## Phase 3: Wipe Captive Core state

Run a one-shot maintenance pod to safely delete only the Captive Core state
directory. **Do not touch the PostgreSQL database.**

### Option A: Using the automated reset script (recommended)

See [`examples/troubleshooting/reset-captive-core.sh`](../../examples/troubleshooting/reset-captive-core.sh)
for the fully automated, idempotent version of these steps.

```bash
NS="$NS" HORIZON_DEPLOY="$HORIZON_DEPLOY" \
  CAPTIVE_CORE_DIR="$CAPTIVE_CORE_DIR" \
  bash examples/troubleshooting/reset-captive-core.sh
```

### Option B: Manual kubectl exec steps

If the pod is still running (e.g., in a `CrashLoopBackOff` restart window),
exec in during a brief window:

```bash
kubectl exec "$POD" -n "$NS" -c horizon -- \
  sh -c "rm -rf ${CAPTIVE_CORE_DIR:?}/ && echo 'Captive Core state wiped'"
```

Expected output:

```text
Captive Core state wiped
```

### Option C: Maintenance pod approach (for terminated/ImagePullBackOff pods)

If the Horizon pod cannot be exec'd into, use a maintenance pod that mounts the
same PVC. Apply the debug pod from
[`examples/debug/maintenance-pod.yaml`](../../examples/debug/maintenance-pod.yaml)
with the Horizon PVC name substituted, then:

```bash
kubectl exec stellar-maintenance-debug -n "$NS" -- \
  sh -c "rm -rf ${CAPTIVE_CORE_DIR:?}/ && mkdir -p ${CAPTIVE_CORE_DIR} && echo 'Done'"
```

After wiping, delete the maintenance pod:

```bash
kubectl delete pod stellar-maintenance-debug -n "$NS"
```

### Verify the state directory is empty

Regardless of which option was used, confirm the directory is clean:

```bash
# Check via a new exec if pod is still up, or inspect via a fresh pod
kubectl run verify-core-wipe --rm -i --image=busybox --restart=Never -n "$NS" \
  --overrides='{"spec":{"volumes":[{"name":"d","persistentVolumeClaim":{"claimName":"horizon-data"}}],"containers":[{"name":"v","image":"busybox","command":["sh","-c","ls -la /data/captive-core/ 2>/dev/null || echo EMPTY"],"volumeMounts":[{"name":"d","mountPath":"/data"}]}]}}' \
  -- sh
```

Expected output (directory should be empty or show only the newly created empty dir):

```text
EMPTY
```

---

## Phase 4: Restart and verify catchup

### 4.1 Scale Horizon back up

```bash
kubectl scale deployment "$HORIZON_DEPLOY" -n "$NS" --replicas=1
kubectl rollout status deployment "$HORIZON_DEPLOY" -n "$NS" --timeout=300s
```

Expected output:

```text
Waiting for deployment "horizon" rollout to finish: 0 of 1 updated replicas are available...
deployment "horizon" successfully rolled out
```

If using the StellarNode CRD, disable maintenance mode first:

```bash
kubectl patch stellarnode horizon -n "$NS" --type=merge \
  -p '{"spec":{"maintenanceMode":false}}'
```

### 4.2 Confirm Captive Core catchup has started

Watch the Horizon logs for the catchup sequence:

```bash
kubectl logs -f deployment/"$HORIZON_DEPLOY" -n "$NS" -c horizon | \
  grep -E "(captive|catchup|ledger|ingest)"
```

**Expected log sequence — a healthy Captive Core rebuild:**

```text
INFO[2026-10-01T09:32:01Z] Starting Captive Core subprocess
INFO[2026-10-01T09:32:02Z] Captive Core starting with fresh state directory
INFO[2026-10-01T09:32:05Z] stellar-core: Connecting to network peers
INFO[2026-10-01T09:32:10Z] stellar-core: Received SCP messages, beginning catchup
INFO[2026-10-01T09:32:12Z] stellar-core: catchup mode: CATCHUP_RECENT
INFO[2026-10-01T09:32:15Z] stellar-core: Downloading checkpoints from history archive
INFO[2026-10-01T09:32:45Z] stellar-core: Applying buckets from checkpoint ledger 9874496
INFO[2026-10-01T09:33:30Z] stellar-core: Replaying ledgers 9874496 -> 9874560
INFO[2026-10-01T09:34:01Z] stellar-core: Ledger 9874560 closed, catching up to network
INFO[2026-10-01T09:35:22Z] Captive Core is synced at ledger 9874575
INFO[2026-10-01T09:35:22Z] Starting ledger ingestion from captive core
INFO[2026-10-01T09:35:23Z] Ingesting ledger: 9874576
INFO[2026-10-01T09:35:24Z] Ingesting ledger: 9874577
```

**If you see this error after restart, the directory was not fully wiped:**

```text
ERRO[2026-10-01T09:32:02Z] stellar-core lock file exists, captive core cannot start
```

Repeat Phase 3 to ensure the lock file is removed.

### 4.3 Monitor catchup progress

The catchup typically takes 15–60 minutes depending on network speed and the
distance from the latest checkpoint. Track progress:

```bash
kubectl exec deployment/"$HORIZON_DEPLOY" -n "$NS" -c horizon -- \
  wget -qO- http://localhost:8000/metrics | grep -E "horizon_ingest_ledger_ingestion_duration"
```

Or check the Horizon status endpoint:

```bash
kubectl exec deployment/"$HORIZON_DEPLOY" -n "$NS" -c horizon -- \
  wget -qO- http://localhost:8000/ | python3 -m json.tool | grep -E "(state|ledger)"
```

Expected output while catching up:

```json
{
  "state": "syncing",
  "current_protocol_version": 21,
  "core_latest_ledger": 9874599,
  "history_latest_ledger": 9874575,
  "ingested_latest_ledger": 9874530
}
```

Expected output when fully synced:

```json
{
  "state": "synced",
  "current_protocol_version": 21,
  "core_latest_ledger": 9874605,
  "history_latest_ledger": 9874605,
  "ingested_latest_ledger": 9874604
}
```

---

## Phase 5: Confirm Horizon API health

### 5.1 Check liveness and readiness probes

```bash
kubectl get pod -n "$NS" -l "app.kubernetes.io/name=horizon" -o wide
```

Expected output (READY `1/1`):

```text
NAME                       READY   STATUS    RESTARTS   AGE
horizon-7d6b8f5c9-xk4qz   1/1     Running   0          8m
```

### 5.2 Verify API responses

```bash
HORIZON_POD=$(kubectl get pod -n "$NS" -l "app.kubernetes.io/name=horizon" \
  -o jsonpath='{.items[0].metadata.name}')

# Check root endpoint returns 200
kubectl exec "$HORIZON_POD" -n "$NS" -c horizon -- \
  wget -qO- --server-response http://localhost:8000/ 2>&1 | head -5
```

Expected output:

```text
  HTTP/1.1 200 OK
  Content-Type: application/hal+json; charset=utf-8
```

### 5.3 Confirm the Horizon database was not touched

Verify the ledger count in PostgreSQL is unchanged from Phase 1:

```bash
kubectl exec "$HORIZON_POD" -n "$NS" -c horizon -- \
  psql "${DATABASE_URL}" -c "SELECT COUNT(*), MAX(sequence) FROM history_ledgers;"
```

Expected output (count should be same or higher — never lower — than before the rebuild):

```text
  count   |    max
----------+---------
 9874605  | 9874605
(1 row)
```

### 5.4 Run the kubectl stellar status check

```bash
kubectl stellar status -n "$NS"
```

Expected output:

```text
NAME      TYPE     NETWORK   STATE    LEDGER    AGE
horizon   Horizon  mainnet   Synced   9874605   12m
```

---

## Post-incident actions

1. **File an incident report** using the
   [post-mortem template](../incident-response/post-mortem.md). Document:
   - Approximate time of the original Captive Core crash
   - Duration of Horizon API unavailability
   - Ledger gap (if any) in the `history_ledgers` table
   - Root cause (disk I/O error, OOM kill, interrupted write, etc.)

2. **Check for recurring disk I/O issues** that may have caused the crash:
   ```bash
   kubectl describe pod "$POD" -n "$NS" | grep -A5 "OOMKilled\|Error\|Reason"
   dmesg | grep -i "i/o error\|ext4\|xfs" | tail -20  # from the node
   ```

3. **Review PVC IOPS capacity.** Captive Core is I/O intensive. If the crash
   was due to a slow disk, see [Proactive Disk Scaling](../proactive-disk-scaling.md).

4. **Consider adding a Captive Core crash alert.** Add a Prometheus alert rule
   that fires when Horizon's `horizon_ingest_captive_core_up` metric drops to 0:
   ```yaml
   - alert: CaptiveCoreDown
     expr: horizon_ingest_captive_core_up == 0
     for: 2m
     labels:
       severity: critical
     annotations:
       summary: "Captive Core subprocess is not running"
       description: "Horizon {{ $labels.namespace }}/{{ $labels.pod }} has no active Captive Core process. API is degraded."
   ```

5. **Re-enable any suspended StellarNode resources:**
   ```bash
   kubectl patch stellarnode horizon -n "$NS" --type=merge \
     -p '{"spec":{"maintenanceMode":false}}'
   ```

---

## Validation: simulating corruption

The issue description requests that this guide be validated by manually
simulating Captive Core corruption. Follow these steps in a non-production
environment only.

### Step 1: Corrupt the Captive Core SQLite database

```bash
HORIZON_POD=$(kubectl get pod -n "$NS" -l "app.kubernetes.io/name=horizon" \
  -o jsonpath='{.items[0].metadata.name}')

# Write random bytes to the SQLite header to corrupt the database
kubectl exec "$HORIZON_POD" -n "$NS" -c horizon -- \
  sh -c "dd if=/dev/urandom bs=16 count=1 of=${CAPTIVE_CORE_DIR}/stellar.db conv=notrunc 2>&1"
```

Expected output:

```text
1+0 records in
1+0 records out
16 bytes copied, ...
```

### Step 2: Restart Horizon and observe corruption errors

```bash
kubectl rollout restart deployment/"$HORIZON_DEPLOY" -n "$NS"
kubectl logs -f deployment/"$HORIZON_DEPLOY" -n "$NS" -c horizon | \
  grep -E "(ERROR|FATAL|malformed|captive)" | head -20
```

You should observe logs matching the [diagnostic patterns](#diagnostic-log-patterns)
described above.

### Step 3: Execute this runbook to restore

Follow Phases 1–5 of this guide. Verify that:

- Horizon returns to `state: synced` in the root endpoint
- The `history_ledgers` row count matches the pre-corruption value
- No ledger sequence gaps exist in `history_ledgers`

```bash
kubectl exec "$HORIZON_POD" -n "$NS" -c horizon -- \
  psql "${DATABASE_URL}" -c \
  "SELECT sequence FROM history_ledgers ORDER BY sequence LIMIT 10;"
```
