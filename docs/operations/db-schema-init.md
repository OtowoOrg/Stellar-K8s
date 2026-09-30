# Database Schema Initialization

Stellar Core stores ledger, SCP, and bucket metadata in an embedded SQLite
database that lives on the validator's data volume. A **fresh persistent
volume contains an empty filesystem — no database file and no schema**. Before
Stellar Core can boot against a new volume, the schema must be created with the
`stellar-core new-db` one-off initialization command. This step appears nowhere
in the container's normal startup path, so it must be run explicitly.

This page documents when initialization is required, how to run it (as a
one-off pod or an init container), and what happens when it is skipped.

---

## Quick reference

| Question | Answer |
|---|---|
| Initialization command | `stellar-core new-db --conf /config/stellar-core.cfg` |
| Required on | Fresh (empty) PVC, or after a full local-state reset |
| Skipped when | Restored volume, attached volume with existing ledger state |
| Data directory | `/opt/stellar/data` (validator data PVC) |
| Config file | `/config/stellar-core.cfg` (operator-generated ConfigMap) |
| Container UID | `10000` (files on the volume must be owned by uid/gid 10000) |
| Symptom if skipped | `No DB schema version found, try stellar-core new-db` |

---

## When initialization is required vs. skipped

### Required: fresh (empty) volume

Run the initialization pass when the data volume is **empty**, which is the
case for:

- A newly provisioned PVC (first deployment of the validator).
- A PVC recreated after deletion (e.g. storage was deleted with the node).
- A volume that was fully reset during storage repair (the
  `stellar-core new-db` step in the
  [PVC corruption playbook](./pvc-troubleshooting.md) already initializes it —
  do not run the procedure on this page on top of it).

### Not required: volumes that already contain ledger state

Skip initialization — and **never run `new-db`**, because it resets local
state — when the volume already holds a usable database:

- A volume restored from a snapshot or backup
  ([Volume Snapshots](../volume-snapshots.md)); the snapshot already contains a
  schema-initialized database.
- An attached volume from a replaced or rescheduled pod (StatefulSet storage
  follows the pod).
- A volume handed over during a blue/green swap
  ([Core Blue/Green Runbook](./core-blue-green-runbook.md)).

> **Warning**
> `stellar-core new-db` **initializes or resets** the database and bucket
> state. Running it against a volume with existing ledger state destroys that
> state. Only run it on volumes you know are empty or are intentionally being
> reset, and never against a volume a live core process is using.

---

## Failure symptom when initialization is skipped

If Stellar Core starts against an empty data volume, it exits shortly after
boot with a schema error similar to:

```text
error: No DB schema version found, try stellar-core new-db
```

The pod then enters `CrashLoopBackOff`. The same symptom is cataloged in the
[Common Issues troubleshooting guide](../troubleshooting/common-issues.md#issue-15-validator-crashloops-with-no-db-schema-version-found-on-a-fresh-volume).
Check for it with:

```bash
kubectl logs <validator-pod> -n <namespace> -c stellar-node | grep -i "schema version"
```

If you see that message on a node whose PVC was *supposed* to be fresh, run the
initialization procedure below. If you see it on a node that previously synced,
treat it as storage damage instead — see the
[PVC corruption playbook](./pvc-troubleshooting.md) before re-initializing,
since `new-db` on a partially damaged volume discards recoverable state.

---

## Procedure: one-off initialization pod

Use a detached one-off pod when you are initializing an existing PVC outside of
deployment (e.g. pre-provisioned storage) or repairing a volume. For
per-deployment automation, prefer the init container variant below.

> **Prerequisite:** the validator's main container must not be running against
> the volume. Scale/suspend the `StellarNode` workload or delete the validator
> pod first, exactly as in the
> [maintenance pod procedure](./pvc-troubleshooting.md#phase-1-stop-writes-and-preserve-evidence).

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: stellar-core-db-init
  namespace: stellar            # match your validator namespace
  labels:
    app.kubernetes.io/name: stellar-core-db-init
    app.kubernetes.io/part-of: stellar-k8s
spec:
  restartPolicy: Never
  securityContext:
    # The main validator containers run as uid/gid 10000 (Pod Security
    # Standards `restricted`, see docs/security/pss.md). The data directory
    # and the files new-db creates must be owned by the same uid, or the
    # validator pod will fail to open its database on first boot.
    runAsNonRoot: true
    runAsUser: 10000
    runAsGroup: 10000
    fsGroup: 10000
    fsGroupChangePolicy: OnRootMismatch
    seccompProfile:
      type: RuntimeDefault
  containers:
    - name: db-init
      image: docker.io/stellar/stellar-core:v21.3.0   # match your validator image/version
      command: ["/usr/bin/stellar-core", "new-db", "--conf", "/config/stellar-core.cfg"]
      volumeMounts:
        - name: data
          mountPath: /opt/stellar/data     # operator mounts the data PVC here
        - name: config
          mountPath: /config
          readOnly: true
      resources:
        requests:
          cpu: 100m
          memory: 256Mi
        limits:
          cpu: "1"
          memory: 1Gi
  volumes:
    - name: data
      persistentVolumeClaim:
        claimName: <node-name>-data        # the validator's data PVC
    - name: config
      configMap:
        name: <node-name>-config           # operator-generated stellar-core.cfg
```

Apply it and wait for completion:

```bash
kubectl apply -f db-init-pod.yaml
kubectl wait --for=condition=Ready pod/stellar-core-db-init -n stellar --timeout=120s
kubectl logs -n stellar stellar-core-db-init
```

Expected output ends with an initialized database, e.g.:

```text
Connected to DB
Application startup
* initialized local database /opt/stellar/data/stellar.db
```

Then delete the one-off pod and start the validator workload:

```bash
kubectl delete pod -n stellar stellar-core-db-init
```

---

## Procedure: automatic init container

The `StellarNode` CRD supports `spec.initContainers`, which run to completion
before the main `stellar-node` container starts. Use this to make fresh-volume
initialization automatic on every deployment:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-primary
  namespace: stellar
spec:
  nodeType: Validator
  network: testnet
  # ... existing storage/config fields ...

  initContainers:
    - name: db-schema-init
      image: docker.io/stellar/stellar-core:v21.3.0
      command:
        - /bin/sh
        - -c
        - |
          # Only initialize when the data directory is empty (fresh volume).
          # Any existing database means the volume is already initialized or
          # restored — running new-db here would reset ledger state.
          if [ ! -f /opt/stellar/data/stellar.db ]; then
            echo "fresh volume detected - running stellar-core new-db"
            stellar-core new-db --conf /config/stellar-core.cfg
          else
            echo "existing database found - skipping schema initialization"
          fi
      volumeMounts:
        - name: data
          mountPath: /opt/stellar/data
        - name: config
          mountPath: /config
          readOnly: true
```

Notes:

- The `data` and `config` volume names match the volumes the operator attaches
  to every node pod; user init containers can mount them by the same names.
- The container inherits the pod security context (uid 10000), so files created
  by `new-db` are automatically owned correctly.
- The guard (`[ ! -f .../stellar.db ]`) keeps the pod startable on volumes that
  are already initialized, so the same manifest is safe to leave in place.

!!! danger "Embedded SQLite only"
    The file-existence guard above is only valid for the default embedded
    SQLite database on the data PVC. If your validator uses an external or
    managed PostgreSQL database (`spec.database` / `spec.managedDatabase`), its
    schema lives in PostgreSQL — the guard will always see an empty data
    directory and run `new-db` on every boot, which **resets the remote
    database**. For PostgreSQL-backed validators, initialize once with the
    one-off pod procedure instead, or gate the init container on a
    one-time flag (e.g. a marker key in a ConfigMap).

!!! warning "Keep the image version in sync"
    Use the same Stellar Core image (and version) as the validator container.
    Schema versions differ between core releases; initializing with a mismatched
    version can leave the database in a state the main container cannot use.

---

## File ownership on the data directory

All Stellar-K8s workloads run non-root as **uid/gid 10000** (the operator sets
`runAsUser`, `runAsGroup`, and `fsGroup: 10000`; see
[Pod Security Standards](../security/pss.md)). Therefore:

- The data directory and the database/bucket files `new-db` creates **must be
  owned by uid/gid 10000**.
- If you run the one-off pod manifest from this page, the `fsGroup: 10000` +
  `fsGroupChangePolicy: OnRootMismatch` settings make Kubernetes fix ownership
  automatically at mount time.
- If you attach the PVC to a debug/maintenance pod with different uid settings
  (e.g. a generic debug pod mounted with a different `fsGroup`, see
  [Phase 2 of the maintenance pod procedure](./pvc-troubleshooting.md#phase-2-attach-a-maintenance-pod)),
  verify and correct ownership before handing the volume back to the validator:

```bash
# From a pod that has the volume mounted:
chown -R 10000:10000 /opt/stellar/data
find /opt/stellar/data -type d -exec chmod 750 {} +
find /opt/stellar/data -type f -exec chmod 640 {} +
```

Incorrect ownership produces a different startup failure — core cannot create
or open `stellar.db` — so double-check it whenever a fresh volume was touched
by any pod other than the validator itself.

---

## Validation

After initialization and first boot, confirm the validator reaches SCP and
closes ledgers without schema errors:

```bash
# Schema errors should return nothing
kubectl logs <validator-pod> -n <namespace> -c stellar-node | grep -i "schema version"

# Node should leave Catching up / join SCP (see the validator runbook)
kubectl stellar status <node-name> -n <namespace>
```

For a deeper check, use the offline commands from the
[PVC troubleshooting playbook](./pvc-troubleshooting.md#phase-3-diagnose-stellar-core-sqlite-corruption):

```bash
stellar-core offline-info --conf /config/stellar-core.cfg
```

---

## Related documentation

- [PVC Corruption and Storage Recovery Playbook](./pvc-troubleshooting.md) —
  recovery procedures (including `new-db` as a **reset** step) and the
  maintenance pod used above
- [Quorum Loss Runbook](./disaster-recovery.md) — local-state reset and replay
- [Volume Snapshots](../volume-snapshots.md) — snapshot restore behavior
- [Pod Security Standards](../security/pss.md) — why uid 10000
- [Common Issues](../troubleshooting/common-issues.md) — symptom catalog
