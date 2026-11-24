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
# audit-features.sh
#
# Static audit for unused crate features and dead imports (Issue #1114).
#
# What this script checks:
#   1. Unused / redundant crate features — features declared in Cargo.toml whose
#      activation guards (`#[cfg(feature = "...")]`) never appear in any .rs file.
#   2. Implicit feature propagation — optional deps activated transitively without
#      an explicit feature flag, which can silently pull in unwanted code.
#   3. Dead `use` imports flagged by `cargo check` with `-D unused_imports`.
#   4. Dead code items (`-D dead_code`) in library and binary targets.
#
# Usage:
#   ./scripts/audit-features.sh          # full audit (exits non-zero on findings)
#   ./scripts/audit-features.sh --report # print report only, always exit 0

set -euo pipefail

REPORT_ONLY=false
for arg in "$@"; do
    [[ "$arg" == "--report" ]] && REPORT_ONLY=true
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

BOLD='\033[1m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
GREEN='\033[0;32m'
RESET='\033[0m'

separator() { echo -e "${BOLD}────────────────────────────────────────────────────────────────${RESET}"; }

findings=0

separator
echo -e "${BOLD}Step 1 — Declare unused crate features${RESET}"
separator

# Extract all feature names from Cargo.toml (skip built-in 'default').
declared_features=$(grep -E '^[a-zA-Z0-9_-]+ =' Cargo.toml \
    | grep -v '^default ' \
    | sed 's/ =.*//' \
    | grep -v '^\[' \
    | sort -u || true)

unused_features=()
while IFS= read -r feature; do
    [[ -z "$feature" ]] && continue
    # Search for any cfg(feature = "...") reference in Rust source.
    if ! grep -rq "feature = \"${feature}\"" src/ 2>/dev/null; then
        unused_features+=("$feature")
    fi
done <<< "$declared_features"

if [[ "${#unused_features[@]}" -gt 0 ]]; then
    echo -e "${YELLOW}⚠  Features declared in Cargo.toml with no cfg-guard in src/:${RESET}"
    for f in "${unused_features[@]}"; do
        echo "   - $f"
    done
    findings=$((findings + ${#unused_features[@]}))
else
    echo -e "${GREEN}✓  All declared features have at least one cfg-guard in src/${RESET}"
fi

separator
echo -e "${BOLD}Step 2 — Unused imports (cargo check)${RESET}"
separator

# Run cargo check with unused_imports denied.
# Capture stderr (where rustc emits warnings/errors).
set +e
check_output=$(RUSTFLAGS="-D unused_imports" cargo check --message-format=short 2>&1)
check_exit=$?
set -e

if [[ $check_exit -ne 0 ]]; then
    # Filter only unused-import lines to keep output readable.
    import_errors=$(echo "$check_output" | grep "unused import" || true)
    if [[ -n "$import_errors" ]]; then
        echo -e "${RED}✗  Unused imports detected:${RESET}"
        echo "$import_errors"
        findings=$((findings + $(echo "$import_errors" | wc -l)))
    else
        echo -e "${YELLOW}⚠  cargo check failed for a reason other than unused imports${RESET}"
        echo "$check_output" | tail -20
    fi
else
    echo -e "${GREEN}✓  No unused imports found${RESET}"
fi

separator
echo -e "${BOLD}Step 3 — Dead code (cargo check)${RESET}"
separator

set +e
dead_output=$(RUSTFLAGS="-D dead_code" cargo check --message-format=short 2>&1)
dead_exit=$?
set -e

if [[ $dead_exit -ne 0 ]]; then
    dead_errors=$(echo "$dead_output" | grep "dead_code\|never used\|is never read" || true)
    if [[ -n "$dead_errors" ]]; then
        echo -e "${YELLOW}⚠  Dead code detected (consider removing or adding #[allow(dead_code)]):${RESET}"
        echo "$dead_errors" | head -40
        findings=$((findings + $(echo "$dead_errors" | wc -l)))
    else
        echo -e "${YELLOW}⚠  cargo check failed for a reason other than dead_code${RESET}"
    fi
else
    echo -e "${GREEN}✓  No dead code detected${RESET}"
fi

separator
echo -e "${BOLD}Step 4 — Optional dependencies without feature guards${RESET}"
separator

# Find optional deps in Cargo.toml.
optional_deps=$(grep -E 'optional\s*=\s*true' Cargo.toml \
    | grep -Eo '^[a-zA-Z0-9_-]+' || true)

ungated_deps=()
while IFS= read -r dep; do
    [[ -z "$dep" ]] && continue
    # Check if any feature in [features] activates this dep.
    if ! grep -qE "dep:${dep}|\"${dep}\"" Cargo.toml; then
        ungated_deps+=("$dep")
    fi
done <<< "$optional_deps"

if [[ "${#ungated_deps[@]}" -gt 0 ]]; then
    echo -e "${YELLOW}⚠  Optional deps with no explicit feature activation:${RESET}"
    for d in "${ungated_deps[@]}"; do
        echo "   - $d"
    done
    findings=$((findings + ${#ungated_deps[@]}))
else
    echo -e "${GREEN}✓  All optional deps are gated by an explicit feature${RESET}"
fi

separator
echo -e "${BOLD}Audit summary${RESET}"
separator

if [[ $findings -eq 0 ]]; then
    echo -e "${GREEN}✓  No findings — codebase is clean${RESET}"
    exit 0
else
    echo -e "${RED}✗  Total findings: ${findings}${RESET}"
    if $REPORT_ONLY; then
        echo "(--report mode: exiting 0)"
        exit 0
    fi
    exit 1
fi
