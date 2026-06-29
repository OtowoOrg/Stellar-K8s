#!/usr/bin/env bash
# scripts/repo-health.sh — Single entry point for common repository health checks.
#
# Runs the most important validations contributors need before opening a PR:
#   1. Format check (cargo fmt)
#   2. Lint (cargo clippy — same flags as CI)
#   3. Unit/integration tests (cargo test)
#   4. API docs drift check (docs/api-reference.md)
#   5. Shell script lint (shellcheck, when installed)
#
# Usage (from anywhere):
#   bash scripts/repo-health.sh
#   make health
#
# Stops at the first failing step and prints a clear summary.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

K8S_OPENAPI_ENABLED_VERSION="${K8S_OPENAPI_ENABLED_VERSION:-1.30}"
export K8S_OPENAPI_ENABLED_VERSION

readonly TOTAL_STEPS=5
STEP=0
FAILED_STEP=""
FAILED_HINT=""

print_header() {
  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "  Stellar-K8s repository health check"
  echo "  repo: ${REPO_ROOT}"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
}

begin_step() {
  STEP=$((STEP + 1))
  local title="$1"
  echo ""
  echo "[${STEP}/${TOTAL_STEPS}] ${title}"
  echo "────────────────────────────────────────────────────────────"
}

pass_step() {
  echo "  ✓ ${1} passed"
}

fail_step() {
  local name="$1"
  local hint="$2"
  FAILED_STEP="$name"
  FAILED_HINT="$hint"
  echo ""
  echo "  ✗ FAILED: ${name}"
  echo "    Hint: ${hint}"
  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "  Health check stopped at step ${STEP}/${TOTAL_STEPS}: ${name}"
  echo "  Fix the issue above, then re-run: make health"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  exit 1
}

run_or_fail() {
  local name="$1"
  local hint="$2"
  shift 2
  if "$@"; then
    pass_step "${name}"
  else
    fail_step "${name}" "${hint}"
  fi
}

print_header

begin_step "Format check (cargo fmt --all --check)"
run_or_fail "Format check" "Run 'make fmt' to auto-format Rust sources." \
  cargo fmt --all --check

begin_step "Lint (cargo clippy)"
run_or_fail "Lint" "Run 'make lint' for details." \
  cargo clippy --workspace --all-targets --all-features -- \
    -D clippy::correctness \
    -D clippy::suspicious \
    -D clippy::perf \
    -D clippy::style \
    -A clippy::new_without_default \
    -A clippy::match_like_matches_macro \
    -A clippy::match_result_ok \
    -A clippy::needless_borrow \
    -A clippy::get_first \
    -A clippy::format_in_format_args \
    -A clippy::single_match \
    -A clippy::redundant_closure \
    -A clippy::items_after_test_module \
    -A clippy::approx_constant \
    -A clippy::should_implement_trait

begin_step "Tests (cargo test)"
run_or_fail "Tests" "Run 'make test' to reproduce locally." \
  cargo test --workspace --features "rest-api,metrics,admission-webhook,k8s-v1-30,reconciler-fuzz" --tests --lib --bins

begin_step "API docs drift check"
if command -v python3 >/dev/null 2>&1; then
  run_or_fail "API docs" "Run 'make generate-api-docs' after CRD changes." \
    python3 scripts/generate-api-docs.py \
      --crd config/crd/stellarnode-crd.yaml \
      --output docs/api-reference.md \
      --check
else
  echo "  ⚠ Skipped — python3 not found (install Python 3 to enable docs check)"
fi

begin_step "Shell script lint (shellcheck)"
if command -v shellcheck >/dev/null 2>&1; then
  mapfile -t shell_files < <(find scripts -name '*.sh' -type f | sort)
  if ((${#shell_files[@]} == 0)); then
    echo "  ⚠ No shell scripts found under scripts/ — skipped"
  else
    run_or_fail "Shellcheck" "Fix shellcheck findings in scripts/*.sh." \
      shellcheck -S error "${shell_files[@]}"
  fi
else
  echo "  ⚠ Skipped — shellcheck not installed (optional; CI runs this check)"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  ✓ All repository health checks passed (${TOTAL_STEPS}/${TOTAL_STEPS})"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
