# Horizon Database High-Availability (HA) Replication Blueprint

With a single PostgreSQL instance behind Horizon, one database failure means an
API outage. This blueprint runs Horizon's database as a **Patroni-managed
PostgreSQL cluster** behind a **PgBouncer pooling tier**. The database survives
the loss of a member, a node, or a network partition, and Horizon needs no
reconfiguration or restart when it fails over. Planned maintenance switchovers
**drop no client requests**.

| File | Purpose |
|---|---|
| [`examples/ha-database/patroni-config.yaml`](../../examples/ha-database/patroni-config.yaml) | 3-member Patroni cluster: RBAC, Patroni config, Services, StatefulSet, PDB |
| [`examples/ha-database/pgbouncer.yaml`](../../examples/ha-database/pgbouncer.yaml) | Stateless PgBouncer tier (2 replicas) in front of the primary |
| [`examples/ha-database/horizon.yaml`](../../examples/ha-database/horizon.yaml) | Horizon `StellarNode` wired to the pooler |
| [`examples/ha-database/Dockerfile`](../../examples/ha-database/Dockerfile) | PostgreSQL 17 + Patroni 4.1.5 image (Patroni publishes no official image) |
| [`examples/ha-database/validation/`](../../examples/ha-database/validation/) | kind cluster, Horizon API tier, traffic probes, and `failover-test.sh` |

Pinned versions: PostgreSQL 17.11, Patroni 4.1.5, PgBouncer 1.26.0
(`ghcr.io/cloudnative-pg/pgbouncer`), Horizon 29.0.0.

---

## Contents

- [1. Architecture](#1-architecture)
- [2. Split-brain protection](#2-split-brain-protection)
- [3. Horizon and PgBouncer compatibility](#3-horizon-and-pgbouncer-compatibility)
- [4. Deploying on Kubernetes](#4-deploying-on-kubernetes)
- [5. Manual switchover without dropping client connections](#5-manual-switchover-without-dropping-client-connections)
- [6. Unplanned failover: what to expect](#6-unplanned-failover-what-to-expect)
- [7. Validation on kind](#7-validation-on-kind)
- [8. Operations reference](#8-operations-reference)
- [9. Production hardening](#9-production-hardening)

---

## 1. Architecture

```mermaid
flowchart TB
    clients["Exchange clients"] --> ingress["Ingress / LB"]
    ingress --> hz["Horizon API (Deployment / StellarNode)<br/>stateless, N replicas"]
    hz -->|"DATABASE_URL<br/>horizon-db-pooler:6432"| pool["PgBouncer (Deployment, 2+ replicas)<br/>transaction pooling"]
    pool -->|"horizon-db:5432<br/>(Service, no selector)"| ep{{"Endpoints horizon-db<br/>= Patroni leader lock"}}
    ep --> p0[("horizon-db-0<br/>PRIMARY")]
    p0 -. "sync streaming" .-> p1[("horizon-db-1<br/>SYNC STANDBY")]
    p0 -. "async streaming" .-> p2[("horizon-db-2<br/>REPLICA")]
    p0 & p1 & p2 <-->|"leader lock, config,<br/>role labels"| api["Kubernetes API (Patroni DCS)"]
```

| Layer | Kind | Stateful? | Scaling / failure unit |
|---|---|---|---|
| Horizon API | Deployment (operator-managed `StellarNode`) | No | Horizontal; any replica can die |
| PgBouncer | Deployment + Service `horizon-db-pooler` | No | Horizontal; clients reconnect to any replica |
| PostgreSQL + Patroni | StatefulSet `horizon-db` (3 members, 1 per node) | Yes (PVC per member) | Tolerates 1 member/node loss |
| Consensus (DCS) | Kubernetes API (Endpoints objects) | Managed by the control plane | No etcd/Consul to operate |

**Services created by `patroni-config.yaml`:**

| Service | Selector | Routes to | Used by |
|---|---|---|---|
| `horizon-db` | *none* | The single address Patroni's leader writes into the `horizon-db` Endpoints | PgBouncer (read-write) |
| `horizon-db-repl` | `role=replica` | Streaming replicas | Optional read scaling (§9) |
| `horizon-db-members` | all members (headless) | Per-member DNS | StatefulSet identity |
| `horizon-db-config` | *none* (headless) | Nothing. It holds Patroni's cluster-config Endpoints | Patroni |

**Why these components:**

- **Patroni with the Kubernetes DCS.** It handles leader election, automatic
  failover, synchronous replication management, `pg_rewind` and switchovers,
  using the Kubernetes API for consensus. There is no extra etcd cluster to run.
- **PgBouncer.** Horizon replicas each keep a Go connection pool. PgBouncer
  multiplexes them onto a bounded number of server connections, and gives
  operators the single choke point (`PAUSE`/`RESUME`) that makes zero-drop
  switchovers possible.
- **Horizon stays stateless.** Its only database coordinate is the pooler DNS
  name, which never changes, so failover never touches Horizon.

---

## 2. Split-brain protection

A split brain happens when two PostgreSQL instances both accept writes, for
example when an old primary is partitioned from the cluster but still reachable
by some clients. The manifests close that window with five independent layers:

| # | Mechanism | Where | What it guarantees |
|---|---|---|---|
| 1 | **Routing *is* the lock.** `kubernetes.use_endpoints: true`: the leader lock is the `horizon-db` Endpoints object, and the lock holder writes its own IP into it in the same atomic update. The `horizon-db` Service has **no selector**. | `patroni-config.yaml` (Patroni config, `horizon-db` Service) | Traffic can only reach the current lock holder. A label-selector Service could keep routing to a partitioned old primary whose `role=primary` label it can no longer remove. This design can't. |
| 2 | **Self-demotion on lost lock.** A primary that can't renew the lock within `ttl` restarts PostgreSQL read-only, which terminates every client session. | `ttl: 30`, `loop_wait: 10`, `retry_timeout: 10` | `loop_wait + 2 × retry_timeout ≤ ttl` (10 + 2×10 = 30): the old primary demotes **before** the lock can expire and a new leader can be elected. |
| 3 | **Synchronous replication.** A commit is acknowledged only after the synchronous standby has it, and only that standby can be promoted. | `synchronous_mode: true`, `synchronous_commit: "on"` | An isolated old primary **can't acknowledge** any write (its sync standby has left or been promoted), so no acknowledged write is ever lost or diverges. |
| 4 | **Rewind before rejoin.** A demoted primary rewinds its data onto the new timeline, or re-clones if rewind fails. | `use_pg_rewind`, `wal_log_hints`, `remove_data_directory_on_*` | A former primary never streams or serves diverged data. |
| 5 | **Fail-safe during control-plane outages.** If the Kubernetes API is unreachable, the primary keeps running only while **every** member confirms over Patroni's REST API that it is still the primary. | `failsafe_mode: true` | A Kubernetes API outage doesn't cause a needless write outage, and it can't create a second primary. |

For clients:

- **Horizon can't be pointed at a stale primary.** It only knows the pooler,
  and the pooler only knows the `horizon-db` Service (layer 1).
- **Half-open connections are detected.** PgBouncer's TCP keepalives
  (`tcp_keepidle=10`, `tcp_keepintvl=5`, `tcp_keepcnt=3`) drop server
  connections to a silently partitioned host within about 25 s. Layers 2 and 3
  already guarantee that nothing written over such a connection is
  acknowledged.
- **`synchronous_mode_strict: false`** keeps the primary writable when *no*
  standby is healthy, at the cost of losing the zero-data-loss guarantee in
  that degraded state. Set it to `true` if you would rather stop accepting
  writes than acknowledge unreplicated ones.
- **Watchdog.** `watchdog.mode: "off"` because kind and most managed node
  pools expose no `/dev/watchdog`. On nodes with `softdog` loaded, set
  `required` to also fence a primary whose Patroni process hangs.

---

## 3. Horizon and PgBouncer compatibility

Transaction pooling (`pool_mode = transaction`) is required for a zero-drop
switchover (§5). Horizon works correctly behind it with two settings. Both come
from Horizon's source code (`github.com/stellar/stellar-horizon`, driver
`github.com/lib/pq`):

| Issue | Cause | Setting |
|---|---|---|
| `unsupported startup parameter: statement_timeout` at connect | Horizon appends `statement_timeout` and `idle_in_transaction_session_timeout` to its DSN (`support/db.augmentDSN`), and lib/pq sends them as startup parameters | PgBouncer: `ignore_startup_parameters = extra_float_digits,statement_timeout,idle_in_transaction_session_timeout`. Horizon still enforces its request timeout on the client side: context cancellation makes lib/pq send a cancel request, which PgBouncer forwards. |
| `unnamed prepared statement does not exist` under load | lib/pq sends a parameterised query as two protocol round trips (Parse, then Bind/Execute). A transaction pooler can hand the second one to a different server connection. | Horizon DSN: `binary_parameters=yes` (a lib/pq option), which sends the query in one round trip |

Other things verified in Horizon's source:

- **Distributed ingestion is compatible.** Horizon elects its single ingesting
  instance with `SELECT … FOR UPDATE` on `key_value_store`, held inside a
  transaction. The lock lives on the primary and is released automatically when
  the primary changes. The new primary's ingester continues from the last
  *committed* ledger.
- **API reads** run in short transactions, which suits transaction pooling.

---

## 4. Deploying on Kubernetes

Prerequisites: a cluster with at least 3 schedulable nodes (one Patroni member
per node is a *required* anti-affinity rule), a default or chosen SSD
StorageClass, and the Stellar-K8s operator for the Horizon `StellarNode`.

```bash
NS=stellar
kubectl create namespace $NS

# 1. Build and publish the Patroni image, then set it in patroni-config.yaml
docker build -t <registry>/stellar-horizon-patroni:17-4.1.5 examples/ha-database
docker push <registry>/stellar-horizon-patroni:17-4.1.5

# 2. Credentials (generated here, never committed)
kubectl -n $NS create secret generic horizon-db-credentials \
  --from-literal=superuser-password="$(openssl rand -hex 24)" \
  --from-literal=replication-password="$(openssl rand -hex 24)" \
  --from-literal=horizon-password="$(openssl rand -hex 24)"

# 3. PostgreSQL + Patroni
kubectl apply -f examples/ha-database/patroni-config.yaml
kubectl -n $NS rollout status statefulset/horizon-db
kubectl -n $NS exec horizon-db-0 -- patronictl -c /etc/patroni/patroni.yml list

# 4. PgBouncer
HZ_PW=$(kubectl -n $NS get secret horizon-db-credentials -o jsonpath='{.data.horizon-password}' | base64 -d)
ADMIN_PW=$(openssl rand -hex 24)
kubectl -n $NS create secret generic horizon-db-pooler-auth \
  --from-literal=userlist.txt="$(printf '"horizon" "%s"\n"pgbouncer_admin" "%s"\n' "$HZ_PW" "$ADMIN_PW")"
kubectl apply -f examples/ha-database/pgbouncer.yaml
kubectl -n $NS rollout status deployment/horizon-db-pooler

# 5. Horizon, pointed at the pooler
kubectl -n $NS create secret generic horizon-db-dsn --from-literal=DATABASE_URL=\
"postgres://horizon:${HZ_PW}@horizon-db-pooler.${NS}.svc.cluster.local:6432/horizon?sslmode=disable&binary_parameters=yes"
kubectl apply -f examples/ha-database/horizon.yaml
```

The Horizon `StellarNode` uses the operator's external-database path
(`spec.database.secretKeyRef`). Don't combine it with `spec.managedDatabase`,
which the operator rejects.

---

## 5. Manual switchover without dropping client connections

Use this for PostgreSQL minor upgrades, node maintenance, or rebalancing.
Horizon clients see a latency spike of a few seconds, but **no failed requests
and no dropped HTTP connections**. PgBouncer holds queries in its queue while
the primary moves.

`PAUSE` must be sent to **every PgBouncer pod** (not through the Service), from
any pod that has `psql`. The Patroni members have it.

```bash
NS=stellar
ADMIN_PW=$(kubectl -n $NS get secret horizon-db-pooler-auth -o jsonpath='{.data.userlist\.txt}' \
  | base64 -d | awk -F'"' '$2 == "pgbouncer_admin" { print $4 }')
pooler() {  # run a PgBouncer admin command on every pooler pod
  for ip in $(kubectl -n $NS get pods -l app.kubernetes.io/name=horizon-db-pooler \
                -o jsonpath='{.items[*].status.podIP}'); do
    kubectl -n $NS exec horizon-db-0 -- psql -XAtq \
      "host=$ip port=6432 dbname=pgbouncer user=pgbouncer_admin password=$ADMIN_PW" -c "$1"
  done
}

# 1. Pre-flight: exactly one Leader, one Sync Standby, all streaming, lag 0
kubectl -n $NS exec horizon-db-0 -- patronictl -c /etc/patroni/patroni.yml list
LEADER=$(kubectl -n $NS get endpoints horizon-db -o jsonpath='{.metadata.annotations.leader}')
CANDIDATE=<the "Sync Standby" member from the list above>

# 2. Hold new queries in PgBouncer. Returns once in-flight transactions have finished.
pooler "PAUSE horizon;"

# 3. Switch over. Patroni demotes the leader cleanly, then promotes the candidate.
kubectl -n $NS exec "$LEADER" -- patronictl -c /etc/patroni/patroni.yml \
  switchover horizon-db --leader "$LEADER" --candidate "$CANDIDATE" --force

# 4. Wait until the primary endpoint points at the candidate
until [ "$(kubectl -n $NS get endpoints horizon-db -o jsonpath='{.metadata.annotations.leader}')" = "$CANDIDATE" ]; do
  sleep 1
done

# 5. Drop idle server connections to the old primary, then release the queue
pooler "RECONNECT horizon;"
pooler "RESUME horizon;"

# 6. Verify: old leader is now a streaming replica on the new timeline
kubectl -n $NS exec "$CANDIDATE" -- patronictl -c /etc/patroni/patroni.yml list
```

Rules:

- Keep the paused window short. Queued queries fail after
  `query_wait_timeout` (60 s), and Horizon's own request timeout is 55 s by
  default. A healthy switchover takes a few seconds.
- If step 3 fails, **run step 5 anyway**. `RESUME` must always follow `PAUSE`.
- Prefer the synchronous standby as the candidate: it is guaranteed to have
  every acknowledged commit.
- If Horizon ingestion is running, it simply waits for its current ledger
  transaction while `PAUSE` drains. No ingestion state is lost.

---

## 6. Unplanned failover: what to expect

| Event | Detection | Writes resume after | Client impact | Data |
|---|---|---|---|---|
| Primary pod deleted or evicted (SIGTERM) | Immediate: Patroni releases the lock on shutdown | Seconds | In-flight transactions error; Horizon returns 5xx only for requests hitting that window | No loss (sync standby promoted) |
| Primary process or node crash, network partition | Lock expiry, up to `ttl` (30 s) | ≈ `ttl` + `loop_wait` (≤ ~40 s) | Queries queue in PgBouncer (up to `query_wait_timeout`) or fail; they recover on their own | No acknowledged loss (layer 3) |
| Replica loss | n/a | No interruption | None (if the sync standby is lost, Patroni picks the other replica) | None |
| Kubernetes API outage | n/a | No interruption | None (`failsafe_mode`) | None |

In every case **Horizon is neither restarted nor reconfigured**: its DSN
points at the pooler, whose upstream Service always resolves to the current
leader.

Tuning: lowering `ttl` shortens crash failover but raises the risk of false
failovers during API-server latency spikes. Keep `loop_wait + 2 × retry_timeout
≤ ttl`.

---

## 7. Validation on kind

[`validation/failover-test.sh`](../../examples/ha-database/validation/failover-test.sh)
deploys the whole stack on a 4-node kind cluster (1 apps node + 3 database
nodes). It runs **Horizon reads** (`GET /ledgers?limit=1` every 200 ms) and
**pooled writes** (an `INSERT` through PgBouncer every 200 ms, recording which
server acknowledged it) continuously, then runs:

| Scenario | Fault | Pass criteria |
|---|---|---|
| 1. Planned switchover | §5 procedure | **0** failed Horizon requests, **0** failed writes |
| 2. Primary pod killed | `kubectl delete pod <primary>` | Replica promoted; Horizon keeps serving without restart |
| 3. Network partition | `docker pause` on the primary's node, then `unpause` | **0** acknowledged writes missing on the final primary; **0** writes acknowledged by the old primary after the partition; old primary rewinds and rejoins |
| All | n/a | Horizon pods never restarted or replaced; cluster healthy at the end |

```bash
cd examples/ha-database/validation
./failover-test.sh                 # create cluster, run, delete cluster
./failover-test.sh --keep          # keep the cluster afterwards for inspection

# Recorded terminal session for the PR
asciinema rec horizon-failover.cast -c ./failover-test.sh
```

Requirements: Docker with about 4 GB of free disk and 6 GB of memory for the
VM, `kind`, `kubectl`, `openssl`, `python3`.

The traffic probes run on the apps node, so pausing a database node never takes
the client path down with it. Writes use `synchronous_commit = on`, so an
acknowledged write means the synchronous standby has it too.

---

## 8. Operations reference

```bash
P="kubectl -n stellar exec horizon-db-0 -- patronictl -c /etc/patroni/patroni.yml"

$P list                                   # members, roles, timeline, lag
$P history                                # timeline / failover history
$P show-config                            # effective DCS configuration
$P edit-config -s 'ttl=30'                # change dynamic config cluster-wide
$P restart horizon-db horizon-db-2        # rolling restart of one member
$P reinit horizon-db horizon-db-2         # re-clone a broken replica from the primary
$P pause  / $P resume                     # maintenance mode: disable automatic failover

# Which member is primary, and where does the Service point?
kubectl -n stellar get endpoints horizon-db -o jsonpath='{.metadata.annotations.leader} {.subsets[0].addresses[0].ip}'

# PgBouncer pool state (run against one pooler pod IP)
psql "host=<pooler-ip> port=6432 dbname=pgbouncer user=pgbouncer_admin" -c 'SHOW POOLS;'
```

**Rolling a PostgreSQL minor version or image:** update the image, delete
replica pods one at a time (waiting for `streaming` in `patronictl list`), run
the §5 switchover, then delete the former primary pod.

**Never force-delete** a Patroni pod (`--force --grace-period=0`) on a node
that may still be running it. The StatefulSet could start a second PostgreSQL
on the same volume before the first has stopped.

---

## 9. Production hardening

- **Storage:** set `storageClassName` in the `volumeClaimTemplates` to a
  low-latency SSD class with `volumeBindingMode: WaitForFirstConsumer`, and size
  it using [capacity planning](../operations/capacity-planning.md#12-horizon-api--ingestion-db).
- **Zones:** add a `topologySpreadConstraints` entry on
  `topology.kubernetes.io/zone` so the three members sit in three zones.
- **Network policy:** restrict port 5432 to PgBouncer and the Patroni members,
  port 8008 to the members, and port 6432 to Horizon. The `pg_hba` rules
  accept any address and rely on this.
- **TLS:** enable `ssl` in the Patroni `parameters` and use `sslmode=verify-full`
  from PgBouncer (`server_tls_sslmode`) and to PgBouncer (`client_tls_*`) if
  your threat model includes in-cluster traffic.
- **Backups and PITR:** HA is not a backup. Add continuous WAL archiving (for
  example pgBackRest or WAL-G via `archive_command`) and test restores. See the
  [disaster recovery runbook](../operations/disaster-recovery.md).
- **Read scaling (optional):** Horizon supports `RO_DATABASE_URL` (a read
  replica, answering with a stale-history error when it lags). Point it at a
  second PgBouncer database entry that targets `horizon-db-repl`. API reads then
  keep working through a primary outage.
- **Monitoring:** alert on the `horizon-db` Endpoints having no address,
  `patronictl list` lag, PgBouncer `cl_waiting` (`SHOW POOLS`), and Horizon
  5xx rates. Patroni exposes Prometheus metrics at `:8008/metrics`.
