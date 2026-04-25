#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/repo.sh"
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 17, 30 x 200 pts).

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

EXPECTED_ISSUE_COUNT=30
ACTUAL_ISSUE_COUNT=$(grep -c '^create_issue_with_retry' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT. Update EXPECTED_ISSUE_COUNT or fix the script." >&2
  exit 1
fi

echo "Creating Batch 17 (30 x 200 pts) issues with auto-retry..."

# ─── 1 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add Shell Completion for stellar-operator CLI" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Generate and package shell completion scripts (Bash, Zsh, Fish) for the operator CLI using \`clap_complete\`.

### ✅ Acceptance Criteria
- Integrate \`clap_complete\` into the build process.
- Add a command to generate completion scripts.
- Document how to install them in the README.

### 📚 Resources
- [clap_complete documentation](https://docs.rs/clap_complete/latest/clap_complete/)
- [Clap Shell Completion Example](https://github.com/clap-rs/clap/tree/master/clap_complete/examples)"


# ─── 2 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar version' command in kubectl-stellar plugin" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

The plugin should be able to report its own version and the version of the operator running in the cluster.

### ✅ Acceptance Criteria
- Add \`version\` subcommand to the plugin.
- Fetch operator version from the deployment or a well-known metric.

### 📚 Resources
- [\`src/kubectl_plugin.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/kubectl_plugin.rs)
- [Kubernetes API: Deployments](https://kubernetes.io/docs/reference/kubernetes-api/workload-resources/deployment-v1/)"


# ─── 3 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add unit tests for 'ensure_pvc' logic" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure that PVC creation and updates are handled correctly across different storage classes.

### ✅ Acceptance Criteria
- Add unit tests in \`src/controller/resources.rs\`.

### 📚 Resources
- [Kubernetes PersistentVolumeClaims](https://kubernetes.io/docs/concepts/storage/persistent-volumes/#persistentvolumeclaims)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)"


# ─── 4 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document mTLS setup in a dedicated guide" "stellar-wave,documentation,security" "### 🔴 Difficulty: High (200 Points)

Create a comprehensive guide on how to configure and rotate mTLS certificates for the operator.

### ✅ Acceptance Criteria
- New file \`docs/mtls-guide.md\`.

### 📚 Resources
- [\`src/controller/mtls.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/mtls.rs)
- [Cert-Manager mTLS Guide](https://cert-manager.io/docs/usage/certificate/)"


# ─── 5 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'reconcile_duration_seconds' histogram metric" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Track how long each reconciliation takes to identify bottlenecks.

### ✅ Acceptance Criteria
- Add the histogram to the metrics module.
- Observe durations in the main reconciler loop.

### 📚 Resources
- [Prometheus Histograms](https://prometheus.io/docs/concepts/metric_types/#histogram)
- [\`src/controller/metrics.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/metrics.rs)"


# ─── 6 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar status' summary in kubectl plugin" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

A high-level summary of all managed StellarNodes and their health.

### ✅ Acceptance Criteria
- Add \`status\` command to \`kubectl-stellar\`.

### 📚 Resources
- [\`src/kubectl_plugin.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/kubectl_plugin.rs)
- [Kubernetes Custom Resource Status](https://kubernetes.io/docs/tasks/extend-kubernetes/custom-resources/custom-resource-definitions/#status-subresource)"


# ─── 7 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add linting for shell scripts in CI" "stellar-wave,enhancement,ci" "### 🔴 Difficulty: High (200 Points)

Ensure all helper scripts are following best practices.

### ✅ Acceptance Criteria
- Integrate \`shellcheck\` into GitHub Actions.

### 📚 Resources
- [ShellCheck Home Page](https://www.shellcheck.net/)
- [\`.github/workflows/ci.yml\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/ci.yml)"


# ─── 8 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Improve error reporting for VSL fetch failures" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Provide more context when a Validator Selection List cannot be retrieved.

### ✅ Acceptance Criteria
- Include URL and status code in the error message.

### 📚 Resources
- [Stellar Validator Selection Guide](https://developers.stellar.org/docs/run-core-node/quorums#validator-selection)
- [\`src/error.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/error.rs)"


# ─── 9 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'stellar-network' label to all child resources" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Easier filtering of resources by the Stellar network they belong to.

### ✅ Acceptance Criteria
- Update resource builders to inject the label.

### 📚 Resources
- [Recommended Kubernetes Labels](https://kubernetes.io/docs/concepts/overview/working-with-objects/common-labels/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)"


# ─── 10 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document disaster recovery failover steps" "stellar-wave,documentation,reliability" "### 🔴 Difficulty: High (200 Points)

A step-by-step manual for performing a failover between regions.

### ✅ Acceptance Criteria
- New file \`docs/dr-failover.md\`.

### 📚 Resources
- [Stellar Disaster Recovery Best Practices](https://developers.stellar.org/docs/run-core-node/disaster-recovery)
- [\`src/controller/dr.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/dr.rs)"


# ─── 11 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add unit tests for ConfigMap generation" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Verify that the generated Stellar Core config is valid TOML.

### ✅ Acceptance Criteria
- Tests in \`src/controller/resources.rs\`.

### 📚 Resources
- [Stellar Core Configuration Guide](https://developers.stellar.org/docs/run-core-node/configuration)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)"


# ─── 12 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar logs' command in CLI" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Shortcut to tail logs from the operator pod.

### ✅ Acceptance Criteria
- Add \`logs\` subcommand to the operator binary.

### 📚 Resources
- [Kubernetes Pod Logs API](https://kubernetes.io/docs/reference/kubernetes-api/workload-resources/pod-v1/#read-log-of-the-specified-pod)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 13 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'resource_version' to reconciliation logs" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Helpful for debugging stale resource issues.

### ✅ Acceptance Criteria
- Log the resource version at the start of each reconcile.

### 📚 Resources
- [Kubernetes Resource Versions](https://kubernetes.io/docs/reference/using-api/api-concepts/#resource-versions)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)"


# ─── 14 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement basic caching for VSL responses" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Reduce external network requests by caching VSLs for a short duration.

### ✅ Acceptance Criteria
- Use an in-memory cache for VSL data.

### 📚 Resources
- [Stellar Validator Selection List (VSL)](https://developers.stellar.org/docs/run-core-node/quorums#validator-selection)
- [\`src/controller/peer_discovery.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/peer_discovery.rs)"


# ─── 15 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'stellar-wave' label to all new issues automatically" "stellar-wave,enhancement,ci" "### 🔴 Difficulty: High (200 Points)

A GitHub Action to label new issues with 'stellar-wave'.

### ✅ Acceptance Criteria
- Workflow file in \`.github/workflows/labeler.yml\`.

### 📚 Resources
- [GitHub Actions: Labeler](https://github.com/actions/labeler)
- [\`.github/workflows/\`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/.github/workflows)"


# ─── 16 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document resource limits for different node types" "stellar-wave,documentation,performance" "### 🔴 Difficulty: High (200 Points)

Recommended CPU/RAM for Validator vs Horizon vs Soroban RPC.

### ✅ Acceptance Criteria
- New section in \`docs/performance.md\`.

### 📚 Resources
- [Stellar Core Hardware Requirements](https://developers.stellar.org/docs/run-core-node/prerequisites#hardware-requirements)
- [\`docs/performance.md\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/docs/performance.md)"


# ─── 17 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add unit tests for the error transformation logic" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure Kube errors are correctly mapped to our internal Error type.

### ✅ Acceptance Criteria
- Tests in \`src/error.rs\`.

### 📚 Resources
- [Rust Error Handling](https://doc.rust-lang.org/book/ch09-00-error-handling.html)
- [\`src/error.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/error.rs)"


# ─── 18 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar events' command in kubectl plugin" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Filtered event stream for StellarNode resources.

### ✅ Acceptance Criteria
- Add \`events\` subcommand.

### 📚 Resources
- [Kubernetes Events API](https://kubernetes.io/docs/reference/kubernetes-api/cluster-resources/event-v1/)
- [\`src/kubectl_plugin.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/kubectl_plugin.rs)"


# ─── 19 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add validation for 'StellarNetwork' custom names" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure custom network strings aren't empty or malicious.

### ✅ Acceptance Criteria
- Validation logic in the CRD module.

### 📚 Resources
- [Kubernetes CRD Validation](https://kubernetes.io/docs/tasks/extend-kubernetes/custom-resources/custom-resource-definitions/#validation)
- [\`src/crd/types.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/types.rs)"


# ─── 20 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document how to run a local dev cluster with k3d" "stellar-wave,documentation,dx" "### 🔴 Difficulty: High (200 Points)

Alternative to Kind for local development.

### ✅ Acceptance Criteria
- Guide in \`docs/development.md\`.

### 📚 Resources
- [k3d Home Page](https://k3d.io/)
- [\`DEVELOPMENT.md\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/DEVELOPMENT.md)"


# ─── 21 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'stellar_operator_reconcile_errors_total' counter" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Track total reconciliation failures.

### ✅ Acceptance Criteria
- Add counter to metrics module.

### 📚 Resources
- [Prometheus Counters](https://prometheus.io/docs/concepts/metric_types/#counter)
- [\`src/controller/metrics.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/metrics.rs)"


# ─── 22 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar info' summary for sub-resources" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Display which Deployment/Service a StellarNode owns.

### ✅ Acceptance Criteria
- Add detailed info to \`stellar info\` output.

### 📚 Resources
- [Kubernetes Owner References](https://kubernetes.io/docs/concepts/overview/working-with-objects/owners-dependents/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 23 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add unit tests for finalizer removal" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure resources can be deleted cleanly.

### ✅ Acceptance Criteria
- Tests in \`src/controller/finalizers.rs\`.

### 📚 Resources
- [Kubernetes Finalizers](https://kubernetes.io/docs/concepts/overview/working-with-objects/finalizers/)
- [\`src/controller/finalizers.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/finalizers.rs)"


# ─── 24 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document Soroban RPC configuration options" "stellar-wave,documentation,dx" "### 🔴 Difficulty: High (200 Points)

Detail the Soroban-specific fields in the CRD.

### ✅ Acceptance Criteria
- New guide \`docs/soroban-rpc.md\`.

### 📚 Resources
- [Soroban RPC Documentation](https://developers.stellar.org/docs/smart-contracts/getting-started/soroban-rpc)
- [\`docs/soroban-rpc.md\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/docs/soroban-rpc.md)"


# ─── 25 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'stellar_node_up' gauge metric" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Simple binary metric for node health.

### ✅ Acceptance Criteria
- Metric updated based on pod readiness.

### 📚 Resources
- [Prometheus Gauges](https://prometheus.io/docs/concepts/metric_types/#gauge)
- [\`src/controller/metrics.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/metrics.rs)"


# ─── 26 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar version' check in CI" "stellar-wave,enhancement,ci" "### 🔴 Difficulty: High (200 Points)

Ensure the version in Cargo.toml matches the expected release version.

### ✅ Acceptance Criteria
- CI step to validate version consistency.

### 📚 Resources
- [Cargo.toml Versioning](https://doc.rust-lang.org/cargo/reference/manifest.html#the-version-field)
- [\`.github/workflows/ci.yml\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/ci.yml)"


# ─── 27 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add unit tests for mTLS configuration building" "stellar-wave,testing,security" "### 🔴 Difficulty: High (200 Points)

Verify cert data is loaded correctly.

### ✅ Acceptance Criteria
- Tests in \`src/controller/mtls.rs\`.

### 📚 Resources
- [\`src/controller/mtls.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/mtls.rs)
- [Rust \`rcgen\` Crate](https://docs.rs/rcgen/latest/rcgen/)"


# ─── 28 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document benchmarking results for operator scale" "stellar-wave,documentation,performance" "### 🔴 Difficulty: High (200 Points)

How many nodes can one operator instance handle?

### ✅ Acceptance Criteria
- Benchmarking report in \`docs/scalability.md\`.

### 📚 Resources
- [Kubernetes Scalability Thresholds](https://kubernetes.io/docs/concepts/cluster-administration/scaling/)
- [\`docs/scalability.md\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/docs/scalability.md)"


# ─── 29 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'stellar_node_sync_status' gauge" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Track whether a node is in 'Syncing' phase via Prometheus.

### ✅ Acceptance Criteria
- Metric reflecting the node phase.

### 📚 Resources
- [\`src/crd/types.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/types.rs)
- [Prometheus Metrics for Kubernetes](https://github.com/kubernetes/metrics)"


# ─── 30 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar check-crd' helper command" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Verify CRD installation and version.

### ✅ Acceptance Criteria
- Subcommand for the operator binary.

### 📚 Resources
- [Kubernetes CRD Versioning](https://kubernetes.io/docs/tasks/extend-kubernetes/custom-resources/custom-resource-definition-versioning/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


echo ""
echo "🎉 Batch 17 (30 x 200 pts) issues created successfully!"

print_skip_summary
