#!/usr/bin/env bash
# scripts/repo-health.sh — Single entry point for common repository health checks.
#
# Usage:
#   bash scripts/repo-health.sh              # full gate (default)
#   bash scripts/repo-health.sh --fast       # fmt + clippy + compile (no tests)
#   bash scripts/repo-health.sh --with-audit # include cargo audit
#   bash scripts/repo-health.sh --with-links # include markdown link check
#   bash scripts/repo-health.sh --with-helm  # include helm lint
#   make health
#   make validate                            # alias for --fast
#
# Stops at the first failing step and prints a clear summary.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# shellcheck source=scripts/lib/errors.sh
source "${SCRIPT_DIR}/lib/errors.sh"
# shellcheck source=scripts/lib/health-steps.sh
source "${SCRIPT_DIR}/lib/health-steps.sh"

MODE="full"
WITH_AUDIT=0
WITH_LINKS=0
WITH_HELM=0

usage() {
  cat <<'EOF'
Usage: repo-health.sh [OPTIONS]

Options:
  --fast         Format, lint, and compile check only (no tests)
  --with-audit   Also run cargo audit
  --with-links   Also run markdown link check
  --with-helm    Also run helm lint
  -h, --help     Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --fast) MODE="fast"; shift ;;
    --with-audit) WITH_AUDIT=1; shift ;;
    --with-links) WITH_LINKS=1; shift ;;
    --with-helm) WITH_HELM=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

declare -a STEPS=()
declare -a STEP_NAMES=()

add_step() {
  STEPS+=("$1")
  STEP_NAMES+=("$2")
}

add_step sk8s_health_fmt_check "Format check (cargo fmt --all --check)"
add_step sk8s_health_clippy "Lint (cargo clippy)"

if [[ "${MODE}" == "fast" ]]; then
  add_step sk8s_health_compile_check "Compile check (cargo test --no-run)"
else
  add_step sk8s_health_test "Tests (cargo test)"
  add_step sk8s_health_api_docs "API docs drift check"
  add_step sk8s_health_shellcheck "Shell script lint (shellcheck)"
fi

if [[ "${WITH_AUDIT}" -eq 1 ]]; then
  add_step sk8s_health_cargo_audit "Security audit (cargo audit)"
fi
if [[ "${WITH_LINKS}" -eq 1 ]]; then
  add_step sk8s_health_link_check "Markdown link check"
fi
if [[ "${WITH_HELM}" -eq 1 ]]; then
  add_step sk8s_health_helm_lint "Helm chart lint"
fi

TOTAL_STEPS="${#STEPS[@]}"

print_header() {
  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "  Stellar-K8s repository health check (${MODE})"
  echo "  repo: ${REPO_ROOT}"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
}

print_header

for i in "${!STEPS[@]}"; do
  step_num=$((i + 1))
  step_fn="${STEPS[$i]}"
  step_title="${STEP_NAMES[$i]}"

  sk8s_step "${step_title}" "[${step_num}/${TOTAL_STEPS}]"

  case "${step_fn}" in
    sk8s_health_api_docs)
      if ! command -v python3 >/dev/null 2>&1; then
        sk8s_warn "python3 not found — skipping API docs check"
        continue
      fi
      if ! sk8s_health_api_docs; then
        sk8s_fail "API docs drift detected" "Run 'make generate-api-docs' after CRD changes."
      fi
      ;;
    sk8s_health_shellcheck)
      if ! command -v shellcheck >/dev/null 2>&1; then
        sk8s_warn "shellcheck not installed — skipping (optional locally; CI runs this)"
        continue
      fi
      if ! sk8s_health_shellcheck; then
        sk8s_fail "Shellcheck reported errors" "Fix shellcheck findings in scripts/*.sh."
      fi
      ;;
    sk8s_health_fmt_check)
      if ! sk8s_health_fmt_check; then
        sk8s_fail "Code is not formatted" "Run 'make fmt' to auto-format Rust sources."
      fi
      ;;
    sk8s_health_clippy)
      if ! sk8s_health_clippy; then
        sk8s_fail "Clippy reported errors" "Run 'make lint' for details."
      fi
      ;;
    sk8s_health_test)
      if ! sk8s_health_test; then
        sk8s_fail "Tests failed" "Run 'make test' to reproduce locally."
      fi
      ;;
    sk8s_health_compile_check)
      if ! sk8s_health_compile_check; then
        sk8s_fail "Compilation failed" "Fix compiler errors and re-run 'make validate'."
      fi
      ;;
    sk8s_health_cargo_audit)
      if ! sk8s_health_cargo_audit; then
        sk8s_fail "Cargo audit reported vulnerabilities" "Review 'cargo audit' output before merging."
      fi
      ;;
    sk8s_health_link_check)
      if ! command -v python3 >/dev/null 2>&1; then
        sk8s_warn "python3 not found — skipping link check"
        continue
      fi
      if ! sk8s_health_link_check; then
        sk8s_fail "Broken markdown links found" "Run 'make link-check' for details."
      fi
      ;;
    sk8s_health_helm_lint)
      if ! command -v helm >/dev/null 2>&1; then
        sk8s_warn "helm not installed — skipping helm lint"
        continue
      fi
      if ! sk8s_health_helm_lint; then
        sk8s_fail "Helm lint failed" "Run 'make helm-lint' for details."
      fi
      ;;
    *)
      sk8s_fail "Unknown health step: ${step_fn}" "Report this as a bug in scripts/repo-health.sh."
      ;;
  esac

  echo "    ✓ ${step_title} passed"
done

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  ✓ All repository health checks passed (${TOTAL_STEPS}/${TOTAL_STEPS})"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
