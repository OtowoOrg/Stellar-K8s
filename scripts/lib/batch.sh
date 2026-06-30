#!/usr/bin/env bash
# scripts/lib/batch.sh
# Shared helpers for archived batch issue-creation scripts.
#
# Usage (from scripts/archive/*.sh):
#   source "$(dirname "$0")/../lib/batch.sh"
#   BATCH_HELP_DESC="Creates GitHub issues for batch N."
#   batch_parse_help "$@"
#   batch_validate_issue_count "${EXPECTED_ISSUE_COUNT:-0}"

# shellcheck source=common.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

# batch_show_help [description]
#   Print standard usage text. Set BATCH_HELP_DESC before calling batch_parse_help.
batch_show_help() {
  local description="${1:-${BATCH_HELP_DESC:-Creates GitHub issues for Stellar-K8s.}}"
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

${description}

Prerequisites:
  - gh CLI installed and authenticated (gh auth login)
  - Network access to api.github.com

Optional environment variables:
  REPO                Target repository (default: OtowoOrg/Stellar-K8s)
  DRY_RUN             Set to true or 1 to print commands without executing
  RETRY_MAX_ATTEMPTS  Number of retry attempts on API failure (default: 10)
  RETRY_DELAY_SECONDS Seconds to wait between retries (default: 15)

Example:
  REPO=myorg/my-fork DRY_RUN=1 $(basename "$0")
EOF
}

# batch_parse_help "$@"
#   Exit 0 after printing help when -h/--help is passed.
batch_parse_help() {
  for arg in "$@"; do
    case "$arg" in
      -h | --help)
        batch_show_help "${BATCH_HELP_DESC:-Creates GitHub issues for Stellar-K8s.}"
        exit 0
        ;;
    esac
  done
}

# batch_validate_issue_count <expected> [script_path]
#   Sanity-check that the script contains the expected number of issue calls.
batch_validate_issue_count() {
  local expected="$1"
  local script="${2:-$0}"
  local actual

  actual=$(grep -cE '^[[:space:]]*(create_issue|create_issue_with_retry|gh issue create)' "$script" || true)
  if [[ "$actual" -ne "$expected" ]]; then
    echo "ERROR: Expected ${expected} issue create calls, found ${actual} in ${script}." >&2
    echo "  Hint: Update EXPECTED_ISSUE_COUNT or fix the script." >&2
    exit 1
  fi
}

# create_issue_with_retry — backward-compatible alias used by archived batch scripts.
create_issue_with_retry() {
  create_issue "$1" "$2" "$3"
}
