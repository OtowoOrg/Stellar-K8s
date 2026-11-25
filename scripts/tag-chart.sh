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
# -----------------------------------------------------------------------------
# tag-chart.sh
#
# Creates an annotated chart tag for the current commit using the version
# declared in the operator Helm chart (issue #1940):
#   - tag format is chart-vX.Y.Z (e.g. chart-v4.1.0), matching the existing
#     chart-v* tags created by helm-release.yml / bump-chart-version.sh
#   - the version is read dynamically from charts/stellar-operator/Chart.yaml
#     (never hardcoded)
#   - tags are created on main only, from a clean working tree, on HEAD
#   - existing tags are never overwritten (no force), locally or on the remote
#
# Usage:
#   scripts/tag-chart.sh [OPTIONS]
#
# Options:
#   --push      Push the tag to origin after creating it
#   --dry-run   Run all checks and print the tag commands without creating it
#   -h, --help  Show this help message
# -----------------------------------------------------------------------------
set -euo pipefail

CHART_YAML="charts/stellar-operator/Chart.yaml"
REMOTE="origin"
RELEASE_BRANCH="main"
# SemVer core plus optional pre-release/build suffix (Helm chart versions).
SEMVER_RE='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'

PUSH=false
DRY_RUN=false

usage() {
  sed -n '/^# Usage:/,/^# -----/p' "$0" | sed 's/^# \{0,1\}//' | sed '$d'
}

die() {
  echo "error: $*" >&2
  exit 1
}

# ── Argument parsing ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --push)     PUSH=true; shift ;;
    --dry-run)  DRY_RUN=true; shift ;;
    -h|--help)  usage; exit 0 ;;
    -*)         die "unknown option: $1" ;;
    *)          die "unexpected argument: $1 (this target takes no positional arguments)" ;;
  esac
done

command -v git >/dev/null 2>&1 || die "git is not available on PATH"

REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null) \
  || die "not inside a Git repository (git rev-parse --show-toplevel failed)"
cd "$REPO_ROOT"

# ── Read the chart version ────────────────────────────────────────────────────
[[ -f "$CHART_YAML" ]] || die "chart metadata not found: $CHART_YAML"

VERSION_LINE=$(grep -m1 -E '^version:' "$CHART_YAML" || true)
[[ -n "$VERSION_LINE" ]] || die "no 'version:' key found in $CHART_YAML"
VERSION=$(sed -E 's/^version:[[:space:]]*//; s/["'"'"']//g' <<<"$VERSION_LINE")
[[ -n "$VERSION" ]] || die "empty 'version:' value in $CHART_YAML"
[[ "$VERSION" =~ $SEMVER_RE ]] || die "invalid chart version '$VERSION' in $CHART_YAML (expected X.Y.Z)"

TAG="chart-v${VERSION}"

# ── Resolve the current commit ────────────────────────────────────────────────
HEAD_SHA=$(git rev-parse -q --verify HEAD) \
  || die "cannot resolve the current commit (HEAD)"

# ── Branch and working-tree guards ────────────────────────────────────────────
BRANCH=$(git rev-parse --abbrev-ref HEAD)
[[ "$BRANCH" == "$RELEASE_BRANCH" ]] \
  || die "tags must be created on '$RELEASE_BRANCH' (currently on '$BRANCH')"

[[ -z "$(git status --porcelain)" ]] \
  || die "working tree is not clean; commit or stash changes before tagging"

# ── Refuse to overwrite an existing tag ───────────────────────────────────────
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null \
  && die "tag '$TAG' already exists locally (never overwriting tags)"

git remote get-url "$REMOTE" >/dev/null 2>&1 \
  || die "remote '$REMOTE' is not configured; refusing to guess a push target"

# ls-remote --exit-code: 0 = ref found, 2 = no match, anything else = query
# failure (network/auth). Treat only "no match" as success.
set +e
git ls-remote --exit-code --tags "$REMOTE" "refs/tags/$TAG" >/dev/null 2>&1
LS_RC=$?
set -e
case "$LS_RC" in
  0) die "tag '$TAG' already exists on remote '$REMOTE' (never overwriting tags)" ;;
  2) ;; # not found — safe to create
  *) die "could not query remote '$REMOTE' for existing tag '$TAG' (git ls-remote exit $LS_RC); refusing to risk a duplicate" ;;
esac

# ── Report ────────────────────────────────────────────────────────────────────
echo "→ Chart:   $CHART_YAML"
echo "→ Version: $VERSION"
echo "→ Tag:     $TAG"
echo "→ Commit:  $(git rev-parse --short HEAD)"
echo "→ Remote:  $REMOTE"

if [[ "$DRY_RUN" == true ]]; then
  echo "[dry-run] git tag -a $TAG -m \"Helm chart release v$VERSION\""
  $PUSH && echo "[dry-run] git push $REMOTE $TAG"
  echo "[dry-run] no tag was created"
  exit 0
fi

# ── Create ────────────────────────────────────────────────────────────────────
# Annotated tag on the exact commit resolved above; never -f/--force.
git tag -a "$TAG" -m "Helm chart release v${VERSION}" \
  || die "failed to create tag '$TAG'"
echo "✓ Created annotated tag $TAG on $(git rev-parse --short HEAD)"

if [[ "$PUSH" == true ]]; then
  if git push "$REMOTE" "$TAG"; then
    echo "✓ Pushed $TAG to $REMOTE"
  else
    echo "error: created '$TAG' locally but pushing to '$REMOTE' failed" >&2
    echo "error: the local tag is correct; retry with: git push $REMOTE $TAG" >&2
    echo "error: to discard it instead: git tag -d $TAG" >&2
    exit 1
  fi
else
  echo "Push with: git push $REMOTE $TAG (or re-run with --push)"
fi
