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
# scripts/dep-gate.sh — Single consolidated lockfile gate for Stellar-K8s.
#
# Runs all dependency-related checks in one place, relying on
# .cargo/audit.toml and deny.toml as the canonical configuration files.
# No inline --ignore flags are used — add advisories to .cargo/audit.toml.
#
# Usage:
#   bash scripts/dep-gate.sh              # full gate (default)
#   bash scripts/dep-gate.sh --quick      # cargo audit + cargo deny only (no licenses or build)
#   bash scripts/dep-gate.sh --audit-only # cargo audit only
#
# Exit codes:
#   0  All checks passed
#   1  One or more checks failed

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# ── Colour helpers ─────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

pass() { echo -e "  ${GREEN}✓${NC} $*"; }
fail() { echo -e "  ${RED}✗${NC} $*"; local s=$?; FAILURES=$((FAILURES + 1)); return "${s:-1}"; }
warn() { echo -e "  ${YELLOW}⚠${NC}  $*"; }
section() { echo -e "\n${BOLD}── $* ──${NC}"; }

FAILURES=0
MODE="${1:-full}"

# ── Tool installation helper ───────────────────────────────────────────────────
ensure_tool() {
  local tool="$1" pkg="${2:-$1}"
  if command -v "${tool}" >/dev/null 2>&1; then
    return 0
  fi
  echo "  Installing ${pkg}..."
  cargo install --locked "${pkg}"
}

# ── Check 1: cargo audit (vulnerability scan) ──────────────────────────────────
check_audit() {
  section "Check 1: cargo audit (vulnerability scan)"
  ensure_tool "cargo-audit"
  if cargo audit --quiet; then
    pass "No known vulnerabilities"
  else
    fail "cargo audit found unignored vulnerabilities — add to .cargo/audit.toml if justified"
    return 1
  fi
}

# ── Check 2: cargo deny (license + bans + advisories + sources) ────────────────
check_deny() {
  section "Check 2: cargo deny (license + bans + advisories + sources)"
  ensure_tool "cargo-deny"
  if cargo deny check; then
    pass "All dependency policies satisfied"
  else
    fail "cargo deny check found violations — review deny.toml"
    return 1
  fi
}

# ── Check 3: Third-party license drift ─────────────────────────────────────────
check_licenses() {
  section "Check 3: Third-party license drift"
  if bash "${SCRIPT_DIR}/generate-third-party-licenses.sh" --check; then
    pass "THIRD_PARTY_LICENSES.md is up to date"
  else
    fail "THIRD_PARTY_LICENSES.md is stale — run: make third-party-licenses"
    return 1
  fi
}

# ── Check 4: Lockfile integrity (cargo build --locked) ─────────────────────────
check_locked() {
  section "Check 4: Lockfile integrity (cargo build --locked)"
  if cargo build --locked --release 2>&1 | tail -5; then
    pass "Cargo.lock is consistent with Cargo.toml"
  else
    fail "Cargo.lock is inconsistent — run: cargo update"
    return 1
  fi
}

ECHO_DONE=0
run_check() {
  local name="$1" fn="$2"
  echo ""
  echo "  • ${name}..."
  if "${fn}"; then
    pass "${name}"
  else
    ECHO_DONE=1
  fi
}

echo ""
echo "${BOLD}Stellar-K8s Lockfile Gate${NC}"
echo "═══════════════════════════"

check_audit

if [[ "${MODE}" != "--audit-only" ]]; then
  check_deny
  check_licenses
fi

if [[ "${MODE}" == "full" ]]; then
  check_locked
fi

# ── Summary ────────────────────────────────────────────────────────────────────
echo ""
echo "═══════════════════════════"
if [[ "${FAILURES}" -eq 0 ]]; then
  echo -e "${GREEN}${BOLD}✓ All lockfile gate checks passed${NC}"
  exit 0
else
  echo -e "${RED}${BOLD}✗ ${FAILURES} check(s) failed${NC}"
  exit 1
fi
