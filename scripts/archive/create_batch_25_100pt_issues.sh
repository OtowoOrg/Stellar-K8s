#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Medium difficulty batch (100 Points each).

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

EXPECTED_ISSUE_COUNT=12
ACTUAL_ISSUE_COUNT=$(grep -c '^gh issue create' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT." >&2
  exit 1
fi

echo "Creating Batch of 12 Medium (100-point) issues..."

gh issue create --repo "$REPO" \
  --title "Add advanced Liveness and Readiness probes to Stellar-Core nodes" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Improve the resilience of Stellar-Core nodes by implementing advanced health checks that query the core API rather than just checking if the process is running.

### ✅ Acceptance Criteria
- Create a lightweight sidecar or script for API health checks.
- Update the operator to inject these new liveness and readiness probes.
- Add tests to ensure nodes are correctly marked Unready during sync phases.
" --label "stellar-wave,feature,reliability"

gh issue create --repo "$REPO" \
  --title "Implement leader election for the operator" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Ensure high availability of the operator itself by implementing leader election, allowing multiple replicas of the operator to run simultaneously.

### ✅ Acceptance Criteria
- Use controller-runtime's leader election features.
- Update deployment manifests to support multiple replicas.
- Add e2e tests demonstrating operator failover.
" --label "stellar-wave,feature,architecture"

gh issue create --repo "$REPO" \
  --title "Build a Helm chart for Prometheus monitoring integration" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Provide a pre-packaged Helm chart that deploys ServiceMonitors, PodMonitors, and Grafana dashboards for Stellar-K8s.

### ✅ Acceptance Criteria
- Create Helm templates for Prometheus CRDs.
- Include default alert rules for Stellar node health.
- Ensure the chart passes \`helm lint\` and is tested in CI.
" --label "stellar-wave,feature,observability"

gh issue create --repo "$REPO" \
  --title "Add Pod Disruption Budgets (PDB) for core nodes" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Prevent accidental downtime during node drains or cluster upgrades by automatically generating Pod Disruption Budgets.

### ✅ Acceptance Criteria
- The operator should automatically create PDBs for Stellar node StatefulSets.
- Ensure quorum is maintained when calculating \`minAvailable\`.
- Add integration tests verifying PDB behavior.
" --label "stellar-wave,feature,reliability"

gh issue create --repo "$REPO" \
  --title "Create a comprehensive Grafana dashboard for Horizon metrics" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Build a detailed Grafana dashboard focusing on Horizon API performance, request rates, and database connection pools.

### ✅ Acceptance Criteria
- JSON dashboard definition included in the repo.
- Dashboard should track HTTP latency, errors, and DB query times.
- Document how to import and configure the dashboard.
" --label "stellar-wave,feature,observability"

gh issue create --repo "$REPO" \
  --title "Implement automated secret rotation for network passphrases" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Enhance security by allowing network passphrases and administrative keys to be rotated seamlessly without manual restarts.

### ✅ Acceptance Criteria
- Operator should detect changes to Secret objects.
- Trigger graceful restarts of affected nodes when keys change.
- Ensure no downtime during the rotation process.
" --label "stellar-wave,feature,security"

gh issue create --repo "$REPO" \
  --title "Add support for custom init containers in node deployments" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Allow users to specify custom init containers in the StellarNode CRD for tasks like fetching custom configuration or restoring state.

### ✅ Acceptance Criteria
- Extend the CRD with an \`initContainers\` field.
- Update the StatefulSet generation logic to append these containers.
- Write unit tests validating the injection.
" --label "stellar-wave,feature,architecture"

gh issue create --repo "$REPO" \
  --title "Build a CLI tool for operator snapshot management" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Create a sub-command for the \`kubectl-stellar\` plugin to easily trigger, list, and restore from volume snapshots.

### ✅ Acceptance Criteria
- Implement \`snapshot create\`, \`snapshot list\`, and \`snapshot restore\` commands.
- Interface directly with the Kubernetes VolumeSnapshot API.
- Document the commands in the README.
" --label "stellar-wave,feature,automation"

gh issue create --repo "$REPO" \
  --title "Implement configurable rate limiting in the ingress controller" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Add ingress annotations and configuration options to rate-limit traffic to public-facing Horizon nodes to prevent abuse.

### ✅ Acceptance Criteria
- Update the operator's ingress generation logic.
- Support NGINX ingress rate-limiting annotations.
- Add configuration fields to the CRD.
" --label "stellar-wave,feature,security"

gh issue create --repo "$REPO" \
  --title "Add extensive e2e tests for node recovery scenarios" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Increase test coverage by adding specific e2e tests that simulate node crashes, disk failures, and network partitions.

### ✅ Acceptance Criteria
- Use kind or minikube for local cluster setup.
- Simulate failures and verify the operator auto-recovers the nodes.
- Ensure tests pass reliably in CI.
" --label "stellar-wave,feature,reliability"

gh issue create --repo "$REPO" \
  --title "Implement strict NetworkPolicies for inter-node communication" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Improve cluster security by automatically generating NetworkPolicies that restrict traffic only to necessary ports (e.g., SCP port, DB port).

### ✅ Acceptance Criteria
- Operator should create default-deny policies.
- Add allow-rules only for specific stellar-core and horizon communication.
- Provide a way to disable this via CRD flag.
" --label "stellar-wave,feature,security"

gh issue create --repo "$REPO" \
  --title "Add admission webhook for resource limits validation" \
  --body "### 🟡 Difficulty: Medium (100 Points)

Prevent users from deploying nodes with insufficient CPU/Memory by validating resource requests in the admission webhook.

### ✅ Acceptance Criteria
- Update the validating webhook logic.
- Reject CRDs that don't meet minimum resource requirements for production mode.
- Add comprehensive unit tests for the validator.
" --label "stellar-wave,feature,architecture"

echo "✅ Created 12 medium (100-point) issues successfully!"
