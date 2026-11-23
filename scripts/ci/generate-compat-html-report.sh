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
#
# Generates an HTML compatibility matrix report from compat-matrix-report.json.

set -euo pipefail

OUTPUT="docs/compatibility-matrix.html"
JSON_FILE="compat-matrix-report.json"
K8S_VERSIONS=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      OUTPUT="$2"
      shift 2
      ;;
    --json)
      JSON_FILE="$2"
      shift 2
      ;;
    --k8s-versions)
      K8S_VERSIONS="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

mkdir -p "$(dirname "${OUTPUT}")"

PASSED=$(jq -r '.summary.passed // 0' "${JSON_FILE}" 2>/dev/null || echo "0")
TOTAL=$(jq -r '.summary.total // 0' "${JSON_FILE}" 2>/dev/null || echo "0")
FAILED=$(jq -r '.summary.failed // 0' "${JSON_FILE}" 2>/dev/null || echo "0")

cat > "${OUTPUT}" <<EOF
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <title>Stellar-K8s Kubernetes Compatibility Matrix</title>
  <style>
    body { font-family: system-ui, sans-serif; margin: 2rem; color: #1f2937; }
    .summary { display: flex; gap: 1.5rem; margin-bottom: 1.5rem; }
    .card { padding: 1rem 1.25rem; border: 1px solid #e5e7eb; border-radius: 8px; }
  </style>
</head>
<body>
  <h1>Stellar-K8s Kubernetes Version Skew Compatibility Matrix</h1>
  <p>Versions tested: ${K8S_VERSIONS}</p>
  <div class="summary">
    <div class="card"><strong>Passed:</strong> ${PASSED}</div>
    <div class="card"><strong>Failed:</strong> ${FAILED}</div>
    <div class="card"><strong>Total:</strong> ${TOTAL}</div>
  </div>
</body>
</html>
EOF

echo "Generated HTML compatibility report at ${OUTPUT}"
