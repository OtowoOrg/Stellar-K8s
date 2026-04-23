#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

# Source shared retry/backoff and dry-run helper.
# shellcheck source=scripts/retry_helper.sh
source "$(dirname "$0")/retry_helper.sh"

echo "Creating Batch 17 (30 x 200 pts) issues with auto-retry..."

# Alias kept for readability; delegates to the shared helper.
create_issue_with_retry() { create_issue "$1" "$2" "$3"; }

# ─── 1 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add Shell Completion for stellar-operator CLI" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Generate and package shell completion scripts (Bash, Zsh, Fish) for the operator CLI using \`clap_complete\`.

### ✅ Acceptance Criteria
- Integrate \`clap_complete\` into the build process.
- Add a command to generate completion scripts.
- Document how to install them in the README."

# ─── 2 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar version' command in kubectl-stellar plugin" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

The plugin should be able to report its own version and the version of the operator running in the cluster.

### ✅ Acceptance Criteria
- Add \`version\` subcommand to the plugin.
- Fetch operator version from the deployment or a well-known metric."

# ─── 3 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add unit tests for 'ensure_pvc' logic" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure that PVC creation and updates are handled correctly across different storage classes.

### ✅ Acceptance Criteria
- Add unit tests in \`src/controller/resources.rs\`."

# ─── 4 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document mTLS setup in a dedicated guide" "stellar-wave,documentation,security" "### 🔴 Difficulty: High (200 Points)

Create a comprehensive guide on how to configure and rotate mTLS certificates for the operator.

### ✅ Acceptance Criteria
- New file \`docs/mtls-guide.md\`."

# ─── 5 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'reconcile_duration_seconds' histogram metric" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Track how long each reconciliation takes to identify bottlenecks.

### ✅ Acceptance Criteria
- Add the histogram to the metrics module.
- Observe durations in the main reconciler loop."

# ─── 6 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar status' summary in kubectl plugin" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

A high-level summary of all managed StellarNodes and their health.

### ✅ Acceptance Criteria
- Add \`status\` command to \`kubectl-stellar\`."

# ─── 7 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add linting for shell scripts in CI" "stellar-wave,enhancement,ci" "### 🔴 Difficulty: High (200 Points)

Ensure all helper scripts are following best practices.

### ✅ Acceptance Criteria
- Integrate \`shellcheck\` into GitHub Actions."

# ─── 8 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Improve error reporting for VSL fetch failures" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Provide more context when a Validator Selection List cannot be retrieved.

### ✅ Acceptance Criteria
- Include URL and status code in the error message."

# ─── 9 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'stellar-network' label to all child resources" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Easier filtering of resources by the Stellar network they belong to.

### ✅ Acceptance Criteria
- Update resource builders to inject the label."

# ─── 10 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document disaster recovery failover steps" "stellar-wave,documentation,reliability" "### 🔴 Difficulty: High (200 Points)

A step-by-step manual for performing a failover between regions.

### ✅ Acceptance Criteria
- New file \`docs/dr-failover.md\`."

# ─── 11 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add unit tests for ConfigMap generation" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Verify that the generated Stellar Core config is valid TOML.

### ✅ Acceptance Criteria
- Tests in \`src/controller/resources.rs\`."

# ─── 12 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar logs' command in CLI" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Shortcut to tail logs from the operator pod.

### ✅ Acceptance Criteria
- Add \`logs\` subcommand to the operator binary."

# ─── 13 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'resource_version' to reconciliation logs" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Helpful for debugging stale resource issues.

### ✅ Acceptance Criteria
- Log the resource version at the start of each reconcile."

# ─── 14 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement basic caching for VSL responses" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Reduce external network requests by caching VSLs for a short duration.

### ✅ Acceptance Criteria
- Use an in-memory cache for VSL data."

# ─── 15 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'stellar-wave' label to all new issues automatically" "stellar-wave,enhancement,ci" "### 🔴 Difficulty: High (200 Points)

A GitHub Action to label new issues with 'stellar-wave'.

### ✅ Acceptance Criteria
- Workflow file in \`.github/workflows/labeler.yml\`."

# ─── 16 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document resource limits for different node types" "stellar-wave,documentation,performance" "### 🔴 Difficulty: High (200 Points)

Recommended CPU/RAM for Validator vs Horizon vs Soroban RPC.

### ✅ Acceptance Criteria
- New section in \`docs/performance.md\`."

# ─── 17 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add unit tests for the error transformation logic" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure Kube errors are correctly mapped to our internal Error type.

### ✅ Acceptance Criteria
- Tests in \`src/error.rs\`."

# ─── 18 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar events' command in kubectl plugin" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Filtered event stream for StellarNode resources.

### ✅ Acceptance Criteria
- Add \`events\` subcommand."

# ─── 19 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add validation for 'StellarNetwork' custom names" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure custom network strings aren't empty or malicious.

### ✅ Acceptance Criteria
- Validation logic in the CRD module."

# ─── 20 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document how to run a local dev cluster with k3d" "stellar-wave,documentation,dx" "### 🔴 Difficulty: High (200 Points)

Alternative to Kind for local development.

### ✅ Acceptance Criteria
- Guide in \`docs/development.md\`."

# ─── 21 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'stellar_operator_reconcile_errors_total' counter" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Track total reconciliation failures.

### ✅ Acceptance Criteria
- Add counter to metrics module."

# ─── 22 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar info' summary for sub-resources" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Display which Deployment/Service a StellarNode owns.

### ✅ Acceptance Criteria
- Add detailed info to \`stellar info\` output."

# ─── 23 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add unit tests for finalizer removal" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure resources can be deleted cleanly.

### ✅ Acceptance Criteria
- Tests in \`src/controller/finalizers.rs\`."

# ─── 24 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document Soroban RPC configuration options" "stellar-wave,documentation,dx" "### 🔴 Difficulty: High (200 Points)

Detail the Soroban-specific fields in the CRD.

### ✅ Acceptance Criteria
- New guide \`docs/soroban-rpc.md\`."

# ─── 25 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'stellar_node_up' gauge metric" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Simple binary metric for node health.

### ✅ Acceptance Criteria
- Metric updated based on pod readiness."

# ─── 26 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar version' check in CI" "stellar-wave,enhancement,ci" "### 🔴 Difficulty: High (200 Points)

Ensure the version in Cargo.toml matches the expected release version.

### ✅ Acceptance Criteria
- CI step to validate version consistency."

# ─── 27 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add unit tests for mTLS configuration building" "stellar-wave,testing,security" "### 🔴 Difficulty: High (200 Points)

Verify cert data is loaded correctly.

### ✅ Acceptance Criteria
- Tests in \`src/controller/mtls.rs\`."

# ─── 28 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Document benchmarking results for operator scale" "stellar-wave,documentation,performance" "### 🔴 Difficulty: High (200 Points)

How many nodes can one operator instance handle?

### ✅ Acceptance Criteria
- Benchmarking report in \`docs/scalability.md\`."

# ─── 29 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add 'stellar_node_sync_status' gauge" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Track whether a node is in 'Syncing' phase via Prometheus.

### ✅ Acceptance Criteria
- Metric reflecting the node phase."

# ─── 30 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar check-crd' helper command" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Verify CRD installation and version.

### ✅ Acceptance Criteria
- Subcommand for the operator binary."

echo ""
echo "🎉 Batch 17 (30 x 200 pts) issues created successfully!"
