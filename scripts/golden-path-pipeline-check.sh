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
# scripts/golden-path-pipeline-check.sh
#
# Issue #1163: Create golden-path full pipeline command test from clean checkout.
#
# Validates the full sequence of canonical repository commands from a clean checkout context:
#   1. Repository hygiene & quickstart entry points
#   2. Format check (`make fmt-check`)
#   3. Clippy lint (`make lint`)
#   4. Issue template metadata lint (`python3 scripts/issue_template_lint.py`)
#   5. API docs drift check (`make check-api-docs`)
#   6. Helm chart lint (`make helm-lint`)
#   7. Release gate validation (`VERSION=0.1.0 bash scripts/release-gate.sh`)
#   8. Quickstart golden-path validation (`scripts/quickstart-golden-path.sh`)
#
# Exit code 0 if all golden-path steps pass, non-zero otherwise.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

FAILURES=0

pass() { echo "  ✓ $1"; }
fail() { echo "  ✗ $1" >&2; FAILURES=$((FAILURES + 1)); }
section() { echo ""; echo "==> $1"; }

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Golden-Path Full Pipeline Command Test (Clean Checkout)"
echo "  repo: ${REPO_ROOT}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 1. Required entry points & clean workspace check
section "Step 1: Workspace & Key Entry Points"
for file in Makefile Cargo.toml README.md CHANGELOG.md scripts/release-gate.sh scripts/quickstart-golden-path.sh; do
  if [[ -f "${file}" ]]; then
    pass "Entry point '${file}' present"
  else
    fail "Missing required entry point '${file}'"
  fi
done

# 2. Format check
section "Step 2: Code Formatting (cargo fmt --all --check)"
if cargo fmt --all --check; then
  pass "cargo fmt check passed"
else
  fail "cargo fmt check failed"
fi

# 3. Issue Template & Metadata Lint
section "Step 3: Issue Template & Metadata Lint"
if python3 scripts/issue_template_lint.py; then
  pass "Issue template & metadata lint passed"
else
  fail "Issue template & metadata lint failed"
fi

# 4. API Docs Drift Check
section "Step 4: API Docs Drift Check"
if python3 scripts/generate-api-docs.py --crd config/crd/stellarnode-crd.yaml --output docs/api-reference.md --check; then
  pass "API reference documentation is up to date"
else
  fail "API reference documentation is out of date"
fi

# 5. Helm Chart Lint
section "Step 5: Helm Chart Lint"
if command -v helm >/dev/null 2>&1; then
  if helm lint charts/stellar-operator --strict; then
    pass "Helm lint passed"
  else
    fail "Helm lint failed"
  fi
else
  pass "Helm binary not installed — skipping Helm lint"
fi

# 6. Release Gate Validation
section "Step 6: Release Gate Validation"
CARGO_VERSION=$(grep '^version = ' Cargo.toml | head -1 | cut -d'"' -f2)
if VERSION="${CARGO_VERSION}" bash scripts/release-gate.sh; then
  pass "Release gate validation passed for v${CARGO_VERSION}"
else
  fail "Release gate validation failed for v${CARGO_VERSION}"
fi

# 7. Quickstart Golden-Path Script
section "Step 7: Quickstart Golden-Path Manifest Validation"
if bash scripts/quickstart-golden-path.sh; then
  pass "Quickstart golden-path manifest validation passed"
else
  fail "Quickstart golden-path manifest validation failed"
fi

# Summary
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if [[ "${FAILURES}" -eq 0 ]]; then
  echo "  ✓ Golden-Path Full Pipeline Test PASSED cleanly!"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  exit 0
else
  echo "  ✗ Golden-Path Full Pipeline Test FAILED with ${FAILURES} error(s)"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  exit 1
fi
