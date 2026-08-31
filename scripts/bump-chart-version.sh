#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# bump-chart-version.sh
#
# Analyzes recent git commits using Conventional Commits format and determines
# the appropriate SemVer bump for the Helm chart.
#
# Conventional Commits reference:
#   https://www.conventionalcommits.org/
#
# Bump rules:
#   MAJOR  — any commit with "BREAKING CHANGE:" footer OR "feat!:" / "fix!:" etc.
#   MINOR  — any "feat:" commit (without breaking)
#   PATCH  — "fix:", "perf:", "refactor:", "revert:" (without breaking/feat)
#   NONE   — "docs:", "chore:", "ci:", "test:", "style:", "build:" only
#
# Minimum bump floor:
#   The `versioning.min-bump` annotation in Chart.yaml (major | minor | patch |
#   none) declares a floor that overrides the conventional-commit result. e.g.
#   min-bump: patch forces a patch release even when only docs/chore commits
#   landed. Set to "none" (the default) to disable.
#
# Usage:
#   scripts/bump-chart-version.sh [OPTIONS]
#
# Options:
#   --chart-path PATH      Path to chart directory (default: charts/stellar-operator)
#   --bump-override TYPE   Force bump type: major | minor | patch
#   --output-env           Write outputs to $GITHUB_OUTPUT (GitHub Actions mode)
#   --dry-run              Print result but do not modify Chart.yaml
#   --since REF            Analyze commits since REF instead of last chart tag
#   -h, --help             Show this help message
#
# Outputs (stdout unless --output-env):
#   BUMP_TYPE        — major | minor | patch | none
#   CURRENT_VERSION  — e.g. 0.1.0
#   NEW_VERSION      — e.g. 0.2.0
#   CHART_TAG        — e.g. chart-v0.1.0
#   HAS_CHANGES      — true | false
#   CHANGELOG_ENTRY  — multiline list of commit subjects
# -----------------------------------------------------------------------------
set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────────
CHART_PATH="charts/stellar-operator"
BUMP_OVERRIDE=""
OUTPUT_ENV=false
DRY_RUN=false
SINCE_REF=""

# ── Helpers ───────────────────────────────────────────────────────────────────
log()  { echo "[bump-chart-version] $*" >&2; }
die()  { echo "[bump-chart-version] ERROR: $*" >&2; exit 1; }

usage() {
  grep '^# ' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 0
}

semver_bump() {
  local current="$1" bump="$2"
  local major minor patch

  IFS='.' read -r major minor patch <<< "${current%-*}"   # strip pre-release
  major="${major:-0}"
  minor="${minor:-0}"
  patch="${patch:-0}"

  case "$bump" in
    major) echo "$((major + 1)).0.0" ;;
    minor) echo "${major}.$((minor + 1)).0" ;;
    patch) echo "${major}.${minor}.$((patch + 1))" ;;
    none)  echo "${major}.${minor}.${patch}" ;;
    *)     die "Unknown bump type: $bump" ;;
  esac
}

# ── Argument parsing ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --chart-path)    CHART_PATH="$2";    shift 2 ;;
    --bump-override) BUMP_OVERRIDE="$2"; shift 2 ;;
    --output-env)    OUTPUT_ENV=true;    shift   ;;
    --dry-run)       DRY_RUN=true;       shift   ;;
    --since)         SINCE_REF="$2";     shift 2 ;;
    -h|--help)       usage ;;
    *)               die "Unknown argument: $1. Use --help for usage." ;;
  esac
done

# ── Validate chart path ───────────────────────────────────────────────────────
CHART_YAML="${CHART_PATH}/Chart.yaml"
[[ -f "$CHART_YAML" ]] || die "Chart.yaml not found at: $CHART_YAML"

# ── Read current version from Chart.yaml ─────────────────────────────────────
CURRENT_VERSION=$(grep -E '^version:' "$CHART_YAML" | head -1 | sed 's/version:[[:space:]]*//')
CURRENT_VERSION="${CURRENT_VERSION//\"/}"   # strip quotes
[[ -n "$CURRENT_VERSION" ]] || die "Could not parse version from $CHART_YAML"
log "Current chart version: $CURRENT_VERSION"

# ── Read versioning.min-bump annotation ──────────────────────────────────────
# The annotation (in Chart.yaml annotations) declares a minimum bump floor:
#   major | minor | patch | none
# When the conventionally-derived bump type is below this floor, the bump is
# promoted to the floor. e.g. min-bump: patch forces a patch release even when
# only docs/chore commits landed.
MIN_BUMP="none"
if [[ -f "$CHART_YAML" ]]; then
  MIN_BUMP=$(awk '
    /^  versioning\.min-bump:/ { gsub(/^[[:space:]]*versioning\.min-bump:[[:space:]]*/, ""); gsub(/[[:space:]]+$/, ""); gsub(/["'"'"']/, ""); print; exit }
  ' "$CHART_YAML")
  MIN_BUMP="${MIN_BUMP:-none}"
  case "$MIN_BUMP" in
    major|minor|patch|none) ;;
    *) log "Ignoring unknown versioning.min-bump value: $MIN_BUMP"; MIN_BUMP="none" ;;
  esac
fi
log "Minimum bump floor (versioning.min-bump): $MIN_BUMP"

# ── Find the last chart git tag ───────────────────────────────────────────────
CHART_TAG=""
if [[ -n "$SINCE_REF" ]]; then
  SINCE="$SINCE_REF"
  log "Analyzing commits since explicit ref: $SINCE"
else
  # Look for tags matching chart-v* pattern
  CHART_TAG=$(git tag --list "chart-v*" --sort=-version:refname 2>/dev/null | head -1 || true)
  if [[ -n "$CHART_TAG" ]]; then
    SINCE="$CHART_TAG"
    log "Last chart tag: $CHART_TAG — analyzing commits since then"
  else
    # Fallback: analyze all commits (no ref). Leaving SINCE empty makes the
    # git log below run without a "ref..HEAD" range, so the very first commit
    # is not excluded by a root-commit SHA compare.
    SINCE=""
    log "No chart tag found — analyzing all commits since repo root"
  fi
fi

# ── Collect commit messages since last tag ────────────────────────────────────
if [[ -n "$SINCE" ]]; then
  COMMITS=$(git log "${SINCE}..HEAD" --pretty=format:"%s%n%b" 2>/dev/null || true)
else
  COMMITS=$(git log --pretty=format:"%s%n%b" 2>/dev/null || true)
fi

if [[ -z "$COMMITS" ]]; then
  log "No commits found since $SINCE"
  HAS_CHANGES="false"
  BUMP_TYPE="none"
  NEW_VERSION="$CURRENT_VERSION"
  CHANGELOG_ENTRY=""
else
  HAS_CHANGES="true"
fi

# ── Parse conventional commits and determine bump type ────────────────────────
if [[ "$HAS_CHANGES" == "true" ]]; then
  BUMP_TYPE="none"
  BREAKING=false
  HAS_FEAT=false
  HAS_PATCH=false
  CHANGELOG_LINES=()

  while IFS= read -r line; do
    [[ -z "$line" ]] && continue

    # Detect breaking changes: footer "BREAKING CHANGE:" or "!" suffix
    if echo "$line" | grep -qiE '(BREAKING[[:space:]]CHANGE:|^[a-z]+(\([^)]+\))?!:)'; then
      BREAKING=true
      CHANGELOG_LINES+=("💥 $line")
      continue
    fi

    # feat / feat(scope):
    if echo "$line" | grep -qE '^feat(\([^)]+\))?:'; then
      HAS_FEAT=true
      CHANGELOG_LINES+=("✨ $line")
      continue
    fi

    # fix / fix(scope): / perf: / refactor: / revert:
    if echo "$line" | grep -qE '^(fix|perf|refactor|revert)(\([^)]+\))?:'; then
      HAS_PATCH=true
      CHANGELOG_LINES+=("🐛 $line")
      continue
    fi

    # Informational (docs, chore, ci, test, style, build)
    if echo "$line" | grep -qE '^(docs|chore|ci|test|style|build)(\([^)]+\))?:'; then
      CHANGELOG_LINES+=("📝 $line")
      continue
    fi

    # Unrecognised format — treat as informational
    CHANGELOG_LINES+=("• $line")
  done <<< "$COMMITS"

  # Determine final bump type
  if [[ "$BREAKING" == "true" ]]; then
    BUMP_TYPE="major"
  elif [[ "$HAS_FEAT" == "true" ]]; then
    BUMP_TYPE="minor"
  elif [[ "$HAS_PATCH" == "true" ]]; then
    BUMP_TYPE="patch"
  else
    BUMP_TYPE="none"
    HAS_CHANGES="false"   # only docs/chore/ci — not worth a release
    log "Only documentation/maintenance commits found — skipping release"
  fi

  CHANGELOG_ENTRY=$(printf '%s\n' "${CHANGELOG_LINES[@]:-}")
fi

# ── Apply the minimum bump floor (versioning.min-bump) ───────────────────────
# Promote a too-small bump type up to the declared floor. Only applied when
# there were actually commits to release (avoids forcing a release on a repo
# with no new commits since the last chart tag).
if [[ -n "$MIN_BUMP" ]] && [[ "$BUMP_TYPE" != "major" ]] && [[ -n "$COMMITS" ]]; then
  case "$BUMP_TYPE:$MIN_BUMP" in
    none:patch|none:minor|none:major|patch:minor|patch:major|minor:major)
      log "Promoting ${BUMP_TYPE} bump to minimum floor: ${MIN_BUMP}"
      BUMP_TYPE="$MIN_BUMP"
      HAS_CHANGES="true"
      ;;
  esac
fi

# ── Apply override if provided ────────────────────────────────────────────────
if [[ -n "$BUMP_OVERRIDE" ]]; then
  log "Applying manual bump override: $BUMP_OVERRIDE (was: $BUMP_TYPE)"
  BUMP_TYPE="$BUMP_OVERRIDE"
  HAS_CHANGES="true"
fi

# ── Calculate new version ─────────────────────────────────────────────────────
NEW_VERSION=$(semver_bump "$CURRENT_VERSION" "$BUMP_TYPE")
log "Bump type: $BUMP_TYPE → new version: $NEW_VERSION"

# ── Apply to Chart.yaml (unless dry-run or none) ──────────────────────────────
if [[ "$DRY_RUN" == "false" && "$BUMP_TYPE" != "none" && "$HAS_CHANGES" == "true" ]]; then
  # Use sed to update in-place; compatible with GNU and BSD sed
  sed -i.bak "s/^version:.*/version: ${NEW_VERSION}/" "$CHART_YAML"
  sed -i.bak "s/^appVersion:.*/appVersion: \"${NEW_VERSION}\"/" "$CHART_YAML"
  rm -f "${CHART_YAML}.bak"
  log "Chart.yaml updated to version ${NEW_VERSION}"
else
  if [[ "$DRY_RUN" == "true" ]]; then
    log "(dry-run) Would update Chart.yaml to version ${NEW_VERSION}"
  elif [[ "$BUMP_TYPE" == "none" ]]; then
    log "No version bump required"
  fi
fi

# ── Output results ────────────────────────────────────────────────────────────
NEW_CHART_TAG="chart-v${NEW_VERSION}"

if [[ "$OUTPUT_ENV" == "true" ]]; then
  # GitHub Actions output format
  {
    echo "bump_type=${BUMP_TYPE}"
    echo "current_version=${CURRENT_VERSION}"
    echo "new_version=${NEW_VERSION}"
    echo "chart_tag=${CHART_TAG:-}"
    echo "has_changes=${HAS_CHANGES}"
    # Multiline output — use heredoc delimiter
    echo "changelog_entry<<CHANGELOG_EOF"
    echo "${CHANGELOG_ENTRY:-}"
    echo "CHANGELOG_EOF"
  } >> "${GITHUB_OUTPUT:-/dev/stdout}"

  log "Outputs written to GITHUB_OUTPUT"
else
  # Human-readable output
  cat <<EOF

=== Helm Chart Version Bump Results ===
  Chart path      : ${CHART_PATH}
  Current version : ${CURRENT_VERSION}
  Last chart tag  : ${CHART_TAG:-<none>}
  Commits analyzed: since ${SINCE:-<beginning>}
  Bump type       : ${BUMP_TYPE}
  New version     : ${NEW_VERSION}
  New chart tag   : ${NEW_CHART_TAG}
  Has changes     : ${HAS_CHANGES}

--- Changelog entry ---
${CHANGELOG_ENTRY:-<no releasable changes>}
EOF
fi
