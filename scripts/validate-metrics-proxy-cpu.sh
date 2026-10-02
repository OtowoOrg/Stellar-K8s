#!/usr/bin/env bash
# validate-metrics-proxy-cpu.sh
#
# Validates the CPU-load impact of the intelligent metrics proxy by comparing
# Stellar Core CPU utilisation during:
#   1. Direct Prometheus scraping (baseline — no proxy)
#   2. Scraping through the stellar-metrics-proxy sidecar
#
# Prerequisites:
#   - kubectl configured with access to the cluster
#   - Prometheus accessible at $PROMETHEUS_URL
#   - The Stellar Core pod must be running in $NAMESPACE
#   - bc, curl, jq installed on the host running this script
#
# Usage:
#   NAMESPACE=stellar-system \
#   POD_NAME=stellar-core-0 \
#   PROMETHEUS_URL=http://prometheus.monitoring.svc:9090 \
#   CONTAINER=stellar-core \
#   ./scripts/validate-metrics-proxy-cpu.sh
#
# Exit codes:
#   0  CPU improvement ≥ 10% (proxy is effective)
#   1  Insufficient improvement or measurement error
#   2  Prerequisite check failed

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
NAMESPACE="${NAMESPACE:-stellar-system}"
POD_NAME="${POD_NAME:-stellar-core-0}"
CONTAINER="${CONTAINER:-stellar-core}"
PROMETHEUS_URL="${PROMETHEUS_URL:-http://localhost:9090}"

# How long to run each load phase (seconds).
MEASURE_WINDOW="${MEASURE_WINDOW:-60}"

# Number of concurrent scrape goroutines during the load test.
CONCURRENT_SCRAPERS="${CONCURRENT_SCRAPERS:-5}"

# Scrape interval during the test (seconds).
SCRAPE_INTERVAL="${SCRAPE_INTERVAL:-2}"

# Minimum required CPU reduction percentage to pass validation.
MIN_CPU_REDUCTION_PCT="${MIN_CPU_REDUCTION_PCT:-10}"

# Direct scrape endpoint (Stellar Core).
DIRECT_METRICS_URL="http://$(kubectl get pod "${POD_NAME}" -n "${NAMESPACE}" \
  --template='{{.status.podIP}}'):11626/metrics"

# Proxy scrape endpoint.
PROXY_METRICS_URL="http://$(kubectl get pod "${POD_NAME}" -n "${NAMESPACE}" \
  --template='{{.status.podIP}}'):9091/metrics"

# ---------------------------------------------------------------------------
# Prerequisite checks
# ---------------------------------------------------------------------------
check_prereqs() {
  local missing=0
  for cmd in kubectl curl jq bc; do
    if ! command -v "$cmd" &>/dev/null; then
      echo "ERROR: Required command not found: $cmd" >&2
      missing=1
    fi
  done
  if [[ "$missing" -eq 1 ]]; then
    exit 2
  fi

  echo "✓ All prerequisites satisfied"
}

# ---------------------------------------------------------------------------
# Prometheus query helper
# ---------------------------------------------------------------------------
# Query average CPU usage (rate of cpu_seconds over 1 m) for the target container.
query_cpu_avg() {
  local window="$1"   # e.g. "1m"
  local promql
  promql="avg_over_time(rate(container_cpu_usage_seconds_total{namespace=\"${NAMESPACE}\",pod=\"${POD_NAME}\",container=\"${CONTAINER}\"}[30s])[${window}:5s])"

  local result
  result=$(curl -sf --max-time 10 \
    "${PROMETHEUS_URL}/api/v1/query" \
    --data-urlencode "query=${promql}" | \
    jq -r '.data.result[0].value[1] // "0"')

  echo "$result"
}

# ---------------------------------------------------------------------------
# Scrape load generator
# ---------------------------------------------------------------------------
run_scrapers() {
  local url="$1"
  local duration="$2"
  local n="$3"
  local interval="$4"

  echo "  Running $n concurrent scrapers → $url for ${duration}s (interval ${interval}s)"

  local pids=()
  for (( i=0; i<n; i++ )); do
    (
      local end=$(( $(date +%s) + duration ))
      while [[ $(date +%s) -lt $end ]]; do
        curl -sf --max-time 5 "$url" >/dev/null 2>&1 || true
        sleep "$interval"
      done
    ) &
    pids+=("$!")
  done

  # Let them run for the measurement window.
  sleep "$duration"

  # Gracefully kill all scrapers.
  for pid in "${pids[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  wait "${pids[@]}" 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# Measurement phase
# ---------------------------------------------------------------------------
measure_phase() {
  local label="$1"
  local url="$2"

  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "Phase: $label"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

  # Warm-up: 10 s of light scraping to settle CPU.
  echo "  Warm-up (10 s)..."
  run_scrapers "$url" 10 1 5

  echo "  Measurement window: ${MEASURE_WINDOW}s with ${CONCURRENT_SCRAPERS} scrapers @ ${SCRAPE_INTERVAL}s interval"
  run_scrapers "$url" "$MEASURE_WINDOW" "$CONCURRENT_SCRAPERS" "$SCRAPE_INTERVAL" &
  local load_pid=$!

  # Let scrapers run for half the window before sampling.
  sleep $(( MEASURE_WINDOW / 2 ))

  local cpu
  cpu=$(query_cpu_avg "30s")
  echo "  Sampled CPU rate: ${cpu} cores"

  wait "$load_pid" 2>/dev/null || true

  echo "$cpu"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
main() {
  echo "╔══════════════════════════════════════════════════════╗"
  echo "║   Stellar Metrics Proxy — CPU Validation             ║"
  echo "╚══════════════════════════════════════════════════════╝"
  echo ""
  echo "Pod:       $POD_NAME / $NAMESPACE"
  echo "Container: $CONTAINER"
  echo "Direct:    $DIRECT_METRICS_URL"
  echo "Proxy:     $PROXY_METRICS_URL"
  echo ""

  check_prereqs

  # Phase 1: direct scraping (baseline).
  CPU_DIRECT=$(measure_phase "Direct scrape (baseline)" "$DIRECT_METRICS_URL")

  # Cool-down.
  echo ""
  echo "Cool-down: 30s..."
  sleep 30

  # Phase 2: proxy scraping.
  CPU_PROXY=$(measure_phase "Proxy scrape" "$PROXY_METRICS_URL")

  # ---------------------------------------------------------------------------
  # Results
  # ---------------------------------------------------------------------------
  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "Results"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  printf "  Direct scrape CPU:  %.6f cores\n" "$CPU_DIRECT"
  printf "  Proxy scrape CPU:   %.6f cores\n" "$CPU_PROXY"

  if (( $(echo "$CPU_DIRECT == 0" | bc -l) )); then
    echo "WARNING: Direct scrape CPU is zero — Prometheus metrics may not be available."
    echo "         Ensure the pod is running and the Prometheus query is correct."
    exit 1
  fi

  REDUCTION_PCT=$(echo "scale=2; (($CPU_DIRECT - $CPU_PROXY) / $CPU_DIRECT) * 100" | bc -l)
  printf "  CPU reduction:      %.2f%%\n" "$REDUCTION_PCT"
  echo ""

  PASS=$(echo "$REDUCTION_PCT >= $MIN_CPU_REDUCTION_PCT" | bc -l)
  if [[ "$PASS" -eq 1 ]]; then
    echo "✅ PASS — proxy reduces Stellar Core CPU by ≥ ${MIN_CPU_REDUCTION_PCT}% under concurrent scraping."
    exit 0
  else
    echo "❌ FAIL — CPU reduction (${REDUCTION_PCT}%) is below the ${MIN_CPU_REDUCTION_PCT}% threshold."
    echo "   Consider:"
    echo "     - Increasing minScrapeIntervalMs in the ConfigMap"
    echo "     - Adding more series to seriesFilterRules (action: drop)"
    echo "     - Adding slow-changing families to cacheRules"
    exit 1
  fi
}

main "$@"
