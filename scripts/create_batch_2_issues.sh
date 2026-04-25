#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
source "$(dirname "$0")/lib/repo.sh"

# Stellar-K8s Wave Issue Creation Script - BATCH 2
# Issues #12 - #21

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 2, issues #12-#21).

Prerequisites:
  - gh CLI installed and authenticated (gh auth login)
  - Network access to api.github.com

Optional environment variables:
  REPO                Target repository (default: OtowoOrg/Stellar-K8s)
  DRY_RUN             Set to 1 to print commands without executing
  MAX_RETRIES         Number of retry attempts on API failure (default: 10)
  RETRY_DELAY_SECONDS Seconds to wait between retries (default: 15)

Example:
  REPO=myorg/my-fork DRY_RUN=1 $(basename "$0")
EOF
}

for arg in "$@"; do
  case "$arg" in
    -h|--help) show_help; exit 0 ;;
  esac
done

EXPECTED_ISSUE_COUNT=10
ACTUAL_ISSUE_COUNT=$(grep -c '^gh issue create' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT. Update EXPECTED_ISSUE_COUNT or fix the script." >&2
  exit 1
fi

echo "Creating Batch 2 of Stellar Wave issues..."

# 12. Add Resource Limit validation (Trivial - 100 Points)
gh issue create --repo "$REPO" \
  --title "Add Resource Limit validation (CPU/Memory)" \
  --body "### 🟢 Difficulty: Trivial (100 Points)

Currently, the operator allows setting CPU/Memory requests and limits without validating them. We need to ensure that \`requests <= limits\` to prevent Kubernetes scheduling errors.

### ✅ Acceptance Criteria
- Update \`src/crd/stellar_node.rs\` validation logic.
- Reject specs where requested resources exceed limits.
- Add unit tests for this validation.

### 📚 Resources
- [Kubernetes Resource Management](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/)
- [Stellar-K8s Validation Example](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/stellar_node.rs)
" --label "stellar-wave,good-first-issue,kubernetes"

# 13. Implement validate() for NodePort range (Trivial - 100 Points)
gh issue create --repo "$REPO" \
  --title "Implement validation for custom NodePort range" \
  --body "### 🟢 Difficulty: Trivial (100 Points)

When a user specifies a NodePort in the service config, we should validate that it falls within the standard Kubernetes range (30000-32767) unless otherwise configured, to provide early feedback.

### ✅ Acceptance Criteria
- Add validation in \`StellarNodeSpec::validate()\` for NodePort fields.
- Throw a meaningful error if the port is out of range.

### 📚 Resources
- [Kubernetes Service NodePort](https://kubernetes.io/docs/concepts/services-networking/service/#type-nodeport)
- [\`src/crd/stellar_node.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/stellar_node.rs)
" --label "stellar-wave,good-first-issue,kubernetes"

# 14. Add topologySpreadConstraints support (Trivial - 100 Points)
gh issue create --repo "$REPO" \
  --title "Add topologySpreadConstraints support to Pod template" \
  --body "### 🟢 Difficulty: Trivial (100 Points)

To ensure high availability, users should be able to specify \`topologySpreadConstraints\` to spread Stellar pods across different Availability Zones (AZs) or nodes.

### ✅ Acceptance Criteria
- Add \`topologySpreadConstraints\` field to the Pod template in \`StellarNodeSpec\`.
- Propagate this field to the generated Deployment/StatefulSet in \`resources.rs\`.

### 📚 Resources
- [Kubernetes Pod Topology Spread Constraints](https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)
" --label "stellar-wave,kubernetes,feature"

# 15. Implement standard Kubernetes Conditions in Status (Medium - 150 Points)
gh issue create --repo "$REPO" \
  --title "Implement standard Kubernetes Conditions in Status" \
  --body "### 🟡 Difficulty: Medium (150 Points)

Instead of a single \`Phase\` string, the operator should use the standard Kubernetes \`Conditions\` pattern (e.g., Ready, Progressing, Degraded) to provide more granular status information.

### ✅ Acceptance Criteria
- Update \`StellarNodeStatus\` to include a \`conditions\` vector.
- Implement a helper to update conditions (TransitionTime, Status, Reason, Message).
- Update the reconciler to report 'Ready' condition when all sub-resources are healthy.

### 📚 Resources
- [Kubernetes API Conventions: Conditions](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md#typical-status-properties)
- [kube-rs Conditions Guide](https://kube.rs/controllers/conditions/)
" --label "stellar-wave,architecture,logic"

# 16. Add support for Sidecar containers (Medium - 150 Points)
gh issue create --repo "$REPO" \
  --title "Add support for Sidecar containers in StellarNode" \
  --body "### 🟡 Difficulty: Medium (150 Points)

Users may need to run sidecar containers (like log forwarders, monitoring agents, or proxies) alongside the main Stellar container.

### ✅ Acceptance Criteria
- Add \`sidecars: Option<Vec<Container>>\` to \`StellarNodeSpec\`.
- Merge these containers into the generated Pod spec in \`resources.rs\`.
- Ensure volumes can be shared between the main container and sidecars.

### 📚 Resources
- [Kubernetes Sidecar Containers](https://kubernetes.io/docs/concepts/workloads/pods/sidecar-containers/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)
" --label "stellar-wave,kubernetes,feature"

# 17. Implement 'Maintenance Mode' flag (Medium - 150 Points)
gh issue create --repo "$REPO" \
  --title "Implement 'Maintenance Mode' flag" \
  --body "### 🟡 Difficulty: Medium (150 Points)

When performing manual operations on a node, it’s useful to have a 'Maintenance Mode' that keeps the Service and PVC but scales the workload temporarily or labels it to prevent the operator from fighting manual changes.

### ✅ Acceptance Criteria
- Add \`maintenanceMode: bool\` to \`StellarNodeSpec\`.
- When active, the reconciler should skip 'Apply' steps for the workload but keep status reporting active.

### 📚 Resources
- [Kubernetes Operator Lifecycle](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)
" --label "stellar-wave,logic,feature"

# 18. Add Prometheus Rule generation (Medium - 150 Points)
gh issue create --repo "$REPO" \
  --title "Add Prometheus Rule generation for Alerting" \
  --body "### 🟡 Difficulty: Medium (150 Points)

The operator should optionally generate a \`PrometheusRule\` custom resource (if Prometheus Operator is present) to alert on node crashes or sync issues.

### ✅ Acceptance Criteria
- Add \`alerting: bool\` to \`StellarNodeSpec\`.
- If enabled, create a \`ConfigMap\` or \`PrometheusRule\` containing standard alerts (NodeDown, HighMemory, etc.).

### 📚 Resources
- [Prometheus Operator: Monitoring Mixins](https://github.com/prometheus-operator/kube-prometheus/tree/main/jsonnet/kube-prometheus/rules)
- [\`src/controller/metrics.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/metrics.rs)
" --label "stellar-wave,observability,feature"

# 19. Implement 'Auto-Sync Health' check for Horizon (High - 200 Points)
gh issue create --repo "$REPO" \
  --title "Implement 'Auto-Sync Health' check for Horizon" \
  --body "### 🔴 Difficulty: High (200 Points)

Horizon nodes can take time to ingest and catch up. The operator should query the Horizon \`/health\` or \`/metrics\` endpoint to verify it is fully 'caught up' before marking the node as \`Ready\`.

### ✅ Acceptance Criteria
- Add an HTTP client to the reconciler (e.g., \`reqwest\`).
- Query the pod's local IP on the health port.
- Block the transition to \`Ready\` status until the node reports it is synced.

### 📚 Resources
- [Horizon API Reference](https://developers.stellar.org/docs/data-availability/horizon/api-reference)
- [kube-rs Health Checks](https://kube.rs/)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)
" --label "stellar-wave,reliability,rust"

# 20. Support for External Postgres Databases (High - 200 Points)
gh issue create --repo "$REPO" \
  --title "Support for External Postgres Databases" \
  --body "### 🔴 Difficulty: High (200 Points)

For production, users often prefer managed databases (RDS, Cloud SQL, CockroachDB). The operator should allow passing external DB connection strings via Secrets.

### ✅ Acceptance Criteria
- Add \`database: ExternalDatabaseConfig\` to \`StellarNodeSpec\`.
- Support fetching credentials from an existing Secret (\`secretKeyRef\`).
- Inject these as environment variables into the Stellar/Horizon containers.

### 📚 Resources
- [Stellar Core Database Config](https://github.com/stellar/stellar-core/blob/master/docs/software/admin.md#database)
- [Kubernetes Secrets](https://kubernetes.io/docs/concepts/configuration/secret/)
- [\`src/crd/stellar_node.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/stellar_node.rs)
" --label "stellar-wave,architecture,feature"

# 21. Implement Automated Database Migrations for Horizon (High - 200 Points)
gh issue create --repo "$REPO" \
  --title "Implement Automated Database Migrations for Horizon" \
  --body "### 🔴 Difficulty: High (200 Points)

When upgrading Horizon, the database schema often needs a migration. The operator should automatically run an InitContainer or Job to perform \`horizon db init\` or \`horizon db upgrade\` before starting the main process.

### ✅ Acceptance Criteria
- Add logic to detect version changes.
- Launch a one-time \`Job\` or \`InitContainer\` to run migration commands.
- Block the main container startup until the migration success is confirmed.

### 📚 Resources
- [Horizon DB Management](https://developers.stellar.org/docs/data-availability/horizon/admin#database-management)
- [Kubernetes Init Containers](https://kubernetes.io/docs/concepts/workloads/pods/init-containers/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)
" --label "stellar-wave,reliability,automation"

echo "Done! Batch 2 issues created (12-21)."
