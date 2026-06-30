#!/usr/bin/env bash
# scripts/validate.sh — Fast local validation that mirrors the CI pipeline.
#
# Runs from the repository root. All paths are relative to the project root so
# the script is safe to invoke from any working directory via:
#   bash scripts/validate.sh
#
# Steps performed (in order):
#   1. Format check     — cargo fmt --all --check
#   2. Lint             — cargo clippy (same flags as Makefile `lint` target)
#   3. Compile check    — cargo test --no-run (catches compile errors only)
#
# For the full contributor health gate (includes tests and docs), use:
#   make health
#
# Environment variables:
#   K8S_OPENAPI_ENABLED_VERSION  Kubernetes API version for k8s-openapi codegen
#                                (default: 1.30, must match Makefile)

set -euo pipefail

# Resolve the repo root regardless of where the script is called from.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/errors.sh
source "${SCRIPT_DIR}/lib/errors.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

cd "${REPO_ROOT}"

K8S_OPENAPI_ENABLED_VERSION="${K8S_OPENAPI_ENABLED_VERSION:-1.30}"
export K8S_OPENAPI_ENABLED_VERSION

echo "==> Starting local validation (repo: ${REPO_ROOT})"

# ── Step 1: Format check ─────────────────────────────────────────────────────
sk8s_step "format check" "Running cargo fmt --all --check"
if ! cargo fmt --all --check; then
  sk8s_fail "Code is not formatted" "Run 'make fmt' or 'cargo fmt --all' and retry."
fi
echo "    Format OK"

# ── Step 2: Lint ─────────────────────────────────────────────────────────────
sk8s_step "lint" "Running cargo clippy with CI flags"
if ! cargo clippy --workspace --all-targets --all-features -- \
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
  -A clippy::should_implement_trait; then
  sk8s_fail "Clippy reported errors" "Run 'make lint' for the full output."
fi
echo "    Lint OK"

# ── Step 3: Compile check ────────────────────────────────────────────────────
sk8s_step "compile check" "Running cargo test --no-run"
if ! cargo test --workspace --no-run; then
  sk8s_fail "Compilation failed during test build" "Fix compiler errors and rerun 'make validate'."
fi
echo "    Compile check OK"

echo ""
echo "==> Validation complete."
