#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Hard difficulty batch (200 Points each).

Prerequisites:
  - gh CLI installed and authenticated
  - Network access to api.github.com

Optional environment variables:
  REPO                Target repository (default: OtowoOrg/Stellar-K8s)
  DRY_RUN             Set to 1 to print commands without executing

Example:
  REPO=myorg/my-fork DRY_RUN=1 $(basename "$0")
EOF
}

for arg in "$@"; do
  case "$arg" in
    -h|--help) show_help; exit 0 ;;
  esac
done

EXPECTED_ISSUE_COUNT=8
ACTUAL_ISSUE_COUNT=$(grep -c '^gh issue create' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT." >&2
  exit 1
fi

echo "Creating Batch of 8 Hard (200-point) issues..."

gh issue create --repo "$REPO" \
  --title "Implement automated cross-region state synchronization" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Build a mechanism to continuously sync Stellar core state across multiple geographical regions to enable zero-RPO disaster recovery.

### ✅ Acceptance Criteria
- Implement a sidecar that streams captive core ledger state.
- Create cross-cluster network bridges.
- Prove state consistency under high load.
- Document the failover procedure.
" --label "stellar-wave,feature,architecture"

gh issue create --repo "$REPO" \
  --title "Build intelligent pod scheduling based on network latency" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Optimize SCP consensus times by ensuring nodes that communicate frequently are scheduled on nodes with the lowest possible network latency.

### ✅ Acceptance Criteria
- Create a custom Kubernetes scheduler plugin.
- Dynamically monitor inter-node latency.
- Evict and reschedule pods automatically to optimize consensus.
- Provide benchmarks showing latency reduction.
" --label "stellar-wave,feature,performance"

gh issue create --repo "$REPO" \
  --title "Create a GitOps pipeline for zero-touch network upgrades" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Automate the process of upgrading the entire Stellar network (protocol upgrades) using a GitOps workflow with ArgoCD or Flux.

### ✅ Acceptance Criteria
- Implement an operator controller that understands protocol version timelines.
- Integrate with GitOps tools to deploy new configurations synchronously.
- Add rollback mechanisms in case an upgrade fails consensus.
" --label "stellar-wave,feature,automation"

gh issue create --repo "$REPO" \
  --title "Implement dynamic volume resizing for historical data archives" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Stellar history archives grow indefinitely. Implement a system that dynamically monitors disk usage and resizes PVCs automatically without manual intervention.

### ✅ Acceptance Criteria
- Monitor disk usage metrics via Prometheus.
- Implement an automatic PVC expansion controller.
- Ensure the underlying storage class supports expansion.
- Handle edge cases where storage quotas are exceeded.
" --label "stellar-wave,feature,reliability"

gh issue create --repo "$REPO" \
  --title "Build a custom Kubernetes metrics server for Stellar-specific scaling" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Allow the Horizontal Pod Autoscaler (HPA) to scale Horizon instances based on Stellar-specific metrics (e.g. transactions per second) rather than just CPU/Memory.

### ✅ Acceptance Criteria
- Create a custom metrics API server implementation.
- Expose TPS and queue lengths to the HPA.
- Add complete e2e tests demonstrating dynamic scaling under simulated load.
" --label "stellar-wave,feature,performance"

gh issue create --repo "$REPO" \
  --title "Implement a generic webhook framework for custom validations" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Provide a flexible framework that allows cluster administrators to write custom Rego (OPA) or CEL policies to validate StellarNode CRDs before admission.

### ✅ Acceptance Criteria
- Integrate CEL validation rules directly into the CRD schema.
- Provide a webhook endpoint that delegates to OPA/Gatekeeper.
- Provide a library of default security policies.
" --label "stellar-wave,feature,security"

gh issue create --repo "$REPO" \
  --title "Create an automated chaos engineering test suite" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Build a continuous chaos testing suite using LitmusChaos or Chaos Mesh to regularly test the resilience of the operator and the deployed Stellar network.

### ✅ Acceptance Criteria
- Define chaos experiments (network partition, pod kill, disk fill).
- Integrate the chaos suite into a nightly GitHub Action.
- Generate automated reports on system resilience.
" --label "stellar-wave,feature,reliability"

gh issue create --repo "$REPO" \
  --title "Implement zero-downtime database migrations for Horizon" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Upgrading Horizon often requires complex database migrations. Implement a controller that orchestrates these migrations with zero downtime.

### ✅ Acceptance Criteria
- Implement blue/green deployment strategy for Horizon pods.
- Orchestrate the schema migration using temporary jobs.
- Handle rollback automatically if the migration fails.
- Provide metrics on migration duration and success rate.
" --label "stellar-wave,feature,architecture"

echo "✅ Created 8 hard (200-point) issues successfully!"
