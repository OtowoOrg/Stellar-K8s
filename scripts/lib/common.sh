#!/usr/bin/env bash
# scripts/lib/common.sh
# Shared utilities: repository resolution, dry-run parsing, and retry logic.

# ── Repository Resolution ───────────────────────────────────────────────────
_DEFAULT_REPO="OtowoOrg/Stellar-K8s"
export REPO="${REPO:-$_DEFAULT_REPO}"

if [[ ! "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "ERROR [validate repo]: REPO='$REPO' is not a valid 'owner/name' format." >&2
  echo "  Hint: Set REPO=owner/name or unset it to use the default ($_DEFAULT_REPO)." >&2
  exit 1
fi
echo "Active repository: $REPO"

# ── Retry & Dry-Run Logic ───────────────────────────────────────────────────
RETRY_MAX_ATTEMPTS="${RETRY_MAX_ATTEMPTS:-10}"
RETRY_DELAY_SECONDS="${RETRY_DELAY_SECONDS:-15}"

# create_issue <title> <labels> <body>
#   Respects DRY_RUN=true and REPO (optional --repo flag).
create_issue() {
  local title="$1"
  local labels="$2"
  local body="$3"

  if [[ "${DRY_RUN:-}" == "true" ]]; then
    echo "[DRY RUN] title:  ${title}"
    echo "[DRY RUN] labels: ${labels}"
    local preview
    preview=$(printf '%s' "${body}" | grep -v '^[[:space:]]*$' | head -2)
    echo "[DRY RUN] body (preview):"
    echo "${preview}" | sed 's/^/  /'
    echo ""
    return 0
  fi

  local attempt=0
  local gh_args=(issue create --repo "${REPO}" --title "${title}" --label "${labels}" --body "${body}")

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
