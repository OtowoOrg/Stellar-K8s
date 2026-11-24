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
# check-unreachable-modules.sh
#
# Static reachability audit for Rust modules and dead code-path markers.
# Closes Issue #1150.
#
# Wraps the `check-unreachable-modules` Cargo binary so Makefile and CI share
# a single entrypoint. See docs/unreachable-modules-check.md.
#
# Exit codes
# ----------
#   0  — No hard findings (or --report / --warn-only)
#   1  — Hard findings (new orphans / ambiguous paths)
#   2  — Tooling / usage error
#
# Usage:
#   ./scripts/check-unreachable-modules.sh
#   ./scripts/check-unreachable-modules.sh --report
#   ./scripts/check-unreachable-modules.sh --warn-only
#   ./scripts/check-unreachable-modules.sh --strict-dead-paths

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

EXTRA_ARGS=()
for arg in "$@"; do
  case "$arg" in
    --report|--warn-only|--strict-dead-paths|--help|-h)
      EXTRA_ARGS+=("$arg")
      ;;
    *)
      EXTRA_ARGS+=("$arg")
      ;;
  esac
done

echo "→ Checking unreachable modules and dead code paths..."
exec cargo run --quiet --locked --bin check-unreachable-modules -- "${EXTRA_ARGS[@]}"
