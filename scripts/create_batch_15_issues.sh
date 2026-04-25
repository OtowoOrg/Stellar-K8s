#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/repo.sh"
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 15, 10 x 200 pts).

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
ACTUAL_ISSUE_COUNT=$(grep -c '^create_issue_with_retry' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT. Update EXPECTED_ISSUE_COUNT or fix the script." >&2
  exit 1
fi

echo "Creating Batch 15 (10 x 200 pts) issues with auto-retry..."

# ─── ISSUE 1 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Structured JSON Logging Across All Operator Modules" \
  "stellar-wave,enhancement,observability" \
  "### 🔴 Difficulty: High (200 Points)

The operator currently uses a mix of plain-text and structured log output. For production-grade observability, all modules should emit structured JSON logs that integrate with ELK/Loki stacks.

### ✅ Acceptance Criteria
- Configure \`tracing-subscriber\` with the \`json\` feature across all modules.
- Ensure every \`info!\`, \`warn!\`, and \`error!\` macro call includes structured fields (e.g., \`node_name\`, \`namespace\`, \`reconcile_id\`).
- Add a \`--log-format\` CLI flag to toggle between \`json\` and \`pretty\` output.
- Write unit tests verifying JSON log output structure.

### 📚 Resources
- [tracing-subscriber JSON layer](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/format/struct.Json.html)
"

# ─── ISSUE 2 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Comprehensive Helm Chart Unit Tests Using helm-unittest" \
  "stellar-wave,testing,enhancement" \
  "### 🔴 Difficulty: High (200 Points)

Our Helm chart lacks automated unit tests. This makes it easy for regressions to slip through when updating templates.

### ✅ Acceptance Criteria
- Install and configure \`helm-unittest\` for the \`charts/stellar-operator/\` chart.
- Write tests for every template file covering key scenarios:
  - Default values rendering.
  - RBAC resources generated correctly.
  - ServiceAccount annotations applied.
  - Conditional template rendering (e.g., ingress enabled/disabled).
- Integrate into the CI pipeline.

### 📚 Resources
- [helm-unittest plugin](https://github.com/helm-unittest/helm-unittest)
"

# ─── ISSUE 3 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Health Check Endpoints: /healthz, /readyz, /livez" \
  "stellar-wave,enhancement,reliability" \
  "### 🔴 Difficulty: High (200 Points)

The operator binary needs standard Kubernetes health check endpoints so that its own Deployment can use proper liveness and readiness probes.

### ✅ Acceptance Criteria
- Add \`/healthz\`, \`/readyz\`, and \`/livez\` endpoints to the Axum REST API server.
- \`/readyz\` should check that the K8s client can reach the API server and the CRD is installed.
- \`/livez\` should verify the reconciler loop is not stuck (e.g., last successful reconcile was within the last 60s).
- Update the Helm chart Deployment to use these probes.

### 📚 Resources
- [Kubernetes Health Check Best Practices](https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/)
"

# ─── ISSUE 4 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Generate and Publish CRD API Reference Documentation" \
  "stellar-wave,documentation,dx" \
  "### 🔴 Difficulty: High (200 Points)

Users need a browsable API reference for the \`StellarNode\` CRD. We should auto-generate this from the Rust types.

### ✅ Acceptance Criteria
- Use \`crdgen\` or a custom script to extract the OpenAPI schema from the CRD.
- Generate a Markdown or HTML API reference document from the schema.
- Include descriptions for every field, default values, and validation constraints.
- Publish to the \`docs/\` directory and link from the README.

### 📚 Resources
- [kube-rs CRD generation](https://kube.rs/controllers/crd/)
"

# ─── ISSUE 5 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Configurable Resource Defaults via Helm Values" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

Currently, resource requests and limits for managed Stellar nodes are hardcoded in the CRD defaults. Operators should be able to override global defaults via Helm values.

### ✅ Acceptance Criteria
- Add \`defaultResources.validator\`, \`defaultResources.horizon\`, and \`defaultResources.sorobanRpc\` sections to \`values.yaml\`.
- The operator should read these from a mounted ConfigMap at startup.
- If a \`StellarNode\` spec doesn't specify resources, the operator applies the Helm-configured defaults.
- Document the precedence order: Spec > Helm defaults > hardcoded defaults.

### 📚 Resources
- [Helm Values Best Practices](https://helm.sh/docs/chart_best_practices/values/)
"

# ─── ISSUE 6 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Graceful Shutdown and Drain Handling for the Operator" \
  "stellar-wave,enhancement,reliability" \
  "### 🔴 Difficulty: High (200 Points)

When the operator pod is evicted or restarted, in-flight reconciliations can be interrupted. We need to handle SIGTERM gracefully.

### ✅ Acceptance Criteria
- Catch SIGTERM / SIGINT signals and initiate a graceful shutdown.
- Complete any in-flight reconciliation before exiting.
- Release the leader election lease cleanly on shutdown.
- Add integration tests that verify the drain behavior.

### 📚 Resources
- [Tokio Graceful Shutdown](https://tokio.rs/tokio/topics/shutdown)
"

# ─── ISSUE 7 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Owner Reference Labels to All Managed Child Resources" \
  "stellar-wave,enhancement,reliability" \
  "### 🔴 Difficulty: High (200 Points)

The operator creates multiple child resources (Deployments, Services, PVCs). All of them must have proper \`ownerReferences\` and consistent labels for garbage collection and querying.

### ✅ Acceptance Criteria
- Audit all resource builders in \`src/controller/resources.rs\` and ensure \`ownerReferences\` are set correctly pointing back to the \`StellarNode\` CR.
- Add a standard label set: \`app.kubernetes.io/name\`, \`app.kubernetes.io/instance\`, \`app.kubernetes.io/managed-by\`, \`app.kubernetes.io/component\`.
- Write unit tests verifying labels and owner references on generated resources.

### 📚 Resources
- [K8s Recommended Labels](https://kubernetes.io/docs/concepts/overview/working-with-objects/common-labels/)
"

# ─── ISSUE 8 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Create End-to-End Quickstart Tutorial for New Contributors" \
  "stellar-wave,documentation,dx" \
  "### 🔴 Difficulty: High (200 Points)

New contributors need a single, end-to-end tutorial that walks them from \`git clone\` to running the operator on a local Kind cluster.

### ✅ Acceptance Criteria
- Create a \`docs/quickstart.md\` guide.
- Cover: Prerequisites, cloning, building, creating a Kind cluster, installing the CRD, deploying the operator, and creating a sample \`StellarNode\`.
- Include a \`Makefile\` target (\`make quickstart\`) that automates these steps.
- Test the guide from scratch on a clean machine.

### 📚 Resources
- [Kind Quick Start](https://kind.sigs.k8s.io/docs/user/quick-start/)
"

# ─── ISSUE 9 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement ConfigMap-Based Feature Flags for the Operator" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

We need a runtime feature flag system so operators can toggle experimental features without redeploying.

### ✅ Acceptance Criteria
- Create a \`stellar-operator-config\` ConfigMap with feature flags (e.g., \`enable_cve_scanning\`, \`enable_read_pool\`, \`enable_dr\`).
- The operator should watch this ConfigMap and reload feature flags without restart.
- Log when a feature flag changes at runtime.
- Document all available feature flags in the README.

### 📚 Resources
- [kube-rs Watchers](https://docs.rs/kube/latest/kube/runtime/watcher/index.html)
"

# ─── ISSUE 10 (200 pts) ───────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Operator Version and Build Info to Prometheus Metrics" \
  "stellar-wave,enhancement,observability" \
  "### 🔴 Difficulty: High (200 Points)

Operators running in production need to know which version of the operator is deployed and whether it's the leader. This should be exposed as Prometheus metrics.

### ✅ Acceptance Criteria
- Add a \`stellar_operator_info\` gauge with labels: \`version\`, \`git_sha\`, \`rust_version\`.
- Add a \`stellar_operator_leader_status\` gauge (1 if leader, 0 otherwise).
- Expose a \`stellar_operator_uptime_seconds\` counter.
- Update the Grafana dashboard example (if it exists) to display these metrics.

### 📚 Resources
- [prometheus-client crate](https://docs.rs/prometheus-client/latest/prometheus_client/)
"

echo ""
echo "🎉 Batch 15 (10 x 200 pts) issues created successfully!"

print_skip_summary
