#!/bin/bash
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
# Aggregate Chaos Drill Results
#
# #1412 — Results tracking for chaos engineering drills.
#
# Scans the drill JSON artifacts produced by `run-chaos-drill.sh` (under
# $RESULTS_DIR, which defaults to ./results/chaos) and emits:
#
#   1. A chronological drill log (date | drill | RTO actual | RTO target |
#      pass/fail | notes) to stdout.
#   2. A refreshable `results/chaos/RESULTS.md` summary used by the monthly
#      drill review (see docs/chaos-drills.md).
#
# Usage:
#   ./scripts/aggregate-chaos-results.sh [results-dir]
#
# Exits non-zero if any recorded drill failed its RTO target.

set -euo pipefail

RESULTS_DIR="${1:-./results/chaos}"
SUMMARY_FILE="${RESULTS_DIR}/RESULTS.md"

if [[ ! -d "${RESULTS_DIR}" ]]; then
    echo "Results directory not found: ${RESULTS_DIR}" >&2
    exit 1
fi

shopt -s nullglob
RESULTS=( "${RESULTS_DIR}"/drill_*.json )

if [[ ${#RESULTS[@]} -eq 0 ]]; then
    echo "No drill results found in ${RESULTS_DIR}"
    exit 0
fi

echo "=== Chaos Drill Log ==="
printf "%-20s %-18s %-10s %-10s %-6s %s\n" \
    "Date" "Drill" "RTO actual" "RTO target" "Pass" "Drill ID"
FAILED=0
TOTAL=0
for file in "${RESULTS[@]}"; do
    TOTAL=$((TOTAL + 1))
    id=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('drill_id',''))" 2>/dev/null || echo "")
    date_iso=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('start_time',''))" 2>/dev/null || echo "")
    drill=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('drill_type',''))" 2>/dev/null || echo "?")
    rto=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('rto_actual_seconds',''))" 2>/dev/null || echo "0")
    target=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('rto_target_seconds',''))" 2>/dev/null || echo "0")
    pass=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('pass',False))" 2>/dev/null || echo "false")

    # Normalise to a short human date when ISO is missing.
    if [[ -z "${date_iso}" || "${date_iso}" == "None" ]]; then
        date_iso=$(stat -c %y "${file}" 2>/dev/null | cut -d. -f1 || date -r "${file}" +%Y-%m-%dT%H:%M:%SZ)
    fi
    short_date="${date_iso:0:10}"

    if [[ "${pass}" != "True" && "${pass}" != "true" ]]; then
        FAILED=$((FAILED + 1))
    fi
    printf "%-20s %-18s %-10s %-10s %-6s %s\n" \
        "${short_date:-?}" "${drill}" "${rto}s" "${target}s" "${pass}" "${id}"
done

echo ""
echo "Total drills: ${TOTAL} | Failed RTO: ${FAILED}"

# ── Refresh the tracked summary file ──────────────────────────────────────────
cat > "${SUMMARY_FILE}" <<EOF
# Chaos Drill Results Tracker

Last aggregated: $(date -u +%Y-%m-%dT%H:%M:%SZ)

| Date | Drill | RTO actual | RTO target | Pass |
|------|-------|------------|------------|------|
EOF
for file in "${RESULTS[@]}"; do
    date_iso=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('start_time',''))" 2>/dev/null || echo "")
    if [[ -z "${date_iso}" || "${date_iso}" == "None" ]]; then
        date_iso=$(stat -c %y "${file}" 2>/dev/null | cut -d. -f1 || date -r "${file}" +%Y-%m-%dT%H:%M:%SZ)
    fi
    drill=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('drill_type',''))" 2>/dev/null || echo "?")
    rto=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('rto_actual_seconds',''))" 2>/dev/null || echo "0")
    target=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('rto_target_seconds',''))" 2>/dev/null || echo "0")
    pass=$(python3 -c "import json,sys;print(json.load(open('${file}')).get('pass',False))" 2>/dev/null || echo "false")
    printf "| %s | %s | %ss | %ss | %s |\n" "${date_iso:0:10}" "${drill}" "${rto}" "${target}" "${pass}" >> "${SUMMARY_FILE}"
done

echo ""
echo "Tracked summary written to ${SUMMARY_FILE}"

if [[ "${FAILED}" -gt 0 ]]; then
    echo "ERROR: ${FAILED} drill(s) did not meet their RTO target." >&2
    exit 1
fi