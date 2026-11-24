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
# scripts/ci/check-python-toml-lint.sh — Stdlib-only syntax gate for Python
# tools and TOML configs.
#
# Checks:
#   • Every scripts/**/*.py file compiles (python3 -m py_compile)
#   • Every tracked *.toml file parses (python3 stdlib tomllib, 3.11+)
#
# No third-party dependencies and no network access, so the gate runs in
# ~30s on any runner with Python 3.11+.
#
# Usage:
#   bash scripts/ci/check-python-toml-lint.sh
#
# Make target: make python-lint
# CI job:      .github/workflows/ci.yml → python-toml-lint

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

ERRORS=0

echo "=== Python & TOML lint ==="

# ── 1. Python syntax (py_compile, stdlib only) ────────────────────────────────
echo "→ Checking Python tools compile (py_compile)..."
PY_FILES=()
while IFS= read -r -d '' f; do
  PY_FILES+=("$f")
done < <(find "$ROOT/scripts" -type f -name "*.py" -print0 2>/dev/null || true)

if [[ "${#PY_FILES[@]}" -eq 0 ]]; then
  echo "  (no Python files found — skipping)"
else
  PY_FAILED=0
  for f in "${PY_FILES[@]}"; do
    if ! python3 -m py_compile "$f" 2>&1; then
      echo "  ✗ py_compile failed: ${f#"$ROOT"/}" >&2
      PY_FAILED=$((PY_FAILED + 1))
    fi
  done
  if [[ "$PY_FAILED" -eq 0 ]]; then
    echo "  ✓ ${#PY_FILES[@]} Python file(s) compile"
  else
    echo "  ✗ $PY_FAILED Python file(s) failed py_compile" >&2
    ERRORS=$((ERRORS + PY_FAILED))
  fi
fi

# ── 2. TOML parse (tomllib, stdlib only, Python 3.11+) ───────────────────────
echo "→ Checking TOML configs parse (tomllib)..."
if ! python3 -c "import tomllib" 2>/dev/null; then
  echo "  (tomllib unavailable — Python <3.11 — skipping TOML parse check)"
else
  TOML_FILES=()
  if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    while IFS= read -r f; do
      [[ -n "$f" ]] && TOML_FILES+=("$ROOT/$f")
    done < <(git ls-files '*.toml' 2>/dev/null || true)
  fi
  if [[ "${#TOML_FILES[@]}" -eq 0 ]]; then
    while IFS= read -r -d '' f; do
      TOML_FILES+=("$f")
    done < <(find "$ROOT" -name "*.toml" -not -path "$ROOT/target/*" -not -path "$ROOT/node_modules/*" -not -path "$ROOT/.git/*" -print0 2>/dev/null || true)
  fi
  if [[ "${#TOML_FILES[@]}" -eq 0 ]]; then
    echo "  (no TOML files found — skipping)"
  else
    TOML_FAILED=0
    for f in "${TOML_FILES[@]}"; do
      if ! python3 -c "import sys, tomllib; tomllib.load(open(sys.argv[1], 'rb'))" "$f" 2>&1; then
        echo "  ✗ TOML parse failed: ${f#"$ROOT"/}" >&2
        TOML_FAILED=$((TOML_FAILED + 1))
      fi
    done
    if [[ "$TOML_FAILED" -eq 0 ]]; then
      echo "  ✓ ${#TOML_FILES[@]} TOML file(s) parse"
    else
      echo "  ✗ $TOML_FAILED TOML file(s) failed to parse" >&2
      ERRORS=$((ERRORS + TOML_FAILED))
    fi
  fi
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
if [[ "$ERRORS" -eq 0 ]]; then
  echo "✓ Python & TOML lint passed"
  exit 0
else
  echo "✗ $ERRORS Python/TOML lint failure(s)" >&2
  echo "  Fix syntax errors above, then re-run: make python-lint" >&2
  exit 1
fi
