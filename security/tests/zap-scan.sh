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
# OWASP ZAP Baseline Penetration Test for Stellar-K8s Operator API
# Usage: ./zap-scan.sh http://localhost:9090

set -euo pipefail

TARGET_URL=${1:-http://localhost:9090}
REPORT_DIR="security/reports"
mkdir -p "${REPORT_DIR}"

docker run -t --rm \
  -v "${REPORT_DIR}":/reports \
  -e TARGET="${TARGET_URL}" \
  -e AUTOSEED_URL="${TARGET_URL}" \
  --user root \
  ghcr.io/zaproxy/zap-stable \
  zap-baseline.py \
    -t "${TARGET_URL}" \
    -r /reports/zap-baseline.html \
    -w /reports/zap-baseline.xml \
    --auto-seed \
    -J /reports/zap-json-report.json

echo "✅ ZAP scan complete. Reports in ${REPORT_DIR}/"
echo "HTML: ${REPORT_DIR}/zap-baseline.html"

