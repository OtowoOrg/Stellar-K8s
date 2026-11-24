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
# tag-release.sh
#
# Creates an annotated release tag following docs/release-process.md:
#   - tag must be vX.Y.Z or vX.Y.Z-<pre> (e.g. v1.2.0-rc.1)
#   - tag version must match the `version` field in Cargo.toml
#   - tags are created on main only, from a clean working tree
#
# Usage:
#   scripts/tag-release.sh <vX.Y.Z[-pre]> [OPTIONS]
#
# Options:
#   --push      Push the tag to origin after creating it
#   --dry-run   Run all checks and print the tag command without creating it
#   -h, --help  Show this help message
# -----------------------------------------------------------------------------
set -euo pipefail

SEMVER_RE='^v[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)\.[0-9]+)?$'
CARGO_FILE="Cargo.toml"
RELEASE_BRANCH="main"

TAG=""
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
    *)
      [[ -z "$TAG" ]] || die "unexpected argument: $1"
      TAG="$1"; shift ;;
  esac
done

[[ -n "$TAG" ]] || { usage >&2; exit 1; }

# ── Checks ────────────────────────────────────────────────────────────────────
[[ "$TAG" =~ $SEMVER_RE ]] || die "tag '$TAG' does not match vX.Y.Z or vX.Y.Z-<pre>"

REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT"

CARGO_VERSION=$(grep -m1 '^version' "$CARGO_FILE" | sed -E 's/^version *= *"([^"]+)".*/\1/')
[[ "${TAG#v}" == "$CARGO_VERSION" ]] \
  || die "tag version '${TAG#v}' does not match $CARGO_FILE version '$CARGO_VERSION'"

BRANCH=$(git rev-parse --abbrev-ref HEAD)
[[ "$BRANCH" == "$RELEASE_BRANCH" ]] \
  || die "tags must be created on '$RELEASE_BRANCH' (currently on '$BRANCH')"

[[ -z "$(git status --porcelain)" ]] || die "working tree is not clean"

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null \
  && die "tag '$TAG' already exists"

# ── Create ────────────────────────────────────────────────────────────────────
echo "→ Tag:    $TAG"
echo "→ Commit: $(git rev-parse --short HEAD)"

if [[ "$DRY_RUN" == true ]]; then
  echo "[dry-run] git tag -a $TAG -m \"Release $TAG\""
  $PUSH && echo "[dry-run] git push origin $TAG"
  exit 0
fi

git tag -a "$TAG" -m "Release $TAG"
echo "✓ Created annotated tag $TAG"

if [[ "$PUSH" == true ]]; then
  git push origin "$TAG"
  echo "✓ Pushed $TAG to origin"
else
  echo "Push with: git push origin $TAG (or re-run with --push)"
fi
