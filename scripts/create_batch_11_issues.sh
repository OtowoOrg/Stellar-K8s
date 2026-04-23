#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

# Source shared retry/backoff and dry-run helper.
# shellcheck source=scripts/retry_helper.sh
source "$(dirname "$0")/retry_helper.sh"

echo "Creating Batch 11 (5 x 150 pts) issues with auto-retry..."

# Alias kept for readability; delegates to the shared helper.
create_issue_with_retry() { create_issue "$1" "$2" "$3"; }

# ─── ISSUE 1 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Enable Multi-Arch Container Support (ARM64/AMD64) for Operator and Plugins" \
  "stellar-wave,enhancement,performance,ci" \
  "### 🟡 Difficulty: Medium (150 Points)

Many Stellar node operators are moving to ARM64 (AWS Graviton, Google Tau T2A) for cost efficiency. Our current CI/CD and Dockerfile must be validated and optimized for multi-arch builds.

### ✅ Acceptance Criteria
- Update the \`Dockerfile\` to correctly handle cross-compilation for \`arm64\` and \`amd64\` targets.
- Modify the \`.github/workflows/ci.yml\` to build and push multi-arch manifests to GHCR.
- Verify that the \`distroless\` base image used is multi-arch compatible.
- Document how to pull and run the ARM64 version specifically.

### 📚 Resources
- [Docker Multi-arch builds](https://docs.docker.com/build/building/multi-platform/)
- [\`Dockerfile\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/Dockerfile)
"

# ─── ISSUE 2 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Proactive Alerting: PrometheusRule manifests for Stellar Node Health" \
  "stellar-wave,enhancement,observability,reliability" \
  "### 🟡 Difficulty: Medium (150 Points)

Metrics are only useful if they trigger alerts. We need a set of standard \`PrometheusRule\` manifests for the Prometheus Operator.

### ✅ Acceptance Criteria
- Create \`charts/stellar-operator/templates/prometheusrule.yaml\`.
- Implement alerts for:
  - \`StellarNodeSyncLag\`: Trigger if a node is > 100 ledgers behind the network.
  - \`StellarNodeMemoryPressure\`: Trigger if RSS is > 90% of limit.
  - \`StellarOperatorReconcileErrors\`: Trigger if the operator fails to reconcile for > 5 mins.
  - \`StellarHistoryArchiveUnresponsive\`: If history URLs return 404/500.
- Allow enabling/disabling these via Helm values.

### 📚 Resources
- [Prometheus Operator Alerting Rules](https://prometheus-operator.dev/docs/operator/design/#prometheusrule)
- [\`src/controller/metrics.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/metrics.rs)
"

# ─── ISSUE 3 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Stellar-Specific K8s Event Emission for Improved Auditing" \
  "stellar-wave,enhancement,observability,dx" \
  "### 🟡 Difficulty: Medium (150 Points)

Status fields are transient. We should emit permanent Kubernetes **Events** for critical node state changes so they appear in \`kubectl describe sn <name>\`.

### ✅ Acceptance Criteria
- Integrate the \`Recorder\` from \`kube-rs\` into the reconciler.
- Emit events for:
  - \`SuccessfulReconciliation\`: When resources are first created.
  - \`NodePromotedToPrimary\`: During a DR failover.
  - \`SyncLagDetected\`: When a node falls behind.
  - \`FinalizerCleanupStarted\`: When a node is being deleted.
- Verify events are visible via \`kubectl get events\`.

### 📚 Resources
- [kube-rs Event Recorder](https://docs.rs/kube/latest/kube/runtime/events/index.html)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)
"

# ─── ISSUE 4 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Zero-Trust Network Policies for Stellar Node Isolation" \
  "stellar-wave,enhancement,security,kubernetes" \
  "### 🟡 Difficulty: Medium (150 Points)

By default, Kubernetes allows all pod-to-pod communication. We need to implement strict \`NetworkPolicy\` manifests to isolate Stellar components.

### ✅ Acceptance Criteria
- Update \`src/controller/traffic.rs\` to generate default-deny policies for managed nodes.
- Explicitly allow:
  - Peer-to-peer traffic on port 11625 (only between validated peers).
  - HTTP traffic on port 11626 (only from specified internal CIDRs or the operator).
  - Metrics scraping from the Prometheus namespace.
- Ensure Horizon nodes can connect to their specific Core backends but not to other random pods.

### 📚 Resources
- [Kubernetes Network Policies](https://kubernetes.io/docs/concepts/services-networking/network-policies/)
- [\`src/controller/traffic.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/traffic.rs)
"

# ─── ISSUE 5 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "kubectl-stellar Plugin: Implement Debug and Logs commands" \
  "stellar-wave,enhancement,dx" \
  "### 🟡 Difficulty: Medium (150 Points)

The \`kubectl-stellar\` plugin is currently minimal. We need debugging commands that simplify troubleshooting for node operators.

### ✅ Acceptance Criteria
- Add a \`logs\` sub-command: \`kubectl stellar logs <node-name>\` (automatically finds the correct container and tails logs).
- Add a \`debug\` sub-command: \`kubectl stellar debug <node-name>\` (execs into a sidecar or temporary pod with diagnostic tools like \`curl\`, \`dig\`, and \`stellar-core\`).
- Add a \`status\` sub-command: Displays a pretty-printed table of the \`StellarNodeStatus\` conditions.
- Update \`src/kubectl_plugin.rs\` with these new commands.

### 📚 Resources
- [\`src/kubectl_plugin.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/kubectl_plugin.rs)
- [clap-rs for CLI parsing](https://docs.rs/clap/latest/clap/)
"

echo ""
echo "🎉 Batch 11 (5 x 150 pts) issues created successfully!"
