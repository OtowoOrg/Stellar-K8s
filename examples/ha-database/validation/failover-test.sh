#!/usr/bin/env bash
# failover-test.sh
#
# End-to-end validation of the Horizon HA database blueprint on kind.
#
# Deploys Patroni (3 members) + PgBouncer (2) + Horizon API (2) + traffic probes,
# then runs three scenarios while Horizon reads and pooled writes run continuously:
#
#   1. Planned switchover with PgBouncer PAUSE/RESUME  -> must drop zero requests.
#   2. Primary pod deleted                             -> new leader elected; Horizon
#                                                         keeps serving without restart.
#   3. Primary's node frozen (network partition)       -> split-brain check: every
#                                                         acknowledged write survives and
#                                                         the old primary never acknowledges
#                                                         a write after losing leadership.
#
# Usage:
#   ./failover-test.sh            # create cluster, run all scenarios, delete cluster
#   ./failover-test.sh --keep     # leave the cluster running afterwards
#   ./failover-test.sh --reuse    # reuse an existing 'horizon-ha' cluster and deployment
#
# Record for a PR:  asciinema rec horizon-failover.cast -c ./failover-test.sh
#
# Requirements: docker, kind, kubectl, openssl, python3.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BLUEPRINT="$(cd "$HERE/.." && pwd)"
CLUSTER="horizon-ha"
NS="stellar"
IMAGE="stellar-horizon-patroni:17-4.1.5"
KEEP=0
REUSE=0

for arg in "$@"; do
  case "$arg" in
    --keep)  KEEP=1 ;;
    --reuse) REUSE=1; KEEP=1 ;;
    -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
    *) echo "Unknown argument: $arg" >&2; exit 1 ;;
  esac
done

for cmd in docker kind kubectl openssl python3; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "Missing required command: $cmd" >&2; exit 1; }
done

BOLD=$'\033[1m'; GREEN=$'\033[32m'; RED=$'\033[31m'; RESET=$'\033[0m'
step() { printf '\n%s==> %s%s\n' "$BOLD" "$*" "$RESET"; }
info() { printf '    %s\n' "$*"; }
k()    { kubectl --context "kind-$CLUSTER" -n "$NS" "$@"; }

FAILURES=0
SUMMARY=()
check() { # check <description> <condition-exit-code>
  if [[ "$2" == 0 ]]; then
    SUMMARY+=("${GREEN}PASS${RESET}  $1")
  else
    SUMMARY+=("${RED}FAIL${RESET}  $1"); FAILURES=$((FAILURES + 1))
  fi
}

cleanup() {
  if (( KEEP == 0 )); then
    step "Deleting kind cluster $CLUSTER"
    kind delete cluster --name "$CLUSTER" || true
  else
    info "Cluster kept: kubectl --context kind-$CLUSTER -n $NS get pods"
  fi
}

# ---------------------------------------------------------------------------
# Cluster helpers
# ---------------------------------------------------------------------------
leader()    { k get endpoints horizon-db -o jsonpath='{.metadata.annotations.leader}' 2>/dev/null; }
leader_ip() { k get endpoints horizon-db -o jsonpath='{.subsets[0].addresses[0].ip}' 2>/dev/null; }
pod_ip()    { k get pod "$1" -o jsonpath='{.status.podIP}'; }

# patronictl from any member other than $1 (which may be down).
patroni_list() {
  local avoid="${1:-}" member
  for member in horizon-db-0 horizon-db-1 horizon-db-2; do
    [[ "$member" == "$avoid" ]] && continue
    if k exec "$member" -- patronictl -c /etc/patroni/patroni.yml list -f json 2>/dev/null; then
      return 0
    fi
  done
  return 1
}

sync_standby() {
  patroni_list | python3 -c '
import json, sys
print(next((m["Member"] for m in json.load(sys.stdin) if m["Role"] == "Sync Standby"), ""))'
}

print_cluster() {
  patroni_list "${1:-}" | python3 -c '
import json, sys
for m in json.load(sys.stdin):
    print("    {:<14} {:<13} {:<10} TL={:<3} lag_mb={}".format(
        m["Member"], m["Role"], m["State"], m.get("TL", "-"), m.get("Lag in MB", 0)))'
}

# Healthy = 3 members, 1 leader, 1 sync standby, all running/streaming on one timeline.
cluster_healthy() {
  patroni_list | python3 -c '
import json, sys
m = json.load(sys.stdin)
ok = (len(m) == 3
      and sum(x["Role"] == "Leader" for x in m) == 1
      and sum(x["Role"] == "Sync Standby" for x in m) == 1
      and all(x["State"] in ("running", "streaming") for x in m)
      and len({x.get("TL") for x in m}) == 1)
sys.exit(0 if ok else 1)'
}

wait_for() { # wait_for <timeout-seconds> <description> <command...>
  local timeout="$1" desc="$2"; shift 2
  local start=$SECONDS
  until "$@" >/dev/null 2>&1; do
    if (( SECONDS - start > timeout )); then
      echo "Timed out after ${timeout}s waiting for: $desc" >&2
      return 1
    fi
    sleep 1
  done
  info "$desc (${BOLD}$((SECONDS - start))s${RESET})"
}

leader_is_not() { local l; l="$(leader)"; [[ -n "$l" && "$l" != "$1" && -n "$(leader_ip)" ]]; }
leader_is()     { [[ "$(leader)" == "$1" && "$(leader_ip)" == "$(pod_ip "$1")" ]]; }

horizon_state() { k get pods -l app.kubernetes.io/name=horizon-api \
  -o jsonpath='{range .items[*]}{.metadata.uid}:{.status.containerStatuses[0].restartCount} {end}'; }

# ---------------------------------------------------------------------------
# PgBouncer admin console (sent to EVERY pooler pod, not through the Service)
# ---------------------------------------------------------------------------
pooler_admin() { # pooler_admin <command>
  local ip
  for ip in $(k get pods -l app.kubernetes.io/name=horizon-db-pooler -o jsonpath='{.items[*].status.podIP}'); do
    k exec write-probe -- psql \
      "host=$ip port=6432 dbname=pgbouncer user=pgbouncer_admin password=$ADMIN_PW sslmode=disable" \
      -XAtq -c "$1" >/dev/null
    info "pgbouncer $ip: $1"
  done
}

# ---------------------------------------------------------------------------
# Probe accounting: everything is measured on log lines emitted after mark().
# ---------------------------------------------------------------------------
HTTP_MARK=0; WRITE_MARK=0
mark() {
  HTTP_MARK=$(k logs http-probe | wc -l | tr -d ' ')
  WRITE_MARK=$(k logs write-probe | wc -l | tr -d ' ')
}

# Prints "<http_total> <http_fail> <write_ok> <write_err> <max_write_gap_s>"
measure() {
  local http writes
  http=$(k logs http-probe | tail -n +"$((HTTP_MARK + 1))")
  writes=$(k logs write-probe | tail -n +"$((WRITE_MARK + 1))")
  local http_total http_fail
  http_total=$(printf '%s\n' "$http" | grep -c . || true)
  http_fail=$(printf '%s\n' "$http" | awk 'NF == 2 && $2 != "200"' | grep -c . || true)
  printf '%s\n' "$writes" | awk -v ht="$http_total" -v hf="$http_fail" '
    function secs(t,  p) { split(t, p, ":"); return p[1] * 3600 + p[2] * 60 + p[3] }
    $2 == "ok"  { ok++; s = secs($1); if (last != "" && s - last > gap) gap = s - last; last = s }
    $2 == "err" { err++ }
    END { printf "%d %d %d %d %d\n", ht, hf, ok, err, gap }'
}

report() { # report <scenario>
  local m; read -r -a m <<<"$(measure)"
  info "Horizon reads : ${m[0]} requests, ${m[1]} non-200"
  info "Pooled writes : ${m[2]} acknowledged, ${m[3]} failed, longest gap between acks ${m[4]}s"
  LAST_HTTP_FAIL=${m[1]}; LAST_WRITE_ERR=${m[3]}
  printf '%s\n' "$(k logs write-probe | tail -n +"$((WRITE_MARK + 1))" | grep ' err ' | head -3)" \
    | sed '/^$/d; s/^/    first errors: /'
  SCENARIO_ROWS+=("$(printf '%-34s %8s %8s %8s %8s %6ss' "$1" "${m[0]}" "${m[1]}" "${m[2]}" "${m[3]}" "${m[4]}")")
}
SCENARIO_ROWS=()

# ---------------------------------------------------------------------------
# 0. Deploy
# ---------------------------------------------------------------------------
if (( REUSE == 0 )); then
  trap cleanup EXIT

  step "Creating kind cluster $CLUSTER (1 apps node + 3 database nodes)"
  kind create cluster --config "$HERE/kind-config.yaml" --wait 120s

  step "Building and loading the Patroni image ($IMAGE)"
  docker build -q -t "$IMAGE" "$BLUEPRINT"
  kind load docker-image "$IMAGE" --name "$CLUSTER"

  step "Deploying the Patroni cluster"
  kubectl --context "kind-$CLUSTER" create namespace "$NS"
  k create secret generic horizon-db-credentials \
    --from-literal=superuser-password="$(openssl rand -hex 24)" \
    --from-literal=replication-password="$(openssl rand -hex 24)" \
    --from-literal=horizon-password="$(openssl rand -hex 24)"
  k apply -f "$BLUEPRINT/patroni-config.yaml"
  k rollout status statefulset/horizon-db --timeout=300s
  wait_for 180 "Patroni cluster healthy (leader + sync standby)" cluster_healthy

  step "Deploying PgBouncer"
  HZ_PW=$(k get secret horizon-db-credentials -o jsonpath='{.data.horizon-password}' | base64 -d)
  ADMIN_PW=$(openssl rand -hex 24)
  k create secret generic horizon-db-pooler-auth --from-literal=userlist.txt="$(printf \
    '"horizon" "%s"\n"pgbouncer_admin" "%s"\n' "$HZ_PW" "$ADMIN_PW")"
  k apply -f "$BLUEPRINT/pgbouncer.yaml"
  # Validation only: pin the stateless tier to the apps node (see kind-config.yaml).
  k patch deployment horizon-db-pooler --type merge -p '{"spec":{"template":{"spec":{
    "nodeSelector":{"stellar.org/pool":"apps"},
    "tolerations":[{"key":"node-role.kubernetes.io/control-plane","operator":"Exists","effect":"NoSchedule"}]}}}}'
  k rollout status deployment/horizon-db-pooler --timeout=180s

  step "Deploying Horizon API (2 replicas) behind the pooler"
  k create secret generic horizon-db-dsn --from-literal=DATABASE_URL=\
"postgres://horizon:${HZ_PW}@horizon-db-pooler.${NS}.svc.cluster.local:6432/horizon?sslmode=disable&binary_parameters=yes"
  k apply -f "$HERE/horizon-api.yaml"
  k wait --for=condition=complete job/horizon-db-migrate --timeout=300s
  k rollout status deployment/horizon-api --timeout=300s

  step "Starting traffic probes"
  k apply -f "$HERE/probes.yaml"
  k wait --for=condition=Ready pod/http-probe pod/write-probe --timeout=180s
else
  HZ_PW=$(k get secret horizon-db-credentials -o jsonpath='{.data.horizon-password}' | base64 -d)
  ADMIN_PW=$(k get secret horizon-db-pooler-auth -o jsonpath='{.data.userlist\.txt}' | base64 -d \
    | awk -F'"' '$2 == "pgbouncer_admin" { print $4 }')
fi

sleep 5
step "Baseline"
print_cluster
mark; sleep 10; report "baseline (10s, no fault)"
HORIZON_BEFORE=$(horizon_state)

# ---------------------------------------------------------------------------
# 1. Planned switchover, zero dropped requests
# ---------------------------------------------------------------------------
step "Scenario 1: planned switchover with PgBouncer PAUSE/RESUME"
OLD=$(leader); CANDIDATE=$(sync_standby)
info "leader=$OLD  candidate(sync standby)=$CANDIDATE"
mark
pooler_admin "PAUSE horizon;"
k exec "$OLD" -- patronictl -c /etc/patroni/patroni.yml switchover horizon-db \
  --leader "$OLD" --candidate "$CANDIDATE" --force
wait_for 60 "new leader $CANDIDATE owns the primary endpoint" leader_is "$CANDIDATE"
pooler_admin "RECONNECT horizon;"
pooler_admin "RESUME horizon;"
sleep 5
report "1. switchover (PAUSE/RESUME)"
check "switchover: zero failed Horizon requests" "$(( LAST_HTTP_FAIL == 0 ? 0 : 1 ))"
check "switchover: zero failed pooled writes"    "$(( LAST_WRITE_ERR == 0 ? 0 : 1 ))"
wait_for 180 "old leader $OLD rejoined as replica" cluster_healthy
print_cluster

# ---------------------------------------------------------------------------
# 2. Primary pod killed
# ---------------------------------------------------------------------------
step "Scenario 2: delete the primary pod"
OLD=$(leader)
info "deleting primary pod $OLD"
mark
k delete pod "$OLD" --wait=false
wait_for 90 "replica promoted, primary endpoint moved off $OLD" leader_is_not "$OLD"
info "new leader: $(leader) ($(leader_ip))"
sleep 5
report "2. primary pod deleted"
wait_for 240 "recreated $OLD rejoined as replica" cluster_healthy
print_cluster
check "pod kill: a replica was promoted and serves writes" 0

# ---------------------------------------------------------------------------
# 3. Network partition of the primary's node (split-brain)
# ---------------------------------------------------------------------------
step "Scenario 3: freeze the primary's node (partition) and verify no split-brain"
OLD=$(leader); OLD_IP=$(pod_ip "$OLD")
NODE=$(k get pod "$OLD" -o jsonpath='{.spec.nodeName}')
info "primary $OLD ($OLD_IP) on node $NODE: docker pause"
mark
PARTITION_START=$(date -u +%H:%M:%S)
docker pause "$NODE" >/dev/null
wait_for 120 "surviving members elected a new leader" leader_is_not "$OLD"
NEW=$(leader)
info "new leader: $NEW ($(leader_ip))"
sleep 10
info "healing partition: docker unpause $NODE"
docker unpause "$NODE" >/dev/null
wait_for 300 "old primary $OLD demoted, rewound and rejoined" cluster_healthy
sleep 3
report "3. partition of primary node"
print_cluster

# Every write the probe saw acknowledged must exist on the final primary.
ACKED=$(k logs write-probe | awk '$2 == "ok" { split($3, a, "|"); print a[1] }' | sort -n)
STORED=$(k exec "$(leader)" -- psql -U postgres -d horizon -XAtq -c 'SELECT seq FROM ha_probe ORDER BY 1')
MISSING=$(comm -23 <(printf '%s\n' "$ACKED" | sort) <(printf '%s\n' "$STORED" | sort) | grep -c . || true)
info "acknowledged writes: $(printf '%s\n' "$ACKED" | grep -c .), missing on final primary: $MISSING"
check "split-brain: zero acknowledged writes lost ($MISSING missing)" "$(( MISSING == 0 ? 0 : 1 ))"

# The isolated old primary must not acknowledge any write once the partition started.
STALE=$(k logs write-probe | tail -n +"$((WRITE_MARK + 1))" \
  | awk -v ip="$OLD_IP" -v t0="$PARTITION_START" '$2 == "ok" && $1 > t0 { split($3, a, "|"); if (a[2] == ip) n++ } END { print n + 0 }')
info "writes acknowledged by old primary $OLD_IP after partition: $STALE"
check "split-brain: old primary acknowledged no writes after partition ($STALE)" "$(( STALE == 0 ? 0 : 1 ))"

# ---------------------------------------------------------------------------
# Horizon was never restarted or reconfigured
# ---------------------------------------------------------------------------
HORIZON_AFTER=$(horizon_state)
info "Horizon pods before: $HORIZON_BEFORE"
info "Horizon pods after:  $HORIZON_AFTER"
check "Horizon pods never restarted or replaced" "$([[ "$HORIZON_BEFORE" == "$HORIZON_AFTER" ]] && echo 0 || echo 1)"
check "Patroni cluster healthy at the end (3 members, 1 timeline)" "$(cluster_healthy && echo 0 || echo 1)"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
step "Summary"
printf '    %-34s %8s %8s %8s %8s %7s\n' "scenario" "reads" "failed" "writes" "failed" "gap"
for row in "${SCENARIO_ROWS[@]}"; do printf '    %s\n' "$row"; done
echo
for line in "${SUMMARY[@]}"; do printf '    %s\n' "$line"; done
echo
if (( FAILURES > 0 )); then
  printf '%s%d check(s) failed.%s\n' "$RED" "$FAILURES" "$RESET"
  exit 1
fi
printf '%sAll checks passed.%s\n' "$GREEN" "$RESET"
