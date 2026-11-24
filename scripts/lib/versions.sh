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
# scripts/lib/versions.sh — single source of truth for pinned toolchain
# minimum versions.
#
# Sourced by scripts/preflight.sh so version pins stay in one place.
# Bump the version here once and every consumer picks it up automatically.
#
# Usage:
#   source "${SCRIPT_DIR}/lib/versions.sh"
#   version_ge "$installed" "$RUST_TOOLCHAIN" || echo "too old"

# ── Pinned minimum versions ───────────────────────────────────────────────────
RUST_TOOLCHAIN="1.92"    # keep in sync with .github/workflows/ci.yml lint job
KIND_VERSION="0.24.0"
KUBECTL_VERSION="1.30.0"
HELM_VERSION="3.16.0"

# ── version_ge: portable dotted-numeric version comparison ───────────────────
# Returns 0 (true) if $1 >= $2. Works without GNU `sort -V`, which isn't
# available on stock macOS/BSD, so this compares each dot-separated field
# numerically instead of shelling out.
#
#   version_ge "1.30.2" "1.30.0"  → true
#   version_ge "1.29.9" "1.30.0"  → false
#   version_ge "missing" "1.30.0" → false
version_ge() {
  local got="${1:-}" want="${2:-}"
  [[ -z "${got}" || "${got}" == "missing" ]] && return 1

  local IFS=.
  local -a got_parts=(${got})
  local -a want_parts=(${want})
  local i len=${#want_parts[@]}
  (( ${#got_parts[@]} > len )) && len=${#got_parts[@]}

  for (( i = 0; i < len; i++ )); do
    local g="${got_parts[i]:-0}" w="${want_parts[i]:-0}"
    g="${g//[^0-9]/}"; g="${g:-0}"
    w="${w//[^0-9]/}"; w="${w:-0}"
    if (( 10#${g} > 10#${w} )); then return 0; fi
    if (( 10#${g} < 10#${w} )); then return 1; fi
  done
  return 0
}
