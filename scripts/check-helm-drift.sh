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
# check-helm-drift.sh — Automated drift detection between Helm templates and
# rendered manifests (issue #1045, enhanced in #1395).
#
# Why this exists
# ---------------
# scripts/check-chart-diff.sh compares a render against a baseline kept in
# .cache/, which is gitignored. In CI that directory is always empty, so the
# baseline is (re)created on every run and the comparison never actually
# happens — drift detection that structurally cannot fail.
#
# This script stores the baseline in git instead: charts/stellar-operator/
# rendered/<profile>.yaml is committed, reviewed like any other file, and
# diffed on every run. A template change that alters rendered output shows up
# as a concrete manifest diff in the pull request.
#
# Renders are normalised through scripts/sort-manifests.py, which sorts
# documents and recursively sorts mapping keys, so the goldens are stable
# regardless of Helm's map iteration order.
#
# Usage:
#   scripts/check-helm-drift.sh              # verify; non-zero on drift
#   scripts/check-helm-drift.sh --update     # regenerate the golden files
#   scripts/check-helm-drift.sh --profile ha # restrict to one profile
#   scripts/check-helm-drift.sh --list       # show the configured profiles
#   scripts/check-helm-drift.sh --check-high-risk  # detect high-risk field changes
#
# Exit codes: 0 = no drift, 1 = drift or render failure, 2 = bad invocation.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CHART_DIR="${PROJECT_ROOT}/charts/stellar-operator"
GOLDEN_DIR="${CHART_DIR}/rendered"
SORTER="${SCRIPT_DIR}/sort-manifests.py"
RELEASE_NAME="stellar-operator"
RELEASE_NAMESPACE="stellar-system"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# ── Render profiles ───────────────────────────────────────────────────────────
# Each entry is "<name>|<extra helm args>". Keep the list in sync with the
# documented values files; a profile with no golden file is treated as drift
# so that adding a values file cannot silently escape coverage.
PROFILES=(
  "default|"
  "ha|-f ${CHART_DIR}/values-ha.yaml"
  "production|-f ${CHART_DIR}/examples/values-production.yaml"
  "development|-f ${CHART_DIR}/examples/values-development.yaml"
  "dr-cross-region|--set featureFlags.enableDr=true --set crossRegion.enabled=true --set crossRegion.peerClusters[0].clusterId=us-west-2 --set crossRegion.peerClusters[0].enabled=true --set crossRegion.peerClusters[0].endpoint=api.us-west-2.example.com --set crossRegion.peerClusters[0].region=us-west-2"
)

# ── High-risk YAML paths ──────────────────────────────────────────────────────
# Paths (yq-style) that warrant an elevated alert when changed. Detected via
# yq on the unified diff context or via grep on the rendered files directly.
# These correspond to fields that, if changed accidentally, could cause
# outages or security regressions in production.
HIGH_RISK_PATTERNS=(
  'image:'
  'replicaCount:'
  'resources:'
  'rules:'
  'secrets:'
  'serviceAccount:'
  'securityContext:'
  'podSecurityContext:'
  'automountServiceAccountToken:'
)

UPDATE=0
ONLY_PROFILE=""
MAX_CHANGED_LINES=0
CHECK_HIGH_RISK=0
DRIFTED=0
FAILED=0
HIGH_RISK_DRIFTED=0

usage() {
  sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --update) UPDATE=1; shift ;;
    --profile) ONLY_PROFILE="${2:?--profile needs a name}"; shift 2 ;;
    --max-changed-lines) MAX_CHANGED_LINES="${2:?--max-changed-lines needs a number}"; shift 2 ;;
    --check-high-risk) CHECK_HIGH_RISK=1; shift ;;
    --list)
      for entry in "${PROFILES[@]}"; do echo "${entry%%|*}"; done
      exit 0 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

command -v helm >/dev/null 2>&1 || { echo "ERROR: helm is not installed" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "ERROR: python3 is not installed" >&2; exit 1; }
[[ -d "${CHART_DIR}" ]] || { echo "ERROR: chart directory not found: ${CHART_DIR}" >&2; exit 1; }

# Detect dyff (YAML-aware diff) availability
HAS_DYFF=0
if command -v dyff >/dev/null 2>&1; then
  HAS_DYFF=1
fi

TEMP_DIR="$(mktemp -d)"
cleanup() { rm -rf "${TEMP_DIR}"; }
trap cleanup EXIT

mkdir -p "${GOLDEN_DIR}"

echo "→ Helm template drift detection"
echo "  chart:    ${CHART_DIR#"${PROJECT_ROOT}"/}"
echo "  goldens:  ${GOLDEN_DIR#"${PROJECT_ROOT}"/}"
echo "  dyff:     $([ "${HAS_DYFF}" -eq 1 ] && echo 'available' || echo 'not found — falling back to diff')"
echo "  high-risk checks: $([ "${CHECK_HIGH_RISK}" -eq 1 ] && echo 'enabled' || echo 'disabled')"
echo ""

render_profile() {
  # render_profile <name> <extra-args> <destination>
  local name="$1" extra="$2" dest="$3"
  local stderr_file="${TEMP_DIR}/${name}.stderr"

  # shellcheck disable=SC2086 # $extra intentionally word-splits into helm flags
  if ! helm template "${RELEASE_NAME}" "${CHART_DIR}" \
        --namespace "${RELEASE_NAMESPACE}" \
        ${extra} 2>"${stderr_file}" \
      | python3 "${SORTER}" > "${dest}" 2>>"${stderr_file}"; then
    echo -e "  ${RED}✗${NC} ${name}: render failed"
    sed 's/^/      /' "${stderr_file}"
    return 1
  fi

  if [[ ! -s "${dest}" ]]; then
    echo -e "  ${RED}✗${NC} ${name}: render produced no output"
    return 1
  fi
  return 0
}

# ── Diff two YAML files, preferring dyff when available ──────────────────────
# Outputs the diff to stdout and returns 0 if identical, 1 if different.
yaml_diff() {
  # yaml_diff <left> <right> <output_file>
  local left="$1" right="$2" output="$3"

  if [[ "${HAS_DYFF}" -eq 1 ]]; then
    dyff between --output human "${left}" "${right}" > "${output}" 2>/dev/null || true
  else
    diff -u "${left}" "${right}" > "${output}" 2>&1 || true
  fi

  [[ -s "${output}" ]]
}

# ── Check for high-risk field changes in a diff ──────────────────────────────
# Scans the diff output for lines that modify high-risk YAML keys.
# Returns the count of high-risk changed lines.
check_high_risk_drift() {
  # check_high_risk_diff <diff_file>
  local diff_file="$1"
  local count=0

  for pattern in "${HIGH_RISK_PATTERNS[@]}"; do
    local matches
    matches="$(grep -c "^[+-].*${pattern}" "${diff_file}" 2>/dev/null || true)"
    count=$((count + matches))
  done

  echo "${count}"
}

# ── Generate $GITHUB_STEP_SUMMARY if available ───────────────────────────────
write_summary() {
  # write_summary <title> <body>
  local title="$1" body="$2"
  if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    {
      echo "## ${title}"
      echo ""
      echo "${body}"
    } >> "${GITHUB_STEP_SUMMARY}"
  fi
}

for entry in "${PROFILES[@]}"; do
  name="${entry%%|*}"
  extra="${entry#*|}"

  if [[ -n "${ONLY_PROFILE}" && "${name}" != "${ONLY_PROFILE}" ]]; then
    continue
  fi

  golden="${GOLDEN_DIR}/${name}.yaml"
  actual="${TEMP_DIR}/${name}.yaml"

  if ! render_profile "${name}" "${extra}" "${actual}"; then
    FAILED=$((FAILED + 1))
    continue
  fi

  docs="$(grep -c '^---$' "${actual}" || true)"

  if [[ "${UPDATE}" -eq 1 ]]; then
    cp "${actual}" "${golden}"
    echo -e "  ${GREEN}✓${NC} ${name}: golden updated (${docs} documents)"
    continue
  fi

  if [[ ! -f "${golden}" ]]; then
    echo -e "  ${RED}✗${NC} ${name}: no golden file at ${golden#"${PROJECT_ROOT}"/}"
    echo "      run: scripts/check-helm-drift.sh --update"
    DRIFTED=$((DRIFTED + 1))
    continue
  fi

  diff_file="${TEMP_DIR}/${name}.diff"
  if yaml_diff "${golden}" "${actual}" "${diff_file}"; then
    echo -e "  ${GREEN}✓${NC} ${name}: no drift (${docs} documents)"
  else
    changed="$(grep -c '^[+-]' "${diff_file}" || true)"
    echo -e "  ${RED}✗${NC} ${name}: rendered output drifted from the golden file (${changed} changed lines)"

    high_risk_count=0
    if [[ "${CHECK_HIGH_RISK}" -eq 1 ]]; then
      high_risk_count="$(check_high_risk_drift "${diff_file}")"
      if [[ "${high_risk_count}" -gt 0 ]]; then
        echo -e "      ${RED}⚠ HIGH-RISK FIELD DRIFT: ${high_risk_count} changes to image/replica/resources/RBAC/secrets${NC}"
        HIGH_RISK_DRIFTED=$((HIGH_RISK_DRIFTED + 1))
      fi
    fi

    if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
      echo "::error file=charts/stellar-operator/rendered/${name}.yaml::Helm template drift detected in profile '${name}' (${changed} changed lines)."
      if [[ "${high_risk_count}" -gt 0 ]]; then
        echo "::warning file=charts/stellar-operator/rendered/${name}.yaml::High-risk field drift detected in profile '${name}' (${high_risk_count} changes to image/replica/resources/RBAC/secrets)."
      fi
    fi
    if [[ "${MAX_CHANGED_LINES}" -gt 0 && "${changed}" -gt "${MAX_CHANGED_LINES}" ]]; then
      echo -e "      ${RED}ALERT: Changed lines (${changed}) exceeds acceptable threshold (${MAX_CHANGED_LINES})${NC}"
    fi
    echo ""
    if [[ "${HAS_DYFF}" -eq 1 ]]; then
      head -80 "${diff_file}" | sed 's/^/      /'
    else
      head -80 "${diff_file}" | sed 's/^/      /'
    fi
    if [[ "${changed}" -gt 80 ]]; then
      echo "      … diff truncated; run 'dyff between ${golden#"${PROJECT_ROOT}"/} <(helm template …)' for the full output"
    fi
    echo ""
    DRIFTED=$((DRIFTED + 1))
  fi
done

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if [[ "${UPDATE}" -eq 1 ]]; then
  echo "Helm Drift: goldens regenerated"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo ""
  echo "Review the diff with 'git diff ${GOLDEN_DIR#"${PROJECT_ROOT}"/}' before committing."
  exit 0
fi

echo -e "Helm Drift Summary:  drifted: ${DRIFTED}   render failures: ${FAILED}"
if [[ "${CHECK_HIGH_RISK}" -eq 1 ]]; then
  echo "  high-risk drift:  ${HIGH_RISK_DRIFTED} profile(s) with image/replica/resources/RBAC/secrets changes"
fi
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

if [[ "${DRIFTED}" -gt 0 || "${FAILED}" -gt 0 ]]; then
  echo ""
  echo -e "${RED}❌ Helm template drift detected${NC}"
  echo ""
  if [[ "${HIGH_RISK_DRIFTED}" -gt 0 ]]; then
    echo -e "   ${YELLOW}⚠ ${HIGH_RISK_DRIFTED} profile(s) contain changes to HIGH-RISK fields (image, replicas, resources, RBAC, secrets).${NC}"
    echo "     Review carefully before regenerating goldens."
    echo ""
  fi
  echo "   If the change is intentional, regenerate and commit the goldens:"
  echo "     make helm-drift-update"
  echo "     git add ${GOLDEN_DIR#"${PROJECT_ROOT}"/}"
  if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    write_summary "Helm Drift Detection" \
      "Drift detected in ${DRIFTED} profile(s) (${FAILED} render failures).
      $([ "${HIGH_RISK_DRIFTED}" -gt 0 ] && echo "**⚠ ${HIGH_RISK_DRIFTED} profile(s) contain high-risk field changes.**" || echo "")
      Run \`make helm-drift-update\` to regenerate goldens if the changes are intentional."
  fi
  exit 1
fi

echo ""
echo -e "${GREEN}✅ Rendered manifests match the committed goldens${NC}"
exit 0
