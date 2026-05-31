#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
source "$(dirname "$0")/lib/repo.sh"

# Create all necessary labels for Stellar Wave issues
# Uses || true to ignore errors if label already exists

echo "Creating labels..."

gh label create --repo "$REPO" rust --color DEA584 --description "Rust related" || true
gh label create --repo "$REPO" soroban --color 7F129E --description "Soroban smart contracts" || true
gh label create --repo "$REPO" observability --color C2E0C6 --description "Metrics and logs" || true
gh label create --repo "$REPO" ci --color 0075ca --description "CI/CD" || true
gh label create --repo "$REPO" security --color d73a4a --description "Security related" || true
gh label create --repo "$REPO" reliability --color d93f0b --description "Reliability and stability" || true
gh label create --repo "$REPO" architecture --color 0e8a16 --description "Architecture design" || true
gh label create --repo "$REPO" logic --color 5319e7 --description "Business logic" || true
gh label create --repo "$REPO" kubernetes --color 326ce5 --description "Kubernetes related" || true
gh label create --repo "$REPO" feature --color a2eeef --description "New feature" || true
gh label create --repo "$REPO" testing --color C2E0C6 --description "Tests" || true
#!/bin/bash
# Create all necessary labels for Stellar Wave issues and export reusable label
# constants. Other scripts may source this file to reference consistent label
# combinations:
#
#   source "$(dirname "$0")/create_labels.sh" --source-only
#
# and then use variables like $LABEL_K8S_FEATURE in their gh issue create calls.
#
# To add a new label:
#   1. Add a `gh label create` line below with the colour and description.
#   2. Export a matching LABEL_* constant so every batch script can use it.

# ── Constants (export so sourcing scripts inherit them) ───────────────────────

# Individual labels
export LABEL_RUST="rust"
export LABEL_SOROBAN="soroban"
export LABEL_OBSERVABILITY="observability"
export LABEL_CI="ci"
export LABEL_SECURITY="security"
export LABEL_RELIABILITY="reliability"
export LABEL_ARCHITECTURE="architecture"
export LABEL_LOGIC="logic"
export LABEL_KUBERNETES="kubernetes"
export LABEL_FEATURE="feature"
export LABEL_TESTING="testing"
export LABEL_WAVE="stellar-wave"
export LABEL_GOOD_FIRST="good-first-issue"

# Common combinations used across batch scripts
export LABEL_K8S_GOOD_FIRST="${LABEL_WAVE},${LABEL_GOOD_FIRST},${LABEL_KUBERNETES}"
export LABEL_K8S_FEATURE="${LABEL_WAVE},${LABEL_KUBERNETES},${LABEL_FEATURE}"
export LABEL_ARCH_LOGIC="${LABEL_WAVE},${LABEL_ARCHITECTURE},${LABEL_LOGIC}"
export LABEL_ARCH_FEATURE="${LABEL_WAVE},${LABEL_ARCHITECTURE},${LABEL_FEATURE}"
export LABEL_LOGIC_FEATURE="${LABEL_WAVE},${LABEL_LOGIC},${LABEL_FEATURE}"
export LABEL_OBS_FEATURE="${LABEL_WAVE},${LABEL_OBSERVABILITY},${LABEL_FEATURE}"
export LABEL_RELIABILITY_RUST="${LABEL_WAVE},${LABEL_RELIABILITY},${LABEL_RUST}"
export LABEL_RELIABILITY_AUTO="${LABEL_WAVE},${LABEL_RELIABILITY},automation"

# ── Label creation (skip when sourced with --source-only) ────────────────────

if [[ "${1:-}" == "--source-only" ]]; then
  return 0 2>/dev/null || exit 0
fi

echo "Creating labels..."

gh label create "$LABEL_RUST"          --color DEA584 --description "Rust related"              || true
gh label create "$LABEL_SOROBAN"       --color 7F129E --description "Soroban smart contracts"    || true
gh label create "$LABEL_OBSERVABILITY" --color C2E0C6 --description "Metrics and logs"           || true
gh label create "$LABEL_CI"            --color 0075ca --description "CI/CD"                      || true
gh label create "$LABEL_SECURITY"      --color d73a4a --description "Security related"           || true
gh label create "$LABEL_RELIABILITY"   --color d93f0b --description "Reliability and stability"  || true
gh label create "$LABEL_ARCHITECTURE"  --color 0e8a16 --description "Architecture design"        || true
gh label create "$LABEL_LOGIC"         --color 5319e7 --description "Business logic"             || true
gh label create "$LABEL_KUBERNETES"    --color 326ce5 --description "Kubernetes related"         || true
gh label create "$LABEL_FEATURE"       --color a2eeef --description "New feature"                || true
gh label create "$LABEL_TESTING"       --color C2E0C6 --description "Tests"                      || true
gh label create "$LABEL_WAVE"          --color BFD4F2 --description "Stellar Wave contributor issue" || true
gh label create "$LABEL_GOOD_FIRST"    --color 7057ff --description "Good for newcomers"         || true

echo "Labels created."
