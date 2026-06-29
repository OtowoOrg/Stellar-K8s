#!/usr/bin/env bash
# scripts/preflight.sh — Check required tools and (optionally) repo labels.
#
# Usage:
#   ./scripts/preflight.sh              # check tools only
#   ./scripts/preflight.sh --labels     # check tools + GitHub repo labels
#   REPO=OtowoOrg/Stellar-K8s ./scripts/preflight.sh --labels
#
# Exit codes: 0 = all pass, 1 = one or more checks failed.

set -euo pipefail

# --------------------------------------------------------------------------- #
# Required tools: (binary hint)
# --------------------------------------------------------------------------- #
declare -A TOOLS=(
  [docker]="Install Docker Engine: https://docs.docker.com/engine/install/"
  [kind]="Install kind: https://kind.sigs.k8s.io/docs/user/quick-start/#installation"
  [kubectl]="Install kubectl: https://kubernetes.io/docs/tasks/tools/"
  [helm]="Install Helm 3: https://helm.sh/docs/intro/install/"
  [cargo]="Install Rust via rustup: https://rustup.rs/"
)

# Labels that must exist in the GitHub repo before issue automation runs.
REQUIRED_LABELS=("ci" "security" "stellar-wave" "maintenance" "hygiene")

# --------------------------------------------------------------------------- #
# Helpers
# --------------------------------------------------------------------------- #
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'

pass() { echo -e "  ${GREEN}[PASS]${NC} $*"; }
fail() { echo -e "  ${RED}[FAIL]${NC} $*"; }
warn() { echo -e "  ${YELLOW}[WARN]${NC} $*"; }

# --------------------------------------------------------------------------- #
# Tool checks
# --------------------------------------------------------------------------- #
check_tools() {
  echo "=== Required Tools ==="
  local errors=0

  for binary in "${!TOOLS[@]}"; do
    if version=$(${binary} --version 2>&1 | head -1); then
      pass "${binary} — ${version}"
    else
      fail "${binary} not found in PATH"
      echo "         → ${TOOLS[$binary]}"
      (( errors++ )) || true
    fi
  done

  return "${errors}"
}

# --------------------------------------------------------------------------- #
# GitHub label checks (requires gh CLI)
# --------------------------------------------------------------------------- #
check_labels() {
  local repo="${REPO:-}"
  if [[ -z "${repo}" ]]; then
    # Try to detect from git remote
    repo=$(git remote get-url origin 2>/dev/null \
      | sed -E 's|.*github\.com[:/]||; s|\.git$||') || true
  fi

  if [[ -z "${repo}" ]]; then
    warn "REPO not set and could not detect from git remote — skipping label check"
    return 0
  fi

  echo ""
  echo "=== GitHub Repo Labels (${repo}) ==="

  if ! command -v gh &>/dev/null; then
    warn "'gh' CLI not found — skipping label check. Install: https://cli.github.com/"
    return 0
  fi

  if ! gh auth status &>/dev/null 2>&1; then
    warn "Not authenticated with gh CLI — run 'gh auth login' to enable label checks"
    return 0
  fi

  local errors=0
  existing=$(gh label list --repo "${repo}" --json name --limit 200 \
    | python3 -c "import sys,json; print('\n'.join(l['name'] for l in json.load(sys.stdin)))" \
    2>/dev/null || true)

  for label in "${REQUIRED_LABELS[@]}"; do
    if echo "${existing}" | grep -qx "${label}"; then
      pass "label '${label}' exists"
    else
      warn "label '${label}' missing — creating..."
      if gh label create "${label}" --repo "${repo}" --color "ededed" &>/dev/null; then
        pass "label '${label}' created"
      else
        fail "could not create label '${label}'"
        (( errors++ )) || true
      fi
    fi
  done

  return "${errors}"
}

# --------------------------------------------------------------------------- #
# Main
# --------------------------------------------------------------------------- #
main() {
  local check_labels_flag=false
  for arg in "$@"; do
    [[ "${arg}" == "--labels" ]] && check_labels_flag=true
  done

  local total_errors=0

  check_tools || (( total_errors += $? )) || true

  if "${check_labels_flag}"; then
    check_labels || (( total_errors += $? )) || true
  fi

  echo ""
  if (( total_errors == 0 )); then
    echo -e "${GREEN}=== Preflight passed ✓ ===${NC}"
    exit 0
  else
    echo -e "${RED}=== Preflight failed: ${total_errors} issue(s) found ===${NC}"
    exit 1
  fi
}

main "$@"
