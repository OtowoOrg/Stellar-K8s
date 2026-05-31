#!/usr/bin/env bash
# Proximity-aware scheduler benchmark script.
# Compares default vs proximity-optimized validator placement.
#
# Usage:
#   ./benchmarks/run-proximity-benchmark.sh [--baseline-only|--optimized-only]
#
# Requires: kubectl, curl, jq

set -euo pipefail

MODE="${1:-all}"
NAMESPACE="${NAMESPACE:-stellar}"
THRESHOLD_MS="${LATENCY_EVICTION_THRESHOLD_MS:-150}"

echo "=== Stellar Proximity Scheduler Benchmark ==="
echo "Namespace: $NAMESPACE"
echo "Latency threshold: ${THRESHOLD_MS}ms"
echo ""

measure_latency() {
  local label="$1"
  echo "--- $label ---"

  # Query Prometheus for average quorum consensus latency
  PROM_URL="${PROMETHEUS_URL:-http://localhost:9090}"
  LATENCY=$(curl -sf "${PROM_URL}/api/v1/query" \
    --data-urlencode "query=avg(stellar_quorum_consensus_latency_ms{namespace=\"${NAMESPACE}\"})" \
    | jq -r '.data.result[0].value[1] // "N/A"')

  POD_COUNT=$(kubectl get pods -n "$NAMESPACE" \
    -l stellar.org/node-type=Validator --no-headers 2>/dev/null | wc -l | tr -d ' ')

  echo "  Validator pods: $POD_COUNT"
  echo "  Avg quorum latency: ${LATENCY}ms"
  echo ""
  echo "$LATENCY"
}

case "$MODE" in
  --baseline-only)
    measure_latency "Baseline (default scheduler)"
    ;;
  --optimized-only)
    measure_latency "Proximity-optimized (custom scheduler)"
    ;;
  *)
    echo "Step 1: Deploy validators with default scheduler"
    echo "  Set proximityAware: false in StellarNode specs"
    echo ""
    echo "Step 2: Deploy validators with proximityAware: true"
    echo "  Enable custom scheduler: stellar-operator run --scheduler"
    echo ""
    BASELINE=$(measure_latency "Baseline")
    OPTIMIZED=$(measure_latency "Optimized")

    if [[ "$BASELINE" != "N/A" && "$OPTIMIZED" != "N/A" ]]; then
      REDUCTION=$(echo "scale=1; ($BASELINE - $OPTIMIZED) / $BASELINE * 100" | bc)
      echo "=== Results ==="
      echo "  Baseline latency:  ${BASELINE}ms"
      echo "  Optimized latency: ${OPTIMIZED}ms"
      echo "  Reduction:         ${REDUCTION}%"
      echo ""
      echo "See docs/SCHEDULER_PROXIMITY_BENCHMARK.md for expected improvements."
    fi
    ;;
esac
