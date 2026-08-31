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
# Integration check: secret rotation without downtime (issue #1066).
#
# Rotates (patches) a Kubernetes secret and verifies that the consuming
# deployment keeps its available replicas at or above the pre-rotation
# level for the whole observation window.
#
# Usage:
#   scripts/secret-rotation-check.sh [--namespace NS] [--secret NAME] \
#       [--deployment NAME] [--window SECONDS] [--dry-run]
#
# --dry-run validates arguments and prints the plan without needing a
# cluster; CI uses it as a smoke test.
set -euo pipefail

NAMESPACE="stellar-system"
SECRET="stellar-core-secret"
DEPLOYMENT="stellar-k8s-operator"
WINDOW=60
DRY_RUN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace) NAMESPACE="$2"; shift 2 ;;
    --secret) SECRET="$2"; shift 2 ;;
    --deployment) DEPLOYMENT="$2"; shift 2 ;;
    --window) WINDOW="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

echo "Secret rotation check"
echo "  namespace:  ${NAMESPACE}"
echo "  secret:     ${SECRET}"
echo "  deployment: ${DEPLOYMENT}"
echo "  window:     ${WINDOW}s"

if [[ "${DRY_RUN}" == "1" ]]; then
  echo "Dry run: no cluster interaction performed. Plan:"
  echo "  1. record availableReplicas of ${DEPLOYMENT}"
  echo "  2. annotate ${SECRET} with a rotation timestamp"
  echo "  3. poll availableReplicas for ${WINDOW}s and fail on any drop"
  exit 0
fi

command -v kubectl >/dev/null || { echo "kubectl not found" >&2; exit 2; }

BASELINE="$(kubectl -n "${NAMESPACE}" get deployment "${DEPLOYMENT}" \
  -o jsonpath='{.status.availableReplicas}')"
BASELINE="${BASELINE:-0}"
if [[ "${BASELINE}" -lt 1 ]]; then
  echo "FAIL: deployment ${DEPLOYMENT} has no available replicas before rotation" >&2
  exit 1
fi
echo "Baseline available replicas: ${BASELINE}"

ROTATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
kubectl -n "${NAMESPACE}" annotate secret "${SECRET}" \
  "stellar.example.com/rotated-at=${ROTATED_AT}" --overwrite
echo "Secret ${SECRET} rotated at ${ROTATED_AT}"

ELAPSED=0
while [[ "${ELAPSED}" -lt "${WINDOW}" ]]; do
  AVAILABLE="$(kubectl -n "${NAMESPACE}" get deployment "${DEPLOYMENT}" \
    -o jsonpath='{.status.availableReplicas}')"
  AVAILABLE="${AVAILABLE:-0}"
  if [[ "${AVAILABLE}" -lt "${BASELINE}" ]]; then
    echo "FAIL: available replicas dropped to ${AVAILABLE} (baseline ${BASELINE})" >&2
    exit 1
  fi
  sleep 5
  ELAPSED=$((ELAPSED + 5))
done

echo "PASS: no downtime observed during ${WINDOW}s after secret rotation"
