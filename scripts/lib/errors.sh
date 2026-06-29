#!/usr/bin/env bash
# scripts/lib/errors.sh
# Shared step-aware diagnostics for shell scripts.
#
# Usage:
#   source "${SCRIPT_DIR}/lib/errors.sh"
#   sk8s_step "format check" "Running cargo fmt --all --check"
#   sk8s_fail "Code is not formatted" "Run 'make fmt' and retry"
#
# Messages follow the same `[step] detail` style as Rust helpers in src/error.rs.

: "${SK8S_STEP:=unknown step}"

sk8s_step() {
  SK8S_STEP="$1"
  echo ""
  echo "--> [${SK8S_STEP}] $2"
}

sk8s_fail() {
  local detail="$1"
  local hint="${2:-}"
  echo "ERROR [${SK8S_STEP}]: ${detail}" >&2
  if [[ -n "${hint}" ]]; then
    echo "  Hint: ${hint}" >&2
  fi
  exit 1
}

sk8s_warn() {
  local detail="$1"
  echo "WARN [${SK8S_STEP}]: ${detail}" >&2
}
