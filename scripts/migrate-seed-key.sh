#!/usr/bin/env bash
#
# migrate-seed-key.sh - Migrate legacy SEED secret keys to STELLAR_CORE_SEED
#
# Issue #1554: Seed Secret Key Standardization
#
# This helper migrates Kubernetes secrets that use the legacy `SEED` key to
# also include the standardized `STELLAR_CORE_SEED` key. The legacy `SEED`
# key is left in place for backwards compatibility so existing deployments
# continue to work during the transition.
#
# Behavior:
#   - Reads the target secret(s).
#   - If a secret contains `SEED` but NOT `STELLAR_CORE_SEED`, it patches the
#     secret to add `STELLAR_CORE_SEED` with the same value as `SEED`.
#   - If a secret already has `STELLAR_CORE_SEED`, it is left untouched.
#   - If a secret has neither key, it is skipped.
#   - Secret values are NEVER echoed to stdout/stderr.
#
# Usage:
#   scripts/migrate-seed-key.sh [--dry-run] <namespace> <secret-name>
#   scripts/migrate-seed-key.sh [--dry-run] --all <namespace>
#
# Options:
#   --dry-run   Show what would be changed without modifying any secret.
#   --all       Process all secrets in the given namespace.
#   -h, --help  Show this help text.
#
# Examples:
#   # Preview changes for a single secret (recommended first step):
#   scripts/migrate-seed-key.sh --dry-run stellar-system stellar-core-seed
#
#   # Migrate a single secret:
#   scripts/migrate-seed-key.sh stellar-system stellar-core-seed
#
#   # Preview changes for every secret in a namespace:
#   scripts/migrate-seed-key.sh --dry-run --all stellar-system
#
#   # Migrate every secret in a namespace:
#   scripts/migrate-seed-key.sh --all stellar-system
#
# Dry-run test invocation note:
#   Always run with `--dry-run` first to verify the set of secrets that would
#   be modified. A dry-run performs only read operations (kubectl get) and
#   never patches anything, so it is safe to run against production clusters.
#
# Requirements:
#   - kubectl configured with access to the target namespace.
#   - jq for JSON parsing.
#
# Exit codes:
#   0  Success (including dry-run).
#   1  Usage error or missing dependency.
#   2  A kubectl operation failed.

set -euo pipefail

LEGACY_KEY="SEED"
NEW_KEY="STELLAR_CORE_SEED"

DRY_RUN=false
ALL=false
NAMESPACE=""
SECRET_NAME=""

usage() {
  sed -n '2,60p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

log() {
  printf '%s\n' "$*" >&2
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    log "ERROR: required command '$1' not found in PATH"
    exit 1
  fi
}

# Parse arguments.
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      DRY_RUN=true
      shift
      ;;
    --all)
      ALL=true
      shift
      ;;
    -h|--help)
      usage 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      log "ERROR: unknown option '$1'"
      usage 1
      ;;
    *)
      if [[ -z "$NAMESPACE" ]]; then
        NAMESPACE="$1"
      elif [[ -z "$SECRET_NAME" ]]; then
        SECRET_NAME="$1"
      else
        log "ERROR: unexpected argument '$1'"
        usage 1
      fi
      shift
      ;;
  esac
done

if [[ -z "$NAMESPACE" ]]; then
  log "ERROR: namespace is required"
  usage 1
fi

if [[ "$ALL" == "true" && -n "$SECRET_NAME" ]]; then
  log "ERROR: cannot specify both --all and a secret name"
  usage 1
fi

if [[ "$ALL" == "false" && -z "$SECRET_NAME" ]]; then
  log "ERROR: secret name is required (or use --all)"
  usage 1
fi

require_cmd kubectl
require_cmd jq

# Counters for the summary.
TOTAL=0
MIGRATED=0
ALREADY_OK=0
SKIPPED=0
FAILED=0

# Migrate a single secret. Never prints secret values.
migrate_secret() {
  local ns="$1"
  local name="$2"

  TOTAL=$((TOTAL + 1))

  local json
  if ! json="$(kubectl get secret "$name" -n "$ns" -o json 2>/dev/null)"; then
    log "ERROR: failed to read secret '$ns/$name'"
    FAILED=$((FAILED + 1))
    return
  fi

  local has_legacy has_new
  has_legacy="$(printf '%s' "$json" | jq -r --arg k "$LEGACY_KEY" '(.data // {}) | has($k)')"
  has_new="$(printf '%s' "$json" | jq -r --arg k "$NEW_KEY" '(.data // {}) | has($k)')"

  if [[ "$has_new" == "true" ]]; then
    log "OK       $ns/$name: already has $NEW_KEY, nothing to do"
    ALREADY_OK=$((ALREADY_OK + 1))
    return
  fi

  if [[ "$has_legacy" != "true" ]]; then
    log "SKIP     $ns/$name: no $LEGACY_KEY key present"
    SKIPPED=$((SKIPPED + 1))
    return
  fi

  if [[ "$DRY_RUN" == "true" ]]; then
    log "DRY-RUN  $ns/$name: would add $NEW_KEY from $LEGACY_KEY"
    MIGRATED=$((MIGRATED + 1))
    return
  fi

  # Build a patch that copies the legacy value into the new key without ever
  # exposing the value on the command line or in logs.
  local patch
  patch="$(printf '%s' "$json" | jq -c --arg legacy "$LEGACY_KEY" --arg new "$NEW_KEY" \
    '{data: {($new): (.data[$legacy])}}')"

  if kubectl patch secret "$name" -n "$ns" --type merge -p "$patch" >/dev/null 2>&1; then
    log "MIGRATED $ns/$name: added $NEW_KEY (legacy $LEGACY_KEY retained)"
    MIGRATED=$((MIGRATED + 1))
  else
    log "ERROR    $ns/$name: failed to patch secret"
    FAILED=$((FAILED + 1))
  fi
}

if [[ "$ALL" == "true" ]]; then
  log "Scanning all secrets in namespace '$NAMESPACE'..."
  # Collect secret names first to avoid issues with partial reads.
  if ! secret_names="$(kubectl get secrets -n "$NAMESPACE" -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}' 2>/dev/null)"; then
    log "ERROR: failed to list secrets in namespace '$NAMESPACE'"
    exit 2
  fi

  if [[ -z "$secret_names" ]]; then
    log "No secrets found in namespace '$NAMESPACE'."
  else
    while IFS= read -r name; do
      [[ -z "$name" ]] && continue
      migrate_secret "$NAMESPACE" "$name"
    done <<< "$secret_names"
  fi
else
  migrate_secret "$NAMESPACE" "$SECRET_NAME"
fi

# Summary (never includes secret values).
log ""
log "Summary:"
log "  Total secrets examined : $TOTAL"
log "  Migrated               : $MIGRATED"
log "  Already had $NEW_KEY : $ALREADY_OK"
log "  Skipped (no $LEGACY_KEY)  : $SKIPPED"
log "  Failed                 : $FAILED"
if [[ "$DRY_RUN" == "true" ]]; then
  log "  (dry-run: no changes were applied)"
fi

if [[ "$FAILED" -gt 0 ]]; then
  exit 2
fi

exit 0