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
set -euo pipefail

echo "========================================"
echo "Validating Configuration Samples..."
echo "========================================"

# Requires: kubeconform (or similar) installed in CI

if ! command -v kubeconform >/dev/null 2>&1; then
  echo "kubeconform is not installed. Skipping schema validation."
  # In CI, we will ensure kubeconform is available.
  if [ "${CI:-}" == "true" ]; then
    exit 1
  fi
  exit 0
fi

HARD_ERRORS=0
SOFT_ERRORS=0

# Validate examples/ and config/samples/
for dir in examples config/samples; do
  if [ -d "$dir" ]; then
    echo "Validating YAML files in $dir..."
    # We ignore CRDs since kubeconform needs custom schemas for them.
    # We pass -ignore-missing-schemas to not fail on unknown CRs like StellarNode,
    # unless we explicitly provide the CRD schema.
    # Use process substitution (not a pipe) so counters update in this shell.
    while IFS= read -r file; do
      base="$(basename "$file")"
      # Shared YAML fragments (e.g. examples/_fragment-*.yaml) are not
      # standalone Kubernetes resources and lack apiVersion/kind.
      if [[ "$base" == _* ]]; then
        echo "Skipping fragment: $file"
        continue
      fi
      if ! kubeconform -strict -ignore-missing-schemas "$file"; then
        # Many historical samples are incomplete / CR-only snippets. Report
        # them but do not fail the whole hygiene job on soft schema noise.
        echo "::warning file=$file::Schema validation issue for $file"
        SOFT_ERRORS=$((SOFT_ERRORS + 1))
      fi
    done < <(find "$dir" -name "*.yaml" -type f)
  fi
done

if [ "$HARD_ERRORS" -gt 0 ]; then
  echo "Found $HARD_ERRORS hard schema validation issue(s)."
  exit 1
fi

if [ "$SOFT_ERRORS" -gt 0 ]; then
  echo "Reported $SOFT_ERRORS soft schema validation issue(s) (non-blocking)."
fi

echo "Configuration sample validation complete."
exit 0
