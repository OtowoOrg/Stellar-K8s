#!/usr/bin/env bash
# scripts/lib/common.sh
# Shared helpers for batch issue-creation scripts.
#
# Usage: source "$(dirname "$0")/lib/common.sh"
# Requires: $REPO must already be set (source lib/repo.sh first).

# Tunables (all overridable via environment).
MAX_RETRIES="${MAX_RETRIES:-10}"
RETRY_DELAY="${RETRY_DELAY:-15}"
FORCE_CREATE="${FORCE_CREATE:-false}"

# Internal skip tracking — accumulated across all create_issue_with_retry calls
# in a single script run.  Printed by print_skip_summary.
_SKIPPED_ISSUES=()

# _issue_exists <title>
# Returns 0 (true) when an open issue with exactly this title already exists.
_issue_exists() {
  local title="$1"
  local existing
  existing=$(gh issue list --repo "$REPO" --state open --search "\"$title\" in:title" --json title --jq '.[].title' 2>/dev/null)
  # Exact-match against each returned title (the search is fuzzy on GitHub's side).
  while IFS= read -r found; do
    [[ "$found" == "$title" ]] && return 0
  done <<< "$existing"
  return 1
}

# create_issue_with_retry <title> <labels> <body>
#
# • Skips creation when an open issue with the same exact title exists,
#   unless FORCE_CREATE=true.
# • Retries up to MAX_RETRIES times on API failure, waiting RETRY_DELAY
#   seconds between attempts.
# • Exits non-zero only when all retry attempts are exhausted.
create_issue_with_retry() {
  local title="$1"
  local labels="$2"
  local body="$3"

  if [[ "$FORCE_CREATE" != "true" ]]; then
    local existing_number
    existing_number=$(gh issue list --repo "$REPO" --state open \
      --search "\"$title\" in:title" --json number,title \
      --jq ".[] | select(.title == \"$title\") | .number" 2>/dev/null | head -1)

    if [[ -n "$existing_number" ]]; then
      echo "⏭  Skipping (already exists as #${existing_number}): $title"
      _SKIPPED_ISSUES+=("#${existing_number}: $title")
      return 0
    fi
  fi

  local count=0
  while [[ "$count" -lt "$MAX_RETRIES" ]]; do
    if gh issue create --repo "$REPO" --title "$title" --label "$labels" --body "$body"; then
      echo "✓ Issue created: $title"
      return 0
    fi
    count=$(( count + 1 ))
    echo "API failed, retrying ($count/$MAX_RETRIES) in ${RETRY_DELAY}s..."
    sleep "$RETRY_DELAY"
  done

  echo "ERROR: Failed to create issue after $MAX_RETRIES attempts: $title" >&2
  exit 1
}

# print_skip_summary
# Call once at the end of each batch script to report skipped duplicates.
print_skip_summary() {
  local count="${#_SKIPPED_ISSUES[@]}"
  if [[ "$count" -eq 0 ]]; then
    return 0
  fi
  echo ""
  echo "── Duplicate skip summary ($count skipped) ──────────────────────────────"
  for entry in "${_SKIPPED_ISSUES[@]}"; do
    echo "   $entry"
  done
  echo "   Re-run with FORCE_CREATE=true to create them anyway."
  echo "────────────────────────────────────────────────────────────────────────"
}
