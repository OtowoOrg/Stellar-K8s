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
# scripts/preflight.sh — Strict gate: required tools AND pinned minimum
# versions (and, optionally, repo labels).
#
# Usage:
#   ./scripts/preflight.sh              # check tools + versions
#   ./scripts/preflight.sh --labels     # also verify GitHub repo labels
#   REPO=OtowoOrg/Stellar-K8s ./scripts/preflight.sh --labels
#
# Exit codes: 0 = all pass, 1 = one or more checks failed.
#
# Version pins live in scripts/lib/versions.sh so there is exactly one place
# to bump them.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/versions.sh
source "${SCRIPT_DIR}/lib/versions.sh"

# --------------------------------------------------------------------------- #
# Tools checked for presence only — no pinned minimum version in this repo.
# --------------------------------------------------------------------------- #
declare -A PRESENCE_TOOLS=(
  [docker]="Install Docker Engine: https://docs.docker.com/engine/install/"
  [gh]="Install GitHub CLI: https://cli.github.com/"
)

# --------------------------------------------------------------------------- #
# Tools with a pinned minimum version — preflight fails strictly if the
# installed version is below the pin, not just if the binary is missing.
# --------------------------------------------------------------------------- #
declare -A VERSIONED_TOOLS=(
  [cargo]="${RUST_TOOLCHAIN}|Install Rust via rustup: https://rustup.rs/"
  [kind]="${KIND_VERSION}|Install kind: https://kind.sigs.k8s.io/docs/user/quick-start/#installation"
  [kubectl]="${KUBECTL_VERSION}|Install kubectl: https://kubernetes.io/docs/tasks/tools/"
  [helm]="${HELM_VERSION}|Install Helm 3: https://helm.sh/docs/intro/install/"
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

# Pull the first x.y.z (optionally v-prefixed) version token out of arbitrary
# version output, regardless of exact tool-specific formatting.
_extract_semver() {
  grep -oE 'v?[0-9]+\.[0-9]+(\.[0-9]+)?' | head -1 | sed 's/^v//'
}

# Tool-specific version probes — kubectl/helm do not accept a global --version.
_tool_version_output() {
  local binary="$1"
  case "${binary}" in
    kubectl) "${binary}" version --client 2>&1 ;;
    helm)    "${binary}" version --short 2>&1 || "${binary}" version 2>&1 ;;
    kind)    "${binary}" version 2>&1 ;;
    *)       "${binary}" --version 2>&1 ;;
  esac
}

# --------------------------------------------------------------------------- #
# Tool checks
# --------------------------------------------------------------------------- #
check_tools() {
  echo "=== Required Tools ==="
  local errors=0

  for binary in "${!PRESENCE_TOOLS[@]}"; do
    if version=$(${binary} --version 2>&1 | head -1); then
      pass "${binary} — ${version}"
    else
      fail "${binary} not found in PATH"
      echo "         → ${PRESENCE_TOOLS[$binary]}"
      (( errors++ )) || true
    fi
  done

  echo ""
  echo "=== Required Tool Versions (strict — must be >= pinned) ==="

  for binary in "${!VERSIONED_TOOLS[@]}"; do
    local pinned="${VERSIONED_TOOLS[$binary]%%|*}"
    local hint="${VERSIONED_TOOLS[$binary]#*|}"

    if ! command -v "${binary}" >/dev/null 2>&1; then
      fail "${binary} not found in PATH (requires >= ${pinned})"
      echo "         → ${hint}"
      (( errors++ )) || true
      continue
    fi

    local got=""
    if ! command -v "${binary}" >/dev/null 2>&1; then
      got="missing"
    else
      got=$(_tool_version_output "${binary}" | _extract_semver) || got=""
      [[ -z "${got}" ]] && got="missing"
    fi
    # Prefer tool-native version commands: kubectl rejects `--version`.
    case "${binary}" in
      kubectl)
        # `kubectl --version` is not a valid client flag; use --client.
        got=$(kubectl version --client 2>&1 | _extract_semver) || got=""
        ;;
      helm)
        got=$(helm version --short 2>&1 | _extract_semver) || got=""
        ;;
      kind)
        got=$(kind version 2>&1 | _extract_semver) || got=""
        ;;
      *)
        got=$(${binary} --version 2>&1 | _extract_semver) || got=""
        ;;
    esac
    [[ -z "${got}" ]] && got="missing"

    if [[ "${got}" == "missing" ]]; then
      if ! command -v "${binary}" &>/dev/null; then
        fail "${binary} not found in PATH (requires >= ${pinned})"
      else
        fail "${binary} version could not be parsed (requires >= ${pinned})"
      fi
      echo "         → ${hint}"
      (( errors++ )) || true
    elif version_ge "${got}" "${pinned}"; then
      pass "${binary} ${got} (>= ${pinned})"
    else
      fail "${binary} ${got} is below the required minimum ${pinned}"
      echo "         → ${hint}"
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
