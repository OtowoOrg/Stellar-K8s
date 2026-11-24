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
# scripts/ci/check-scripts-convention.sh — Scripts naming and placement gate.
#
# Enforces the conventions documented in CONVENTIONS.md §Shell scripts:
#   • Shell scripts must use kebab-case.sh naming (e.g. cleanup.sh, not cleanup_job.sh)
#   • Python tools must use kebab-case.py naming
#     (snake_case.py grandfathered names are listed in LEGACY_SNAKE_CASE below)
#   • No scripts may live at the repository root
#   • CI-only helpers must live under scripts/ci/, not directly in scripts/
#
# Usage:
#   bash scripts/ci/check-scripts-convention.sh          # exit 0 = pass
#   bash scripts/ci/check-scripts-convention.sh --report # print summary, always exit 0
#
# Make target: make scripts-convention-check
# CI job:      .github/workflows/ci.yml  →  scripts-convention

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

REPORT_ONLY=false
[[ "${1:-}" == "--report" ]] && REPORT_ONLY=true

# ── Grandfathered snake_case Python files (pre-convention, kept for compat) ──
LEGACY_SNAKE_CASE=(
  "scripts/issue_template_lint.py"
  "scripts/crd_migration_lint.py"
  "scripts/check_mkdocs_nav.py"
)

is_legacy() {
  local file="$1"
  for legacy in "${LEGACY_SNAKE_CASE[@]}"; do
    [[ "$file" == "$legacy" ]] && return 0
  done
  return 1
}

ERRORS=0
REPORT=()

fail() {
  ERRORS=$((ERRORS + 1))
  REPORT+=("  ✗ $*")
  if [[ "$REPORT_ONLY" == "false" ]]; then
    echo "  ✗ $*" >&2
  fi
}

echo "=== Scripts convention check ==="

# ── 1. No scripts at the repository root ─────────────────────────────────────
echo "→ Checking for scripts at repository root..."
while IFS= read -r -d '' f; do
  fail "Script at repo root (must live under scripts/): $f"
done < <(find "$ROOT" -maxdepth 1 -type f \( -name "*.sh" -o -name "*.py" \) -print0 2>/dev/null || true)

# ── 2. Shell scripts must use kebab-case.sh ──────────────────────────────────
echo "→ Checking shell script naming (kebab-case.sh)..."
while IFS= read -r -d '' f; do
  rel="${f#"$ROOT/"}"
  base="$(basename "$f")"
  # Allow underscores only in the grandfathered list; otherwise require kebab-case
  if [[ "$base" =~ [_] ]]; then
    is_legacy "$rel" || fail "Shell script uses snake_case, expected kebab-case: $rel"
  fi
done < <(find "$ROOT/scripts" -type f -name "*.sh" -print0 2>/dev/null || true)

# ── 3. Python tools must use kebab-case.py (grandfathered names excepted) ───
echo "→ Checking Python tool naming (kebab-case.py)..."
while IFS= read -r -d '' f; do
  rel="${f#"$ROOT/"}"
  base="$(basename "$f")"
  if [[ "$base" =~ [_] ]]; then
    is_legacy "$rel" || fail "Python tool uses snake_case, expected kebab-case: $rel"
  fi
done < <(find "$ROOT/scripts" -type f -name "*.py" -print0 2>/dev/null || true)

# ── 4. Every shell script must have a Usage: comment ─────────────────────────
echo "→ Checking shell scripts have a Usage: header..."
while IFS= read -r -d '' f; do
  rel="${f#"$ROOT/"}"
  # CI-only scripts and lib/ scripts are exempt (they are sourced or machine-invoked)
  [[ "$rel" == scripts/ci/* ]] && continue
  [[ "$rel" == scripts/lib/* ]] && continue
  if ! grep -q -i 'usage:' "$f" 2>/dev/null; then
    fail "Shell script missing 'Usage:' comment: $rel"
  fi
done < <(find "$ROOT/scripts" -maxdepth 1 -type f -name "*.sh" -print0 2>/dev/null || true)

# ── 5. lib/ files must not be executable entry points ────────────────────────
echo "→ Checking scripts/lib/ files are library-only (no shebang-as-main)..."
while IFS= read -r -d '' f; do
  rel="${f#"$ROOT/"}"
  # lib/ files may have shebangs for shellcheck compatibility, but must not
  # declare a main() function or run top-level code outside of functions.
  if grep -q -E '^[[:space:]]*(main|run)[[:space:]]*\(\)' "$f" 2>/dev/null; then
    fail "Library file declares main()/run() — may be misplaced: $rel"
  fi
done < <(find "$ROOT/scripts/lib" -type f -name "*.sh" -print0 2>/dev/null || true)

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
if [[ "$REPORT_ONLY" == "true" ]]; then
  echo "Scripts convention report:"
  for line in "${REPORT[@]}"; do echo "$line"; done
  echo ""
  echo "Total violations: $ERRORS"
  exit 0
fi

if [[ "$ERRORS" -eq 0 ]]; then
  echo "✓ All scripts follow the naming and placement conventions"
  exit 0
else
  echo ""
  echo "✗ $ERRORS convention violation(s) found." >&2
  echo "  See CONVENTIONS.md §Shell scripts for the rules." >&2
  exit 1
fi
