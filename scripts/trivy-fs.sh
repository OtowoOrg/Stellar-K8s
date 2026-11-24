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
# scripts/trivy-fs.sh — Local Trivy filesystem + IaC scan.
#
# Mirrors the `.github/workflows/vulnerability-scan.yml` CI jobs so
# contributors get the same findings before opening a PR. Advisory-only:
# findings are printed, never fail the build.
#
# Usage:
#   bash scripts/trivy-fs.sh                       # filesystem + IaC scans
#   bash scripts/trivy-fs.sh --severity HIGH,CRITICAL
#   bash scripts/trivy-fs.sh --fs-only | --config-only
#
# Requires: trivy (https://aquasecurity.github.io/trivy/latest/getting-started/installation/)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}" || exit 1

SEVERITY="CRITICAL,HIGH"
MODE="all"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --severity)
      SEVERITY="${2:?--severity requires a value}"
      shift 2
      ;;
    --fs-only)
      MODE="fs"
      shift
      ;;
    --config-only)
      MODE="config"
      shift
      ;;
    -h|--help)
      sed -n '14,24p' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1 (see --help)" >&2
      exit 1
      ;;
  esac
done

if ! command -v trivy >/dev/null 2>&1; then
  echo "error: trivy not found." >&2
  echo "Install it first: https://aquasecurity.github.io/trivy/latest/getting-started/installation/" >&2
  exit 1
fi

if [[ "${MODE}" == "all" || "${MODE}" == "fs" ]]; then
  echo "--- Trivy filesystem scan (severity: ${SEVERITY}) ---"
  trivy fs --severity "${SEVERITY}" --exit-code 0 .
fi

if [[ "${MODE}" == "all" || "${MODE}" == "config" ]]; then
  echo "--- Trivy IaC scan (severity: ${SEVERITY}) ---"
  trivy config --severity "${SEVERITY}" --exit-code 0 .
fi

echo "Done. Findings above are advisory; track them via the Security tab."
