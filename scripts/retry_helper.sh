#!/usr/bin/env bash
# retry_helper.sh — Shared retry/backoff and dry-run helpers for issue batch scripts.
#
# Retry semantics:
#   - Each gh issue create call is retried up to RETRY_MAX_ATTEMPTS times.
#   - On failure, the script waits RETRY_DELAY_SECONDS before the next attempt.
#   - Attempt number and max attempts are logged on every retry.
#   - After exhausting all attempts the function exits with code 1.
#
# Dry-run semantics:
#   - Set DRY_RUN=true before running any batch script.
#   - In dry-run mode, issue title, labels, and the first two lines of the body
#     are printed to stdout; no GitHub API calls are made.
#   - The script exits with code 0 after processing all issues.
#
# Usage:
#   source "$(dirname "$0")/retry_helper.sh"
#   create_issue "Title" "label1,label2" "Body text..."

RETRY_MAX_ATTEMPTS="${RETRY_MAX_ATTEMPTS:-10}"
RETRY_DELAY_SECONDS="${RETRY_DELAY_SECONDS:-15}"

# create_issue <title> <labels> <body>
#   Respects DRY_RUN=true and REPO (optional --repo flag).
create_issue() {
  local title="$1"
  local labels="$2"
  local body="$3"

  # ── Dry-run path ──────────────────────────────────────────────────────────
  if [[ "${DRY_RUN:-}" == "true" ]]; then
    echo "[DRY RUN] title:  ${title}"
    echo "[DRY RUN] labels: ${labels}"
    # Print only the first two non-empty lines of the body for a quick preview.
    local preview
    preview=$(printf '%s' "${body}" | grep -v '^[[:space:]]*$' | head -2)
    echo "[DRY RUN] body (preview):"
    echo "${preview}" | sed 's/^/  /'
    echo ""
    return 0
  fi

  # ── Live path with retry/backoff ──────────────────────────────────────────
  local attempt=0
  local gh_args=(issue create --title "${title}" --label "${labels}" --body "${body}")
  if [[ -n "${REPO:-}" ]]; then
    gh_args=(issue create --repo "${REPO}" --title "${title}" --label "${labels}" --body "${body}")
  fi

  while [[ "${attempt}" -lt "${RETRY_MAX_ATTEMPTS}" ]]; do
    if gh "${gh_args[@]}"; then
      echo "✓ Issue created: ${title}"
      return 0
    fi
    attempt=$(( attempt + 1 ))
    echo "Attempt ${attempt}/${RETRY_MAX_ATTEMPTS} failed. Retrying in ${RETRY_DELAY_SECONDS}s..."
    sleep "${RETRY_DELAY_SECONDS}"
  done

  echo "ERROR: Failed to create issue after ${RETRY_MAX_ATTEMPTS} attempts: ${title}"
  return 1
}
