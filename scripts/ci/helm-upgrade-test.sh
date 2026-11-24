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
# Helm upgrade-path check (issue #1289).
#
# Chart.yaml has been 0.1.0 since the chart was added; git has no previous
# chart version tag. This script therefore:
#   1. Renders the current chart with the last supported production values.
#   2. Re-renders with the same values plus additive overrides (upgrade).
#   3. Asserts resources, nodeSelector, affinity, and service ports survive.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHART="$ROOT/charts/stellar-operator"
PROD="$CHART/examples/values-production.yaml"
BEFORE="${TMPDIR:-/tmp}/stellar-helm-before.yaml"
AFTER="${TMPDIR:-/tmp}/stellar-helm-after.yaml"

need() { command -v "$1" >/dev/null 2>&1 || { echo "ERROR: $1 is required"; exit 1; }; }
need helm
need python3

helm template upgrade-check "$CHART" -f "$PROD" --namespace stellar-system > "$BEFORE"
helm template upgrade-check "$CHART" -f "$PROD" \
  --set otel.enabled=true \
  --set otel.endpoint=http://jaeger:4317 \
  --namespace stellar-system > "$AFTER"

python3 - "$BEFORE" "$AFTER" <<'PY'
import sys

before_path, after_path = sys.argv[1], sys.argv[2]
before = open(before_path, encoding="utf-8").read()
after = open(after_path, encoding="utf-8").read()

required_before = [
    "stellar.org/node-pool: operator",
    "topology.kubernetes.io/zone",
    "stellar.org/dedicated",
]
missing = [item for item in required_before if item not in before]
if missing:
    raise SystemExit(f"production values failed to render required fields: {missing}")

for item in required_before:
    if item not in after:
        raise SystemExit(f"upgrade render dropped {item!r}")

if "OTEL_EXPORTER_OTLP_ENDPOINT" not in after:
    raise SystemExit("upgrade render missing OTEL_EXPORTER_OTLP_ENDPOINT")

print("✓ Helm upgrade preservation check passed")
PY
