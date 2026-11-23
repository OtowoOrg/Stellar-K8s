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
# scripts/ci/lint.sh — Unified lint runner for the Stellar-K8s repository.
#
# Runs every lint gate in the correct order so developers and CI share one
# entry point: shellcheck, shell-safety, scripts-convention, python-lint
# (py_compile + tomllib), yamllint, schema-compat, then cargo fmt + clippy.
# Each step echoes a short header so failures are easy to locate
# in long log streams.
#
# The schema-compat gate diffs the PR's schemas against every registered
# consumer in the schema registry and blocks the merge on breaking changes
# unless an explicit override is supplied (see scripts/ci/schema-compat.sh).
#
# Usage:
#   bash scripts/ci/lint.sh              # full suite (default)
#   bash scripts/ci/lint.sh --fast       # skip Rust lints (shell + YAML only)
#   bash scripts/ci/lint.sh --rust-only  # skip shell / YAML lints
#   bash scripts/ci/lint.sh --schema-only # run only the schema-compat gate
#
# Environment:
#   SCHEMA_COMPAT_OVERRIDE  Set to a non-empty justification string to allow
#                           a breaking schema change (recorded in the report).
=======
#
# Exit codes:
#   0  All lint checks passed
#   1  One or more checks failed
#
# Make target: make lint-all
# CI job:      .github/workflows/ci.yml  →  scripts-convention

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

# ── argument parsing ──────────────────────────────────────────────────────────
FAST=false
RUST_ONLY=false
SCHEMA_ONLY=false

for arg in "$@"; do
  case "$arg" in
    --fast)        FAST=true ;;
    --rust-only)   RUST_ONLY=true ;;
    --schema-only) SCHEMA_ONLY=true ;;
    *) echo "Unknown flag: $arg" >&2; exit 1 ;;
  esac
done

ERRORS=0

run_step() {
  local label="$1"; shift
  echo ""
  echo "=== $label ==="
  if "$@"; then
    echo "✓ $label passed"
  else
    echo "✗ $label FAILED" >&2
    ERRORS=$((ERRORS + 1))
  fi
}

# ── Shell lints ───────────────────────────────────────────────────────────────
if [[ "$RUST_ONLY" == "false" && "$SCHEMA_ONLY" == "false" ]]; then
  run_step "shellcheck" bash -c '
    mapfile -t shell_files < <(find scripts -type f -name "*.sh" 2>/dev/null || true)
    if [[ "${#shell_files[@]}" -eq 0 ]]; then
      echo "No shell scripts found — skipping"
      exit 0
    fi
    shellcheck -S error "${shell_files[@]}"
  '

  run_step "shell-safety" python3 scripts/check-shell-safety.py

  run_step "scripts-convention" bash scripts/ci/check-scripts-convention.sh

  run_step "python-lint" bash scripts/ci/check-python-toml-lint.sh

  run_step "yamllint" bash -c '
    command -v yamllint >/dev/null 2>&1 || { echo "yamllint not installed — skipping"; exit 0; }
    yamllint -c .yamllint.yml .
  '
fi

# ── Schema compatibility gate ─────────────────────────────────────────────────
# Diffs changed schemas against every registered consumer and fails on
# breaking changes unless SCHEMA_COMPAT_OVERRIDE is set. Runs in the default
# suite, in --fast mode, and standalone via --schema-only. Skipped only for
# --rust-only runs.
if [[ "$RUST_ONLY" == "false" ]]; then
  run_step "schema-compat" bash scripts/ci/schema-compat.sh
fi

# ── Rust lints ────────────────────────────────────────────────────────────────
if [[ "$FAST" == "false" && "$SCHEMA_ONLY" == "false" ]]; then
  run_step "cargo fmt --check" bash -c '
    K8S_OPENAPI_ENABLED_VERSION=1.30 cargo fmt --all --check
  '

  run_step "clippy (strict)" bash -c '
    K8S_OPENAPI_ENABLED_VERSION=1.30 cargo clippy \
      --workspace --all-targets --all-features -- \
      -D clippy::correctness -D clippy::suspicious \
      -D clippy::perf -D clippy::style
  '
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
if [[ "$ERRORS" -eq 0 ]]; then
  echo "✓ All lint checks passed"
  exit 0
else
  echo "✗ $ERRORS lint check(s) failed" >&2
  exit 1
fi
