#!/usr/bin/env bash
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
# bench-helm-render.sh — Helm chart render timing benchmark (Issue #1390)
#
# `helm template` rendering isn't something `cargo bench`/criterion can time
# (it's an external CLI, not Rust code), so this script fills the "Helm
# rendering" leg of the benchmark suite the same way scripts/check-helm-drift.sh
# already exercises the chart: shell out to `helm template` for a small set of
# representative values files and measure wall-clock render time directly.
#
# For each profile it renders the chart `--iterations` times (after one
# untimed warm-up render) and reports min/mean/p95/max wall-clock time in
# milliseconds, in the same {"metrics": {...}} JSON shape used by
# benchmarks/baselines/*.json — see benchmarks/scripts/compare_benchmarks.py
# and .github/actions/compare-benchmarks, which already consume that shape
# for the k6 suites and now this one too. See docs/benchmarking.md for how to
# interpret the numbers.
#
# Usage:
#   scripts/bench-helm-render.sh [--iterations N] [--output FILE]
#
# Exit codes: 0 = benchmark completed, 1 = helm/chart missing or render failed.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CHART_DIR="${PROJECT_ROOT}/charts/stellar-operator"
RELEASE_NAME="stellar-operator"
RELEASE_NAMESPACE="stellar-system"

ITERATIONS=20
OUTPUT="${PROJECT_ROOT}/results/helm-render-benchmark.json"

usage() {
  sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --iterations) ITERATIONS="${2:?--iterations needs a number}"; shift 2 ;;
    --output) OUTPUT="${2:?--output needs a path}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

command -v helm >/dev/null 2>&1 || { echo "ERROR: helm is not installed" >&2; exit 1; }
[[ -d "${CHART_DIR}" ]] || { echo "ERROR: chart directory not found: ${CHART_DIR}" >&2; exit 1; }

# Profiles kept deliberately small and aligned with scripts/check-helm-drift.sh's
# representative set: unmodified defaults, plus a values file that turns on a
# meaningfully larger set of features (HA) and one used for production
# deployments, so the benchmark reflects both a cheap and an expensive render.
PROFILES=(
  "default|"
  "ha|-f ${CHART_DIR}/values-ha.yaml"
  "production|-f ${CHART_DIR}/examples/values-production.yaml"
)

TEMP_DIR="$(mktemp -d)"
cleanup() { rm -rf "${TEMP_DIR}"; }
trap cleanup EXIT

now_ns() {
  date +%s%N
}

echo "→ Helm render benchmark (${ITERATIONS} iterations per profile)"
echo "  chart: ${CHART_DIR#"${PROJECT_ROOT}"/}"
echo ""

profile_entries=()

for entry in "${PROFILES[@]}"; do
  name="${entry%%|*}"
  extra="${entry#*|}"

  # Untimed warm-up: pays for chart parsing/lint the first call always does,
  # and fails fast (with the real helm error) if a profile is broken.
  # shellcheck disable=SC2086
  if ! helm template "${RELEASE_NAME}" "${CHART_DIR}" --namespace "${RELEASE_NAMESPACE}" ${extra} \
        > "${TEMP_DIR}/${name}-warmup.yaml" 2>"${TEMP_DIR}/${name}.stderr"; then
    echo "  ✗ ${name}: helm template failed"
    cat "${TEMP_DIR}/${name}.stderr" >&2
    exit 1
  fi

  durations_ms=()
  for ((i = 1; i <= ITERATIONS; i++)); do
    start="$(now_ns)"
    # shellcheck disable=SC2086
    helm template "${RELEASE_NAME}" "${CHART_DIR}" --namespace "${RELEASE_NAMESPACE}" ${extra} \
      > "${TEMP_DIR}/${name}-render.yaml" 2>"${TEMP_DIR}/${name}.stderr"
    end="$(now_ns)"
    durations_ms+=( "$(( (end - start) / 1000000 ))" )
  done

  mapfile -t sorted < <(printf '%s\n' "${durations_ms[@]}" | sort -n)
  count=${#sorted[@]}
  min=${sorted[0]}
  max=${sorted[$((count - 1))]}
  sum=0
  for v in "${sorted[@]}"; do sum=$((sum + v)); done
  mean=$((sum / count))
  # Nearest-rank p95 (1-indexed), clamped to the last element.
  p95_index=$(( (count * 95 + 99) / 100 - 1 ))
  [[ ${p95_index} -ge ${count} ]] && p95_index=$((count - 1))
  p95=${sorted[${p95_index}]}

  echo "  ${name}: min=${min}ms mean=${mean}ms p95=${p95}ms max=${max}ms (n=${count})"

  profile_entries+=( "\"helm_render_${name}\": {\"min_ms\": ${min}, \"mean_ms\": ${mean}, \"p95_ms\": ${p95}, \"max_ms\": ${max}}" )
done

mkdir -p "$(dirname "${OUTPUT}")"
{
  echo "{"
  echo "  \"metrics\": {"
  last=$((${#profile_entries[@]} - 1))
  for idx in "${!profile_entries[@]}"; do
    sep=","
    [[ ${idx} -eq ${last} ]] && sep=""
    echo "    ${profile_entries[${idx}]}${sep}"
  done
  echo "  }"
  echo "}"
} > "${OUTPUT}"

echo ""
echo "✓ Wrote ${OUTPUT}"
