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
# Dedicated YAML lint + CRD schema drift + Helm-render kubeconform gate (issue #1291).
#
# Repository-wide Kubernetes/CR YAML structure is still enforced by
# scripts/validate-yaml-manifests.py (issue #1044, `make validate-yaml`).
# This script adds yamllint, generated CRD JSON schema drift, and kubeconform
# against Helm-rendered manifests.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

KUBECONFORM_VERSION="${KUBECONFORM_VERSION:-v0.6.4}"
KUBERNETES_VERSION="${KUBERNETES_VERSION:-1.30.0}"
RENDERED="${TMPDIR:-/tmp}/stellar-k8s-rendered-chart.yaml"
YAMLLINT_CONFIG="${YAMLLINT_CONFIG:-.yamllint.yml}"

die() { echo "ERROR: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required"; }

echo "=== 1. yamllint ==="
need yamllint
yamllint -c "$YAMLLINT_CONFIG" .
echo "✓ yamllint passed"

echo "=== 2. CRD JSON schema drift ==="
need python3
python3 - <<'PY'
import importlib.util, sys
if importlib.util.find_spec("yaml") is None:
    sys.exit("PyYAML is required: pip install pyyaml")
PY
python3 scripts/ci/extract-crd-json-schemas.py --check
echo "✓ CRD JSON schemas match config/crd/"

echo "=== 3. Render Helm chart ==="
need helm
helm template stellar-operator charts/stellar-operator \
  --namespace stellar-system \
  > "$RENDERED"
test -s "$RENDERED" || die "helm template produced an empty manifest"
echo "✓ Helm chart rendered to $RENDERED"

echo "=== 4. kubeconform (rendered chart + CRDs) ==="
need kubeconform
SCHEMA_FLAGS=(
  -strict
  -summary
  -kubernetes-version "${KUBERNETES_VERSION}"
  -schema-location default
  -schema-location "${ROOT}/schemas/crd/{{ .ResourceKind }}{{ .KindSuffix }}"
)
kubeconform "${SCHEMA_FLAGS[@]}" "$RENDERED"
kubeconform -strict -summary -kubernetes-version "${KUBERNETES_VERSION}" config/crd/*.yaml
echo "✓ kubeconform passed"

echo "✓ YAML / schema validation passed"
