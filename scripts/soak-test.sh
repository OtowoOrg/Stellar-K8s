#!/usr/bin/env bash
# soak-test.sh — 1-hour memory soak test for the stellar-operator.
#
# Creates/deletes StellarNode resources in a loop while sampling the operator's
# RSS every 60 s. Fails if memory grows more than THRESHOLD_KB (default 5 MB)
# from the baseline reading.
#
# Usage:
#   OPERATOR_NAMESPACE=stellar-system SOAK_DURATION=3600 ./scripts/soak-test.sh

set -euo pipefail

OPERATOR_NAMESPACE="${OPERATOR_NAMESPACE:-stellar-system}"
SOAK_DURATION="${SOAK_DURATION:-3600}"        # seconds (1 hour)
SAMPLE_INTERVAL=60                             # seconds between RSS samples
THRESHOLD_KB=5120                              # 5 MB growth limit
NODE_COUNT="${NODE_COUNT:-100}"
TEST_NAMESPACE="${TEST_NAMESPACE:-soak-test}"
CLEANUP_NAMESPACE_ON_EXIT="${CLEANUP_NAMESPACE_ON_EXIT:-false}"
RESULTS_FILE="${RESULTS_FILE:-/tmp/soak-memory.log}"
CLEANUP_DONE=false
CLEANUP_TIMEOUT_SECONDS="${CLEANUP_TIMEOUT_SECONDS:-120}"
RETRY_DELAY_SECONDS="${RETRY_DELAY_SECONDS:-15}"

# ── Input validation ─────────────────────────────────────────────────────────

if ! [[ "$NODE_COUNT" =~ ^[0-9]+$ ]] || [[ "$NODE_COUNT" -lt 1 ]]; then
  echo "ERROR: NODE_COUNT must be an integer >= 1 (got '${NODE_COUNT}')" >&2
  exit 1
fi

if [[ -z "$TEST_NAMESPACE" ]]; then
  echo "ERROR: TEST_NAMESPACE must not be empty" >&2
  exit 1
fi

# ── Startup plan ─────────────────────────────────────────────────────────────

echo "=== Soak Test Configuration ==="
echo "  Operator namespace : $OPERATOR_NAMESPACE"
echo "  Test namespace     : $TEST_NAMESPACE"
echo "  Node count         : $NODE_COUNT"
echo "  Soak duration      : ${SOAK_DURATION}s"
echo "  Memory threshold   : ${THRESHOLD_KB} kB"
echo "  Results file       : $RESULTS_FILE"
echo "  Cleanup on exit    : $CLEANUP_NAMESPACE_ON_EXIT"
echo "==============================="

# ── Helpers ──────────────────────────────────────────────────────────────────

get_operator_pid() {
  kubectl get pods -n "$OPERATOR_NAMESPACE" \
    -l app=stellar-operator \
    -o jsonpath='{.items[0].metadata.name}' 2>/dev/null
}

validate_positive_integer() {
  local name="$1"
  local value="$2"
  if ! [[ "$value" =~ ^[0-9]+$ ]]; then
    echo "ERROR: ${name} must be an integer, got '${value}'"
    exit 1
  fi
  if [[ "$value" -lt 1 ]]; then
    echo "ERROR: ${name} must be >= 1, got '${value}'"
    exit 1
  fi
}

get_operator_pid_with_retry() {
  local max_attempts="${1:-5}"
  local attempt=1
  local pod_name=""
  while [[ "$attempt" -le "$max_attempts" ]]; do
    pod_name=$(get_operator_pid)
    if [[ -n "$pod_name" ]]; then
      echo "$pod_name"
      return 0
    fi
    echo "Operator pod not found (attempt ${attempt}/${max_attempts}); retrying in ${RETRY_DELAY_SECONDS}s..."
    sleep "$RETRY_DELAY_SECONDS"
    attempt=$(( attempt + 1 ))
  done
  return 1
}

get_rss_kb() {
  local pod="$1"
  # Read /proc/1/status from inside the container (PID 1 = operator process)
  kubectl exec -n "$OPERATOR_NAMESPACE" "$pod" -- \
    awk '/^VmRSS:/{print $2}' /proc/1/status 2>/dev/null || echo "0"
}

apply_nodes() {
  for i in $(seq 1 "$NODE_COUNT"); do
    kubectl apply -f - <<EOF
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: soak-node-${i}
  namespace: ${TEST_NAMESPACE}
spec:
  nodeType: Validator
  network: Testnet
  version: "v21.0.0"
  storage:
    storageClass: standard
    size: 10Gi
    retentionPolicy: Delete
EOF
  done
}

delete_nodes() {
  for i in $(seq 1 "$NODE_COUNT"); do
    kubectl delete stellarnode "soak-node-${i}" -n "$TEST_NAMESPACE" \
      --ignore-not-found --wait=false
  done
  # Wait for all to be gone before the next wave.
  if kubectl wait stellarnode \
    --for=delete \
    --all \
    -n "$TEST_NAMESPACE" \
    --timeout="${CLEANUP_TIMEOUT_SECONDS}s" 2>/dev/null; then
    echo "Cleanup finished within ${CLEANUP_TIMEOUT_SECONDS}s"
    return 0
  fi

  local remaining
  remaining=$(kubectl get stellarnode -n "$TEST_NAMESPACE" --no-headers 2>/dev/null | wc -l | tr -d ' ')
  echo "WARN: Cleanup timeout reached after ${CLEANUP_TIMEOUT_SECONDS}s with ${remaining} resource(s) still present."
  echo "Aborting soak loop because it is unsafe to proceed with leftover resources."
  return 1
}

cleanup_resources() {
  local reason="${1:-exit}"
  if [[ "$CLEANUP_DONE" == "true" ]]; then
    echo "[cleanup] Already completed (reason: ${reason})"
    return
  fi
  CLEANUP_DONE=true

  echo "[cleanup] Starting cleanup (reason: ${reason})..."
  delete_nodes || true
  kubectl delete namespace "$TEST_NAMESPACE" --ignore-not-found --wait=false >/dev/null 2>&1 || true
  echo "[cleanup] Cleanup finished for namespace: ${TEST_NAMESPACE}"
}

handle_exit() {
  local exit_code=$?
  cleanup_resources "exit"
  if [[ $exit_code -ne 0 ]]; then
    echo "[cleanup] Exiting with failure code: ${exit_code}"
  else
    echo "[cleanup] Exiting successfully"
  fi
  exit "$exit_code"
}

handle_signal() {
  local signal_name="$1"
  local signal_code=1
  case "$signal_name" in
    INT) signal_code=130 ;;
    TERM) signal_code=143 ;;
  esac
  echo "[cleanup] Received ${signal_name}; requesting graceful shutdown..."
  exit "$signal_code"
}

trap 'handle_signal INT' INT
trap 'handle_signal TERM' TERM
trap 'handle_exit' EXIT

# ── Setup ─────────────────────────────────────────────────────────────────────

# Idempotent namespace creation
kubectl create namespace "$TEST_NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

# Optional cleanup trap — only runs when CLEANUP_NAMESPACE_ON_EXIT=true
cleanup_namespace() {
  if [[ "$CLEANUP_NAMESPACE_ON_EXIT" == "true" ]]; then
    echo "Cleaning up namespace $TEST_NAMESPACE..."
    kubectl delete namespace "$TEST_NAMESPACE" --ignore-not-found --wait=false || true
  fi
}
trap cleanup_namespace EXIT

OPERATOR_POD=$(get_operator_pid)
validate_positive_integer "RETRY_DELAY_SECONDS" "$RETRY_DELAY_SECONDS"
echo "Retry delay: ${RETRY_DELAY_SECONDS}s"

OPERATOR_POD=$(get_operator_pid_with_retry 5)
if [[ -z "$OPERATOR_POD" ]]; then
  echo "ERROR: No stellar-operator pod found in namespace $OPERATOR_NAMESPACE"
  exit 1
fi
echo "Operator pod: $OPERATOR_POD"
echo "Cleanup timeout: ${CLEANUP_TIMEOUT_SECONDS}s"

# Baseline — let the operator settle for one sample interval first
sleep "$SAMPLE_INTERVAL"
OPERATOR_POD=$(get_operator_pid)   # refresh in case of restart
BASELINE_KB=$(get_rss_kb "$OPERATOR_POD")
echo "Baseline RSS: ${BASELINE_KB} kB"
echo "0 ${BASELINE_KB}" > "$RESULTS_FILE"

# ── Main loop ─────────────────────────────────────────────────────────────────

START_TS=$(date +%s)
ELAPSED=0
WAVE=0

while [[ $ELAPSED -lt $SOAK_DURATION ]]; do
  WAVE=$(( WAVE + 1 ))
  echo "--- Wave ${WAVE} (elapsed ${ELAPSED}s) ---"

  apply_nodes
  sleep 10
  delete_nodes

  # Sample memory after each wave (and on the fixed interval)
  OPERATOR_POD=$(get_operator_pid_with_retry 5)
  CURRENT_KB=$(get_rss_kb "$OPERATOR_POD")
  GROWTH_KB=$(( CURRENT_KB - BASELINE_KB ))
  NOW=$(date +%s)
  ELAPSED=$(( NOW - START_TS ))

  echo "${ELAPSED} ${CURRENT_KB}" >> "$RESULTS_FILE"
  echo "  RSS: ${CURRENT_KB} kB  |  growth: ${GROWTH_KB} kB  |  limit: ${THRESHOLD_KB} kB"

  if [[ $GROWTH_KB -gt $THRESHOLD_KB ]]; then
    echo ""
    echo "FAIL: Memory grew ${GROWTH_KB} kB (limit ${THRESHOLD_KB} kB) after ${ELAPSED}s / wave ${WAVE}"
    echo "Samples:"
    cat "$RESULTS_FILE"
    exit 1
  fi

  # Sleep until the next sample boundary (skip if wave took longer)
  SLEEP_FOR=$(( SAMPLE_INTERVAL - (ELAPSED % SAMPLE_INTERVAL) ))
  [[ $SLEEP_FOR -gt 0 ]] && sleep "$SLEEP_FOR" || true
  ELAPSED=$(( $(date +%s) - START_TS ))
done

# ── Final report ──────────────────────────────────────────────────────────────

FINAL_KB=$(get_rss_kb "$OPERATOR_POD")
TOTAL_GROWTH=$(( FINAL_KB - BASELINE_KB ))

echo ""
echo "Soak test complete."
echo "  Duration : ${SOAK_DURATION}s"
echo "  Waves    : ${WAVE}"
echo "  Baseline : ${BASELINE_KB} kB"
echo "  Final    : ${FINAL_KB} kB"
echo "  Growth   : ${TOTAL_GROWTH} kB  (limit ${THRESHOLD_KB} kB)"
echo ""
echo "Memory samples (elapsed_s rss_kb):"
cat "$RESULTS_FILE"

if [[ $TOTAL_GROWTH -gt $THRESHOLD_KB ]]; then
  echo ""
  echo "FAIL: Total memory growth ${TOTAL_GROWTH} kB exceeds threshold ${THRESHOLD_KB} kB"
  exit 1
fi

echo ""
echo "PASS: Memory growth within acceptable limits."
