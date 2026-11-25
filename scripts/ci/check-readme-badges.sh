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
#
# scripts/ci/check-readme-badges.sh — README workflow badge check (#1896).
#
# The README's CI badge is GitHub's live status image for
# .github/workflows/ci.yml, so there is nothing to regenerate; what can break
# is a badge or link pointing at a workflow file that was renamed or removed.
# This check fails when:
#   • the CI status badge (actions/workflows/ci.yml/badge.svg) is missing
#   • any workflow referenced by a badge or link
#     (actions/workflows/<file>.yml or shields.io github/actions/workflow/status)
#     has no matching file under .github/workflows/
#
# Usage:
#   bash scripts/ci/check-readme-badges.sh            # checks README.md
#   bash scripts/ci/check-readme-badges.sh <readme>   # checks another file
#
# Make target: make ci-badge
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
README="${1:-$ROOT/README.md}"
WORKFLOWS_DIR="${WORKFLOWS_DIR:-$ROOT/.github/workflows}"
REPO_SLUG="OtowoOrg/Stellar-K8s"

if [[ ! -f "$README" ]]; then
  echo "✗ README not found: $README" >&2
  exit 1
fi

errors=0

ci_badge="https://github.com/${REPO_SLUG}/actions/workflows/ci.yml/badge.svg"
if grep -qF "$ci_badge" "$README"; then
  echo "  ✓ CI badge present ($ci_badge)"
else
  echo "  ✗ CI badge missing: expected $ci_badge" >&2
  errors=$((errors + 1))
fi

workflows="$(
  {
    grep -oE "github\.com/${REPO_SLUG}/actions/workflows/[A-Za-z0-9_.-]+\.ya?ml" "$README" || true
    grep -oE "shields\.io/github/actions/workflow/status/${REPO_SLUG}/[A-Za-z0-9_.-]+\.ya?ml" "$README" || true
  } | sed -E 's#.*/##' | sort -u
)"

if [[ -z "$workflows" ]]; then
  echo "  ✗ No workflow badges found in $README" >&2
  errors=$((errors + 1))
fi

while IFS= read -r workflow; do
  [[ -z "$workflow" ]] && continue
  if [[ -f "$WORKFLOWS_DIR/$workflow" ]]; then
    echo "  ✓ $workflow"
  else
    echo "  ✗ $workflow is referenced in the README but .github/workflows/$workflow does not exist" >&2
    errors=$((errors + 1))
  fi
done <<< "$workflows"

if [[ "$errors" -gt 0 ]]; then
  echo "✗ README badge check failed ($errors problem(s))" >&2
  exit 1
fi
echo "✓ README workflow badges point at existing workflows"
