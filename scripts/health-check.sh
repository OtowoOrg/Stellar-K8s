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
# scripts/health-check.sh — Comprehensive environment health check.
#
# Reports the status of all installed tools, identifies outdated components,
# and provides troubleshooting guidance.
#
# Usage:
#   ./scripts/health-check.sh                    # Full health check
#   ./scripts/health-check.sh --json             # Machine-readable output
#   ./scripts/health-check.sh --fix              # Attempt auto-fix where possible
#   ./scripts/health-check.sh --outdated-only    # Show only outdated components
#
# Exit codes:
#   0 = all components healthy
#   1 = one or more components outdated or missing
#   2 = critical component missing

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/versions.sh"

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Output format
OUTPUT_FORMAT="text"
FIX_MODE=false
OUTDATED_ONLY=false

# Component status tracking
declare -A COMPONENT_STATUS  # component -> status (healthy|outdated|missing)
declare -A COMPONENT_VERSION # component -> version
declare -A COMPONENT_HINT    # component -> install hint
TOTAL_ISSUES=0
CRITICAL_MISSING=0

# ─────────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────────

msg_pass() { echo -e "${GREEN}[✓]${NC} $*"; }
msg_warn() { echo -e "${YELLOW}[!]${NC} $*"; }
msg_fail() { echo -e "${RED}[✗]${NC} $*"; }
msg_info() { echo -e "${BLUE}[ℹ]${NC} $*"; }

_extract_semver() {
  grep -oE 'v?[0-9]+\.[0-9]+(\.[0-9]+)?' | head -1 | sed 's/^v//'
}

_tool_version() {
  local binary="$1"
  case "${binary}" in
    kubectl) kubectl version --client 2>&1 | _extract_semver ;;
    helm)    helm version --short 2>&1 | _extract_semver ;;
    kind)    kind version 2>&1 | _extract_semver ;;
    *)       ${binary} --version 2>&1 | _extract_semver ;;
  esac 2>/dev/null || echo ""
}

_check_component() {
  local name="$1" binary="$2" min_version="$3" hint="$4"
  
  if ! command -v "${binary}" &>/dev/null; then
    COMPONENT_STATUS["${name}"]="missing"
    COMPONENT_VERSION["${name}"]="not installed"
    COMPONENT_HINT["${name}"]="${hint}"
    (( CRITICAL_MISSING++ )) || true
    return 1
  fi
  
  local got=$(_tool_version "${binary}")
  COMPONENT_VERSION["${name}"]="${got:-unknown}"
  
  if [[ -z "${got}" ]]; then
    COMPONENT_STATUS["${name}"]="unknown"
    return 1
  fi
  
  if version_ge "${got}" "${min_version}"; then
    COMPONENT_STATUS["${name}"]="healthy"
    return 0
  else
    COMPONENT_STATUS["${name}"]="outdated"
    COMPONENT_HINT["${name}"]="${hint}"
    (( TOTAL_ISSUES++ )) || true
    return 1
  fi
}

# ─────────────────────────────────────────────────────────────────────────
# Component checks
# ─────────────────────────────────────────────────────────────────────────

check_rust_toolchain() {
  _check_component "Rust" "cargo" "${RUST_TOOLCHAIN}" \
    "Install Rust: https://rustup.rs/ or run: rustup update stable"
  
  # Also check Rust components
  if command -v cargo &>/dev/null; then
    if ! rustup component list | grep -q "^rustfmt.*installed"; then
      COMPONENT_STATUS["rustfmt"]="missing"
      (( TOTAL_ISSUES++ )) || true
    fi
    if ! rustup component list | grep -q "^clippy.*installed"; then
      COMPONENT_STATUS["clippy"]="missing"
      (( TOTAL_ISSUES++ )) || true
    fi
  fi
}

check_kubernetes_tools() {
  _check_component "kubectl" "kubectl" "${KUBECTL_VERSION}" \
    "Install kubectl: https://kubernetes.io/docs/tasks/tools/"
  _check_component "kind" "kind" "${KIND_VERSION}" \
    "Install kind: https://kind.sigs.k8s.io/docs/user/quick-start/#installation"
  _check_component "Helm" "helm" "${HELM_VERSION}" \
    "Install Helm: https://helm.sh/docs/intro/install/"
}

check_container_tools() {
  _check_component "Docker" "docker" "20.0.0" \
    "Install Docker: https://docs.docker.com/engine/install/"
}

check_optional_tools() {
  # Optional but recommended tools
  if command -v pre-commit &>/dev/null; then
    local version=$(_tool_version "pre-commit")
    COMPONENT_VERSION["pre-commit"]="${version:-unknown}"
    COMPONENT_STATUS["pre-commit"]="healthy"
  else
    COMPONENT_STATUS["pre-commit"]="missing"
    COMPONENT_HINT["pre-commit"]="Install pre-commit: https://pre-commit.com/#install"
  fi
  
  if command -v shellcheck &>/dev/null; then
    local version=$(_tool_version "shellcheck")
    COMPONENT_VERSION["shellcheck"]="${version:-unknown}"
    COMPONENT_STATUS["shellcheck"]="healthy"
  else
    COMPONENT_STATUS["shellcheck"]="missing"
    COMPONENT_HINT["shellcheck"]="Install shellcheck for shell script linting"
  fi
  
  if command -v k6 &>/dev/null; then
    local version=$(_tool_version "k6")
    COMPONENT_VERSION["k6"]="${version:-unknown}"
    COMPONENT_STATUS["k6"]="healthy"
  else
    COMPONENT_STATUS["k6"]="missing"
    COMPONENT_HINT["k6"]="Install k6: https://k6.io/docs/get-started/installation/"
  fi
}

check_git_config() {
  # Check for basic git configuration
  if git config user.name &>/dev/null && git config user.email &>/dev/null; then
    COMPONENT_STATUS["git-config"]="healthy"
    COMPONENT_VERSION["git-config"]="configured"
  else
    COMPONENT_STATUS["git-config"]="missing"
    COMPONENT_HINT["git-config"]="Configure git: git config --global user.name 'Your Name' && git config --global user.email 'email@example.com'"
    (( TOTAL_ISSUES++ )) || true
  fi
}

check_nodejs_optional() {
  if command -v node &>/dev/null; then
    local version=$(_tool_version "node")
    COMPONENT_VERSION["Node.js"]="${version:-unknown}"
    COMPONENT_STATUS["Node.js"]="healthy"
  else
    COMPONENT_STATUS["Node.js"]="missing"
    COMPONENT_HINT["Node.js"]="Optional: Install Node.js for documentation tools"
  fi
}

# ─────────────────────────────────────────────────────────────────────────
# Disk space and system checks
# ─────────────────────────────────────────────────────────────────────────

check_system_resources() {
  local required_disk_gb=20
  local available_gb=$(($(df . 2>/dev/null | tail -1 | awk '{print $4}') / 1024 / 1024))
  
  if [[ ${available_gb} -ge ${required_disk_gb} ]]; then
    COMPONENT_STATUS["disk-space"]="healthy"
    COMPONENT_VERSION["disk-space"]="${available_gb}GB available"
  else
    COMPONENT_STATUS["disk-space"]="warning"
    COMPONENT_VERSION["disk-space"]="${available_gb}GB available (${required_disk_gb}GB recommended)"
    (( TOTAL_ISSUES++ )) || true
  fi
}

# ─────────────────────────────────────────────────────────────────────────
# Auto-fix functionality
# ─────────────────────────────────────────────────────────────────────────

autofix_components() {
  msg_info "Attempting to auto-fix missing components..."
  
  # Rust components
  if [[ "${COMPONENT_STATUS["rustfmt"]:-}" == "missing" ]]; then
    msg_info "Installing rustfmt..."
    rustup component add rustfmt || msg_fail "Failed to install rustfmt"
  fi
  
  if [[ "${COMPONENT_STATUS["clippy"]:-}" == "missing" ]]; then
    msg_info "Installing clippy..."
    rustup component add clippy || msg_fail "Failed to install clippy"
  fi
  
  # Pre-commit
  if [[ "${COMPONENT_STATUS["pre-commit"]:-}" == "missing" ]]; then
    msg_info "Installing pre-commit..."
    pip install pre-commit || msg_fail "Failed to install pre-commit"
  fi
}

# ─────────────────────────────────────────────────────────────────────────
# Output formatting
# ─────────────────────────────────────────────────────────────────────────

print_text_report() {
  echo ""
  echo "╔════════════════════════════════════════════════════════════════╗"
  echo "║           ENVIRONMENT HEALTH CHECK REPORT                      ║"
  echo "╚════════════════════════════════════════════════════════════════╝"
  echo ""
  
  echo "Required Tools (Strict):"
  for component in "Rust" "kubectl" "kind" "Helm" "Docker"; do
    local status="${COMPONENT_STATUS[$component]:-unknown}"
    local version="${COMPONENT_VERSION[$component]:-unknown}"
    case "${status}" in
      healthy)  msg_pass "${component}: ${version}" ;;
      outdated) msg_warn "${component}: ${version} (outdated)" ;;
      missing)  msg_fail "${component}: ${version}" ;;
      *)        msg_info "${component}: ${version}" ;;
    esac
  done
  
  echo ""
  echo "Rust Components:"
  for component in "rustfmt" "clippy"; do
    local status="${COMPONENT_STATUS[$component]:-unknown}"
    case "${status}" in
      healthy)  msg_pass "${component}: installed" ;;
      missing)  msg_warn "${component}: not installed" ;;
      *)        msg_info "${component}: ${status}" ;;
    esac
  done
  
  echo ""
  echo "Optional Tools (Recommended):"
  for component in "pre-commit" "shellcheck" "k6" "Node.js"; do
    local status="${COMPONENT_STATUS[$component]:-unknown}"
    local version="${COMPONENT_VERSION[$component]:-unknown}"
    case "${status}" in
      healthy)  msg_pass "${component}: ${version}" ;;
      missing)  msg_info "${component}: not installed - ${COMPONENT_HINT[$component]:-}" ;;
      *)        msg_warn "${component}: ${version}" ;;
    esac
  done
  
  echo ""
  echo "System Resources:"
  local disk_status="${COMPONENT_STATUS[disk-space]:-unknown}"
  local disk_version="${COMPONENT_VERSION[disk-space]:-unknown}"
  case "${disk_status}" in
    healthy)  msg_pass "Disk space: ${disk_version}" ;;
    warning)  msg_warn "Disk space: ${disk_version}" ;;
    *)        msg_info "Disk space: ${disk_version}" ;;
  esac
  
  echo ""
  echo "Git Configuration:"
  local git_status="${COMPONENT_STATUS[git-config]:-unknown}"
  case "${git_status}" in
    healthy)  msg_pass "Git config: ready" ;;
    missing)  msg_warn "Git config: not configured" ;;
  esac
  
  echo ""
  echo "╔════════════════════════════════════════════════════════════════╗"
  
  if (( CRITICAL_MISSING > 0 )); then
    echo -e "${RED}Critical Issues: ${CRITICAL_MISSING} missing required component(s)${NC}"
    echo "Please install missing required tools before proceeding."
  elif (( TOTAL_ISSUES > 0 )); then
    echo -e "${YELLOW}Issues Found: ${TOTAL_ISSUES} outdated or missing optional component(s)${NC}"
    echo "Consider updating components for best experience."
  else
    echo -e "${GREEN}All checks passed! Environment is healthy. ✓${NC}"
  fi
  echo "╚════════════════════════════════════════════════════════════════╝"
  echo ""
}

print_json_report() {
  local status="healthy"
  (( CRITICAL_MISSING > 0 )) && status="critical"
  (( TOTAL_ISSUES > 0 && CRITICAL_MISSING == 0 )) && status="warning"
  
  echo "{"
  echo "  \"status\": \"${status}\","
  echo "  \"timestamp\": \"$(date -Iseconds)\","
  echo "  \"critical_missing\": ${CRITICAL_MISSING},"
  echo "  \"total_issues\": ${TOTAL_ISSUES},"
  echo "  \"components\": {"
  
  local first=true
  for component in "${!COMPONENT_STATUS[@]}"; do
    if [[ "${first}" == false ]]; then echo ","; fi
    echo -n "    \"${component}\": {"
    echo -n "\"status\": \"${COMPONENT_STATUS[$component]}\", "
    echo -n "\"version\": \"${COMPONENT_VERSION[$component]}\""
    [[ -n "${COMPONENT_HINT[$component]:-}" ]] && echo -n ", \"hint\": \"${COMPONENT_HINT[$component]}\""
    echo -n "}"
    first=false
  done
  
  echo ""
  echo "  }"
  echo "}"
}

# ─────────────────────────────────────────────────────────────────────────
# Troubleshooting guide
# ─────────────────────────────────────────────────────────────────────────

print_troubleshooting() {
  echo ""
  echo "╔════════════════════════════════════════════════════════════════╗"
  echo "║                   TROUBLESHOOTING GUIDE                        ║"
  echo "╚════════════════════════════════════════════════════════════════╝"
  echo ""
  
  if (( CRITICAL_MISSING > 0 )); then
    echo "Missing Required Components:"
    [[ "${COMPONENT_STATUS[Rust]:-}" == "missing" ]] && \
      echo "  • Rust: ${COMPONENT_HINT[Rust]}"
    [[ "${COMPONENT_STATUS[kubectl]:-}" == "missing" ]] && \
      echo "  • kubectl: ${COMPONENT_HINT[kubectl]}"
    [[ "${COMPONENT_STATUS[kind]:-}" == "missing" ]] && \
      echo "  • kind: ${COMPONENT_HINT[kind]}"
    [[ "${COMPONENT_STATUS[Helm]:-}" == "missing" ]] && \
      echo "  • Helm: ${COMPONENT_HINT[Helm]}"
    [[ "${COMPONENT_STATUS[Docker]:-}" == "missing" ]] && \
      echo "  • Docker: ${COMPONENT_HINT[Docker]}"
  fi
  
  if [[ "${COMPONENT_STATUS[git-config]:-}" == "missing" ]]; then
    echo ""
    echo "Configure Git:"
    echo "  git config --global user.name 'Your Name'"
    echo "  git config --global user.email 'your.email@example.com'"
  fi
  
  echo ""
  echo "For a fresh clone, run:"
  echo "  1. make preflight             # Verify required tools"
  echo "  2. make dev-setup             # Install Rust components & git hooks"
  echo "  3. make health-check          # Full environment check"
  echo "  4. make quick                 # Fast compile check"
  echo ""
}

# ─────────────────────────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────────────────────────

main() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --json)           OUTPUT_FORMAT="json" ;;
      --fix)            FIX_MODE=true ;;
      --outdated-only)  OUTDATED_ONLY=true ;;
      *) echo "Unknown option: $1"; exit 1 ;;
    esac
    shift
  done
  
  # Run all checks
  check_rust_toolchain
  check_kubernetes_tools
  check_container_tools
  check_optional_tools
  check_git_config
  check_nodejs_optional
  check_system_resources
  
  # Auto-fix if requested
  if [[ "${FIX_MODE}" == true ]]; then
    autofix_components
  fi
  
  # Output report
  if [[ "${OUTPUT_FORMAT}" == "json" ]]; then
    print_json_report
  else
    print_text_report
    print_troubleshooting
  fi
  
  # Determine exit code
  (( CRITICAL_MISSING > 0 )) && exit 2
  (( TOTAL_ISSUES > 0 )) && exit 1
  exit 0
}

main "$@"
