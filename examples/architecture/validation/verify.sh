#!/usr/bin/env bash
# verify.sh
#
# Validates examples/architecture/captive-core-scaling.yaml on kind:
#   - exactly ONE stellar-core (Captive Core) process runs, inside the ingest pod;
#   - all 5 Horizon API replicas run with NO stellar-core process and serve API requests;
#   - every API replica reports the same, advancing ledger (shared database state) and
#     reaches the single Captive Core over HTTP (transaction-submission path);
#   - scaling the API tier to 10 replicas still leaves exactly one Captive Core.
#
# Usage:
#   ./verify.sh            # create cluster, verify, delete cluster
#   ./verify.sh --keep     # keep the cluster afterwards
#   ./verify.sh --reuse    # run checks against an existing deployment
#
# Requirements: docker, kind, kubectl, openssl, python3.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CLUSTER="horizon-decoupled"
NS="stellar"
KEEP=0
REUSE=0
for arg in "$@"; do
  case "$arg" in
    --keep)  KEEP=1 ;;
    --reuse) REUSE=1; KEEP=1 ;;
    -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
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
check() { # check <description> <0|1>
  if [[ "$2" == 0 ]]; then SUMMARY+=("${GREEN}PASS${RESET}  $1")
  else SUMMARY+=("${RED}FAIL${RESET}  $1"); FAILURES=$((FAILURES + 1)); fi
}

cleanup() {
  if (( KEEP == 0 )); then
    step "Deleting kind cluster $CLUSTER"
    kind delete cluster --name "$CLUSTER" || true
  else
    info "Cluster kept: kubectl --context kind-$CLUSTER -n $NS get pods"
  fi
}

# Number of `stellar-core run` processes inside a pod (reads /proc; no ps needed).
core_procs() {
  # shellcheck disable=SC2016  # $p expands inside the pod, not locally
  k exec "$1" -c horizon -- sh -c \
    'for p in /proc/[0-9]*; do tr "\0" " " < "$p/cmdline" 2>/dev/null; echo; done' \
    | grep -c "stellar-core.* run" || true
}

api_pods()  { k get pods -l app.kubernetes.io/component=api -o jsonpath='{.items[*].metadata.name}'; }
ingest_pod() { echo "horizon-ingest-0"; }

# Prints one line per API pod: "<pod> <root-http> <ledgers-http> <history_latest_ledger> <core_latest_ledger>"
sample_api() {
  local pod ip root code
  for pod in $(api_pods); do
    ip=$(k get pod "$pod" -o jsonpath='{.status.podIP}')
    root=$(k exec api-probe -- curl -s --max-time 10 -w '\n%{http_code}' "http://$ip:8000/")
    code=$(k exec api-probe -- curl -s -o /dev/null --max-time 10 -w '%{http_code}' \
      "http://$ip:8000/ledgers?order=desc&limit=1")
    printf '%s\n' "$root" | python3 -c '
import json, sys
lines = sys.stdin.read().rsplit("\n", 1)
body, status = lines[0], lines[-1]
try:
    j = json.loads(body)
except ValueError:
    j = {}
print(sys.argv[1], status, sys.argv[2], j.get("history_latest_ledger", 0), j.get("core_latest_ledger", 0))
' "$pod" "$code"
  done
}

# ---------------------------------------------------------------------------
# Deploy
# ---------------------------------------------------------------------------
if (( REUSE == 0 )); then
  trap cleanup EXIT
  step "Creating kind cluster $CLUSTER"
  kind create cluster --config "$HERE/kind-config.yaml" --wait 120s

  step "Creating namespace and generated secrets"
  kubectl --context "kind-$CLUSTER" create namespace "$NS"
  PG_PW=$(openssl rand -hex 24)
  k create secret generic horizon-postgres --from-literal=password="$PG_PW"
  k create secret generic horizon-db --from-literal=DATABASE_URL=\
"postgres://horizon:${PG_PW}@horizon-postgres:5432/horizon?sslmode=disable"

  step "Deploying standalone network + decoupled Horizon (1 ingest pod, 5 API pods)"
  kubectl kustomize --load-restrictor=LoadRestrictionsNone "$HERE" | k apply -f -

  step "Waiting for dependencies"
  k rollout status deployment/stellar-core-validator --timeout=300s
  k rollout status deployment/horizon-postgres --timeout=300s
  k wait --for=condition=complete job/stellar-core-protocol-upgrade --timeout=300s
  k logs job/stellar-core-protocol-upgrade | sed 's/^/    /'

  step "Waiting for the ingest pod (Captive Core catch-up) and the API tier"
  k rollout status statefulset/horizon-ingest --timeout=900s
  k rollout status deployment/horizon-api --timeout=300s
  k wait --for=condition=Ready pod/api-probe --timeout=120s
fi

# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------
step "Captive Core placement"
INGEST_CORES=$(core_procs "$(ingest_pod)")
info "$(ingest_pod): $INGEST_CORES stellar-core process(es)"
API_CORES=0
for pod in $(api_pods); do
  n=$(core_procs "$pod"); API_CORES=$((API_CORES + n))
  info "$pod: $n stellar-core process(es)"
done
check "ingest pod runs exactly one Captive Core ($INGEST_CORES)" "$(( INGEST_CORES == 1 ? 0 : 1 ))"
check "API pods run no stellar-core at all ($API_CORES)"       "$(( API_CORES == 0 ? 0 : 1 ))"

step "Every API replica serves requests from the shared database"
READY=$(k get deployment horizon-api -o jsonpath='{.status.readyReplicas}')
info "ready API replicas: $READY"
check "5 API replicas ready" "$(( ${READY:-0} == 5 ? 0 : 1 ))"

S1=$(sample_api); printf '%s\n' "$S1" | awk '{printf "    %-34s /=%s /ledgers=%s history_latest_ledger=%s core_latest_ledger=%s\n", $1, $2, $3, $4, $5}'
sleep 15
S2=$(sample_api); printf '%s\n' "$S2" | awk '{printf "    %-34s /=%s /ledgers=%s history_latest_ledger=%s core_latest_ledger=%s\n", $1, $2, $3, $4, $5}'

ALL_200=$(printf '%s\n' "$S1" "$S2" | awk '$2 != 200 || $3 != 200 { bad++ } END { print bad + 0 }')
check "all API replicas answer / and /ledgers with 200 (non-200: $ALL_200)" "$(( ALL_200 == 0 ? 0 : 1 ))"

SPREAD=$(printf '%s\n' "$S2" | awk 'NR == 1 { mn = mx = $4 } { if ($4 < mn) mn = $4; if ($4 > mx) mx = $4 } END { print mx - mn }')
MIN_LEDGER=$(printf '%s\n' "$S2" | awk 'NR == 1 { mn = $4 } $4 < mn { mn = $4 } END { print mn + 0 }')
check "replicas agree on history_latest_ledger (spread $SPREAD, latest >= $MIN_LEDGER)" \
  "$(( SPREAD <= 2 && MIN_LEDGER > 0 ? 0 : 1 ))"

ADVANCED=$(paste -d' ' <(printf '%s\n' "$S1" | sort) <(printf '%s\n' "$S2" | sort) \
  | awk '$9 <= $4 { stale++ } END { print stale + 0 }')
check "shared state advances on every replica between samples (stale: $ADVANCED)" "$(( ADVANCED == 0 ? 0 : 1 ))"

NO_CORE=$(printf '%s\n' "$S2" | awk '$5 <= 0 { n++ } END { print n + 0 }')
check "every replica reaches the single Captive Core over HTTP (core_latest_ledger > 0)" "$(( NO_CORE == 0 ? 0 : 1 ))"

step "Scaling the API tier to 10 replicas"
k scale deployment/horizon-api --replicas=10
k rollout status deployment/horizon-api --timeout=300s
TOTAL=0
for pod in $(api_pods) "$(ingest_pod)"; do TOTAL=$((TOTAL + $(core_procs "$pod"))); done
S3=$(sample_api)
BAD10=$(printf '%s\n' "$S3" | awk '$2 != 200 || $3 != 200 { bad++ } END { print bad + 0 }')
info "$(printf '%s\n' "$S3" | grep -c .) API replicas sampled, non-200: $BAD10, stellar-core processes in namespace: $TOTAL"
check "10 API replicas all serve 200 with still exactly one Captive Core" "$(( BAD10 == 0 && TOTAL == 1 ? 0 : 1 ))"
k scale deployment/horizon-api --replicas=5 >/dev/null

# ---------------------------------------------------------------------------
step "Summary"
for line in "${SUMMARY[@]}"; do printf '    %s\n' "$line"; done
echo
if (( FAILURES > 0 )); then printf '%s%d check(s) failed.%s\n' "$RED" "$FAILURES" "$RESET"; exit 1; fi
printf '%sAll checks passed.%s\n' "$GREEN" "$RESET"
