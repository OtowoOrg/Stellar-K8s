#!/usr/bin/env bash
#
# arm-upgrade.sh: arm a stellar-core protocol upgrade on one validator pod.
#
# Usage:
#   arm-upgrade.sh --namespace NS --pod POD --protocol-version N \
#                  --upgrade-time 2026-10-15T14:00:00Z [--container NAME] [--dry-run]
#
# Also supports: --clear (disarm) and --get (show what is armed).
# Run once per validator you operate. See docs/operations/protocol-upgrades.md.

set -euo pipefail

NAMESPACE=""
POD=""
CONTAINER="stellar-core"
PROTOCOL_VERSION=""
UPGRADE_TIME=""
DRY_RUN=false
MODE="set"

usage() {
  sed -n '3,10p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-1}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --namespace|-n)        NAMESPACE="${2:-}"; shift 2 ;;
    --pod)                 POD="${2:-}"; shift 2 ;;
    --container)           CONTAINER="${2:-}"; shift 2 ;;
    --protocol-version)    PROTOCOL_VERSION="${2:-}"; shift 2 ;;
    --upgrade-time)        UPGRADE_TIME="${2:-}"; shift 2 ;;
    --dry-run)             DRY_RUN=true; shift ;;
    --clear)               MODE="clear"; shift ;;
    --get)                 MODE="get"; shift ;;
    -h|--help)             usage 0 ;;
    *) echo "Unknown argument: $1" >&2; usage 1 ;;
  esac
done

[[ -n "$NAMESPACE" && -n "$POD" ]] || { echo "error: --namespace and --pod are required" >&2; usage 1; }
command -v kubectl >/dev/null || { echo "error: kubectl not found" >&2; exit 1; }

run_core() {
  # $1 = http-command path, for example "upgrades?mode=get"
  kubectl exec -n "$NAMESPACE" "$POD" -c "$CONTAINER" -- \
    stellar-core http-command "$1"
}

show_armed() {
  echo "Currently armed on ${NAMESPACE}/${POD}:"
  run_core "upgrades?mode=get"
}

case "$MODE" in
  get)
    show_armed
    exit 0
    ;;
  clear)
    if $DRY_RUN; then
      echo "[dry-run] would run: upgrades?mode=clear on ${NAMESPACE}/${POD}"
      exit 0
    fi
    run_core "upgrades?mode=clear"
    show_armed
    exit 0
    ;;
esac

# mode = set
[[ -n "$PROTOCOL_VERSION" && -n "$UPGRADE_TIME" ]] || {
  echo "error: --protocol-version and --upgrade-time are required to arm" >&2
  usage 1
}

[[ "$PROTOCOL_VERSION" =~ ^[0-9]+$ ]] || {
  echo "error: --protocol-version must be an integer" >&2; exit 1;
}

[[ "$UPGRADE_TIME" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]] || {
  echo "error: --upgrade-time must be UTC ISO 8601, e.g. 2026-10-15T14:00:00Z" >&2; exit 1;
}

# The upgrade time must be in the future (GNU date).
if now_epoch=$(date -u +%s) && target_epoch=$(date -u -d "$UPGRADE_TIME" +%s 2>/dev/null); then
  if (( target_epoch <= now_epoch )); then
    echo "error: --upgrade-time is in the past" >&2
    exit 1
  fi
fi

# Refuse to arm a node that is not synced.
STATE_JSON="$(run_core "info" || true)"
if command -v jq >/dev/null && [[ -n "$STATE_JSON" ]]; then
  STATE="$(echo "$STATE_JSON" | jq -r '.info.state // "unknown"')"
  echo "Node state: $STATE"
  if [[ "$STATE" != *Synced* ]]; then
    echo "error: node is not synced; refusing to arm" >&2
    exit 1
  fi
else
  echo "warning: could not verify node state (jq missing or no response)" >&2
fi

CMD="upgrades?mode=set&upgradetime=${UPGRADE_TIME}&protocolversion=${PROTOCOL_VERSION}"

if $DRY_RUN; then
  echo "[dry-run] would run on ${NAMESPACE}/${POD}: stellar-core http-command \"${CMD}\""
  exit 0
fi

echo "Arming protocol ${PROTOCOL_VERSION} at ${UPGRADE_TIME} on ${NAMESPACE}/${POD}"
run_core "$CMD"
show_armed
echo "Done. Repeat for each validator you operate."
