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

# Setup color variables based on TTY check
if [[ -t 1 && -t 2 ]]; then
  SK8S_RED='\033[0;31m'
  SK8S_GREEN='\033[0;32m'
  SK8S_YELLOW='\033[1;33m'
  SK8S_CYAN='\033[0;36m'
  SK8S_BOLD='\033[1m'
  SK8S_RESET='\033[0m'
else
  SK8S_RED=''
  SK8S_GREEN=''
  SK8S_YELLOW=''
  SK8S_CYAN=''
  SK8S_BOLD=''
  SK8S_RESET=''
fi

sk8s_step() {
  SK8S_STEP="$1"
  echo -e "\n${SK8S_BOLD}${SK8S_CYAN}--> [${SK8S_STEP}] $2${SK8S_RESET}"
}

sk8s_fail() {
  local detail="$1"
  local hint="${2:-}"
  echo -e "${SK8S_RED}ERROR [${SK8S_STEP}]: ${detail}${SK8S_RESET}" >&2
  if [[ -n "${hint}" ]]; then
    echo -e "  ${SK8S_YELLOW}Hint: ${hint}${SK8S_RESET}" >&2
  fi
  exit 1
}

sk8s_warn() {
  local detail="$1"
  echo -e "${SK8S_YELLOW}WARN [${SK8S_STEP}]: ${detail}${SK8S_RESET}" >&2
}

sk8s_error() {
  local detail="$1"
  echo -e "${SK8S_RED}ERROR [${SK8S_STEP}]: ${detail}${SK8S_RESET}" >&2
}

sk8s_pass() {
  local detail="$1"
  echo -e "  ${SK8S_GREEN}✓ ${detail}${SK8S_RESET}"
}

sk8s_info() {
  local detail="$1"
  echo -e "  ${SK8S_CYAN}ℹ ${detail}${SK8S_RESET}"
}
