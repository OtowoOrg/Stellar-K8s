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
# collect-failure-diagnostics.sh
#
# Assemble a unified diagnostics artifact bundle for failing CI runs.
# Closes Issue #1151.
#
# Layout written under BUNDLE_DIR (default: /tmp/ci-diagnostics):
#
#   manifest.json          — run metadata (job, sha, timestamp, hostname)
#   summary.txt            — human-readable triage summary
#   cluster/               — kubectl dumps (skipped when kubectl missing
#                            or --no-cluster is passed)
#   extras/                — copies of caller-supplied paths
#   env/                   — sanitized environment snapshot
#
# Exit codes
# ----------
#   0  — Bundle assembled (even if some collectors soft-failed)
#   2  — Usage / tooling error
#
# Usage:
#   ./scripts/ci/collect-failure-diagnostics.sh
#   ./scripts/ci/collect-failure-diagnostics.sh --bundle-dir /tmp/foo
#   ./scripts/ci/collect-failure-diagnostics.sh --extra /tmp/job.log --extra /tmp/triage/
#   ./scripts/ci/collect-failure-diagnostics.sh --no-cluster
#   ./scripts/ci/collect-failure-diagnostics.sh --operator-namespace stellar-system

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

BUNDLE_DIR="${BUNDLE_DIR:-/tmp/ci-diagnostics}"
OPERATOR_NAMESPACE="${OPERATOR_NAMESPACE:-stellar-system}"
EXTRA_NS="${EXTRA_NS:-}"
INCLUDE_CLUSTER=true
EXTRA_PATHS=()
JOB_NAME="${JOB_NAME:-local}"
RUN_ID="${RUN_ID:-local}"
SHA="${SHA:-$(git rev-parse HEAD 2>/dev/null || echo unknown)}"

usage() {
  sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
}

json_escape() {
  # Minimal JSON string escape for ASCII metadata fields.
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/'$'\t''/\\t/g'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle-dir)
      BUNDLE_DIR="${2:?--bundle-dir requires a path}"
      shift 2
      ;;
    --operator-namespace)
      OPERATOR_NAMESPACE="${2:?--operator-namespace requires a value}"
      shift 2
      ;;
    --extra-namespaces)
      EXTRA_NS="${2:-}"
      shift 2
      ;;
    --extra)
      EXTRA_PATHS+=("${2:?--extra requires a path}")
      shift 2
      ;;
    --job-name)
      JOB_NAME="${2:?--job-name requires a value}"
      shift 2
      ;;
    --run-id)
      RUN_ID="${2:?--run-id requires a value}"
      shift 2
      ;;
    --sha)
      SHA="${2:?--sha requires a value}"
      shift 2
      ;;
    --no-cluster)
      INCLUDE_CLUSTER=false
      shift
      ;;
    --help|-h)
      usage
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      ;;
  esac
done

mkdir -p "$BUNDLE_DIR"/{cluster,extras,env}

TIMESTAMP="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
HOSTNAME_VAL="$(hostname 2>/dev/null || echo unknown)"
CLUSTER_JSON=false
[[ "$INCLUDE_CLUSTER" == "true" ]] && CLUSTER_JSON=true

cat >"$BUNDLE_DIR/manifest.json" <<EOF
{
  "schema": "stellar-k8s.ci-diagnostics/v1",
  "issue": 1151,
  "timestamp": "$(json_escape "$TIMESTAMP")",
  "job_name": "$(json_escape "$JOB_NAME")",
  "run_id": "$(json_escape "$RUN_ID")",
  "sha": "$(json_escape "$SHA")",
  "hostname": "$(json_escape "$HOSTNAME_VAL")",
  "operator_namespace": "$(json_escape "$OPERATOR_NAMESPACE")",
  "include_cluster": ${CLUSTER_JSON}
}
EOF

{
  echo "=== Stellar-K8s CI Failure Diagnostics Bundle ==="
  echo "timestamp:           ${TIMESTAMP}"
  echo "job_name:            ${JOB_NAME}"
  echo "run_id:              ${RUN_ID}"
  echo "sha:                 ${SHA}"
  echo "hostname:            ${HOSTNAME_VAL}"
  echo "operator_namespace:  ${OPERATOR_NAMESPACE}"
  echo "include_cluster:     ${INCLUDE_CLUSTER}"
  echo "extra_paths:         ${#EXTRA_PATHS[@]}"
  echo ""
  echo "Bundle layout:"
  echo "  manifest.json  — machine-readable metadata"
  echo "  summary.txt    — this file"
  echo "  cluster/       — kubectl dumps (when available)"
  echo "  extras/        — caller-supplied logs and triage dirs"
  echo "  env/           — sanitized environment snapshot"
} >"$BUNDLE_DIR/summary.txt"

{
  echo "# Sanitized environment — secret-like keys omitted"
  env | sort | grep -viE '(SECRET|TOKEN|PASSWORD|PASSWD|CREDENTIAL|PRIVATE_KEY|API_KEY|AUTH|SEED|BEARER|AWS_|GITHUB_TOKEN|NPM_TOKEN|DOCKER_PASSWORD)=' \
    || true
} >"$BUNDLE_DIR/env/sanitized.env" 2>/dev/null || true

collect_cluster() {
  local out="$BUNDLE_DIR/cluster"
  if ! command -v kubectl >/dev/null 2>&1; then
    echo "(kubectl not available — skipping cluster dumps)" | tee -a "$BUNDLE_DIR/summary.txt"
    echo "kubectl_unavailable" >"$out/SKIPPED.txt"
    return 0
  fi

  {
    echo "=== Pods (${OPERATOR_NAMESPACE}) ==="
    kubectl get pods -n "${OPERATOR_NAMESPACE}" -o wide 2>&1 || true
    echo ""
    echo "=== Operator logs ==="
    kubectl logs --selector=app=stellar-operator --namespace="${OPERATOR_NAMESPACE}" --tail=500 2>&1 \
      || kubectl logs -n "${OPERATOR_NAMESPACE}" \
           -l "app.kubernetes.io/name=stellar-operator" --tail=500 2>&1 \
      || true
  } | tee "$out/operator.txt" >>"$BUNDLE_DIR/summary.txt"

  kubectl get stellarnode --all-namespaces -o wide >"$out/stellarnodes.txt" 2>&1 || true
  kubectl get events --all-namespaces --sort-by='.lastTimestamp' 2>/dev/null \
    | tail -200 >"$out/events-all.txt" 2>&1 || true
  kubectl get crd 2>/dev/null | grep -i stellar >"$out/stellar-crds.txt" 2>&1 || true
  kubectl get nodes -o wide >"$out/nodes.txt" 2>&1 || true

  for ns in ${EXTRA_NS}; do
    kubectl get events -n "${ns}" --sort-by='.lastTimestamp' \
      >"$out/events-${ns}.txt" 2>&1 || true
  done

  echo "" >>"$BUNDLE_DIR/summary.txt"
  echo "Cluster dumps written under cluster/" >>"$BUNDLE_DIR/summary.txt"
}

if [[ "$INCLUDE_CLUSTER" == "true" ]]; then
  collect_cluster
else
  echo "(cluster collection disabled via --no-cluster)" >>"$BUNDLE_DIR/summary.txt"
  echo "disabled" >"$BUNDLE_DIR/cluster/SKIPPED.txt"
fi

idx=0
for path in "${EXTRA_PATHS[@]+"${EXTRA_PATHS[@]}"}"; do
  idx=$((idx + 1))
  if [[ ! -e "$path" ]]; then
    echo "warning: extra path missing: $path" | tee -a "$BUNDLE_DIR/summary.txt"
    continue
  fi
  base="$(basename "$path")"
  dest="$BUNDLE_DIR/extras/$(printf '%02d' "$idx")-${base}"
  if [[ -d "$path" ]]; then
    mkdir -p "$dest"
    cp -a "$path"/. "$dest"/ 2>/dev/null || cp -R "$path"/. "$dest"/ || true
  else
    cp -a "$path" "$dest" 2>/dev/null || cp "$path" "$dest" || true
  fi
  echo "extra: $path -> extras/$(basename "$dest")" >>"$BUNDLE_DIR/summary.txt"
done

echo ""
echo "→ Diagnostics bundle ready: $BUNDLE_DIR"
if command -v find >/dev/null 2>&1; then
  find "$BUNDLE_DIR" -type f 2>/dev/null | sort | sed 's|^|  |'
else
  ls -laR "$BUNDLE_DIR"
fi
exit 0
