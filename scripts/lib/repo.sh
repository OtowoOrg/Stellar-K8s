#!/usr/bin/env bash
# scripts/lib/repo.sh
# Resolves the target GitHub repository, with env-based override and validation.
#
# Usage: source "$(dirname "$0")/lib/repo.sh"
# After sourcing, $REPO is set and validated.

_DEFAULT_REPO="OtowoOrg/Stellar-K8s"

# Allow override via REPO env variable; fall back to default.
REPO="${REPO:-$_DEFAULT_REPO}"

# Validate owner/name format (no slashes in either part, exactly one slash).
if [[ ! "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "ERROR: REPO='$REPO' is not a valid 'owner/name' format." >&2
  echo "       Set REPO=owner/name or unset it to use the default ($_DEFAULT_REPO)." >&2
  exit 1
fi

echo "Active repository: $REPO"
