#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/repo.sh"
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 18, 24 x 200 pts).

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

EXPECTED_ISSUE_COUNT=24
ACTUAL_ISSUE_COUNT=$(grep -c '^create_issue_with_retry' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT. Update EXPECTED_ISSUE_COUNT or fix the script." >&2
  exit 1
fi

echo "Creating Batch 18 (24 x 200 pts) issues with auto-retry..."

# ─── 1 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement Advanced Thread Tuning for Stellar Core Workloads" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Optimize the threading model for Stellar Core pods to ensure maximum performance during high-traffic periods without over-provisioning Kubernetes nodes.

### 📋 Context
Stellar Core uses several worker threads for ledger close, SCP processing, and database interactions. In a containerized environment, mismatches between CPU limits and thread counts can lead to significant latency spikes.

### ✅ Acceptance Criteria
- Implement a \`threading\` configuration block in the \`StellarNode\` CRD.
- Support tuning for: \`workerThreads\`, \`dbThreads\`, and \`backgroundThreads\`.
- Automatically calculate optimal defaults based on the pod's CPU requests/limits.
- Add unit tests verifying the calculation logic in \`src/controller/resources.rs\`.
- Document the impact of hyperthreading vs dedicated CPU cores.

### 📚 Resources
- [Stellar Core Performance Tuning](https://developers.stellar.org/docs/run-core-node/performance-tuning)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)"


# ─── 2 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Integrate with Vertical Pod Autoscaler (VPA) for Right-Sizing" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Enable seamless integration with the Kubernetes Vertical Pod Autoscaler to automatically adjust CPU and memory for Stellar infrastructure based on real-time usage.

### 📋 Context
Stellar nodes have highly variable resource requirements depending on network state (e.g., catching up vs fully synced). VPA helps automate the tedious task of right-sizing.

### ✅ Acceptance Criteria
- Add a \`vpa\` optional manifest to the Helm chart.
- The operator should automatically create a \`VerticalPodAutoscaler\` resource for each \`StellarNode\` when enabled.
- Support \`Auto\` and \`Initial\` update modes.
- Implement a safeguard to prevent VPA from restarting mission-critical validators during high-stress periods.
- Document the integration in the scalability guide.

### 📚 Resources
- [Vertical Pod Autoscaler Documentation](https://github.com/kubernetes/autoscaler/tree/master/vertical-pod-autoscaler)
- [\`charts/stellar-operator/values.yaml\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/charts/stellar-operator/values.yaml)"


# ─── 3 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement Property-Based Testing for Reconciler State Transitions" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Use property-based testing (e.g., using the \`proptest\` crate) to verify that the reconciler always reaches a stable state regardless of input order.

### 📋 Context
Distributed systems reconciliation can fail in subtle ways when events arrive out of order. Standard unit tests often miss these edge cases.

### ✅ Acceptance Criteria
- Integrate the \`proptest\` crate into the dev dependencies.
- Write properties for the \`apply_stellar_node\` logic verifying that the desired K8s state is eventually reached.
- Ensure that the finalizer logic is idempotent even under randomized failure simulations.
- Integrate into the CI pipeline with a dedicated test stage.

### 📚 Resources
- [Proptest Crate Documentation](https://docs.rs/proptest/latest/proptest/)
- [\`src/controller/reconciler_test.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler_test.rs)"


# ─── 4 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Hardening: Implement Seccomp and AppArmor Profiles for Workloads" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Restrict the system calls available to the Stellar Core and Horizon containers to reduce the attack surface in the event of a container breakout.

### 📋 Context
Running financial infrastructure requires the highest level of workload isolation. Default container runtimes allow more syscalls than necessary for Stellar processes.

### ✅ Acceptance Criteria
- Create custom, least-privilege Seccomp profiles for the operator and managed nodes.
- Update resource builders to inject \`securityContext\` with these profiles.
- Add support for AppArmor profiles where available on the underlying nodes.
- Verify that features like ledger snapshots and mTLS still function under the restricted profiles.
- Document the security hardening steps.

### 📚 Resources
- [Kubernetes Seccomp Profiles](https://kubernetes.io/docs/tutorials/security/seccomp/)
- [Kubernetes AppArmor Profiles](https://kubernetes.io/docs/tutorials/security/apparmor/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)"


# ─── 5 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar check-sync' diagnostic command in CLI" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Provide a specialized diagnostic tool that analyzes the sync lag of a specific node compared to its peers directly from the CLI.

### 📋 Context
Operators often need to quickly diagnose if a node is 'stuck' or just 'slow' without digging through raw logs or Grafana dashboards.

### ✅ Acceptance Criteria
- Add \`check-sync\` subcommand to the operator or plugin.
- The command should fetch the latest ledger sequence from the node and cross-reference it with public Stellar endpoints.
- Output a clear summary: 'Fully Synced', 'Catching Up (X ledgers behind)', or 'Stalled'.
- Include peer count and average RTT info if available via the internal API.

### 📚 Resources
- [Stellar Core HTTP API](https://developers.stellar.org/docs/run-core-node/commands#http-commands)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 6 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add Custom Grafana Dashboard for Multi-Node Fleet Overview" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Create a production-ready Grafana dashboard that provides a 'Single Pane of Glass' view for clusters managing multiple Stellar networks.

### 📋 Context
As the fleet grows, monitoring individual nodes becomes unmanageable. We need a high-level overview that flags deviations across the fleet.

### ✅ Acceptance Criteria
- Define a JSON-based Grafana dashboard in the \`charts/\` directory.
- Include panels for: Fleet Health (Validators vs Horizon), Aggregate TPR (Transactions Per Second), Sync Progress over time, and Resource Efficiency.
- Use templating to allow filtering by Namespace, Network, and NodeType.
- Package the dashboard as a sidecar-injectable ConfigMap.

### 📚 Resources
- [Grafana Dashboard JSON Model](https://grafana.com/docs/grafana/latest/dashboards/json-model/)
- [\`charts/stellar-operator/templates/dashboard.yaml\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/charts/stellar-operator/templates/dashboard.yaml)"


# ─── 7 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar-operator' Log Scrubbing for Sensitive Data" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Ensure that any potentially sensitive data (e.g., partial private keys, internal hashes) is never accidentally logged by the operator's reconciliation logic.

### 📋 Context
Even with structured logging, deep traces can sometimes capture raw payloads that contain internal state that shouldn't be exposed in log aggregation systems.

### ✅ Acceptance Criteria
- Implement a custom \`tracing::Layer\` that identifies and redacts sensitive patterns (e.g., base64 segments that look like seeds).
- Audit all \`info!\`, \`debug!\`, and \`error!\` calls in the reconciler to ensure they only log non-sensitive metadata.
- Add unit tests for the scrubbing layer.
- Document the redaction policy.

### 📚 Resources
- [Tracing Subscriber Layer Documentation](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/layer/trait.Layer.html)
- [\`src/telemetry.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/telemetry.rs)"


# ─── 8 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'Node Maintenance Mode' in the Reconciler" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Add a feature that allows operators to pause reconciliation for a specific node during manual maintenance without triggering alerts or remediation.

### 📋 Context
Sometimes an operator needs to manually poke a database or run a forensic tool. They don't want the controller 'fixing' things while they are working.

### ✅ Acceptance Criteria
- Add an \`annotation\` (e.g., \`stellar.org/maintenance: true\`) to the \`StellarNode\`.
- When set, the reconciler should skip all mutation steps but continue reporting health status.
- Add a visible flag to the \`stellar info\` output.
- Log a warning when a node remains in maintenance mode for over 24 hours.

### 📚 Resources
- [Kubernetes Annotations](https://kubernetes.io/docs/concepts/overview/working-with-objects/annotations/)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)"


# ─── 9 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar-operator' Self-Upgrade Simulation" "stellar-wave,enhancement,ci" "### 🔴 Difficulty: High (200 Points)

Add an automated E2E test that verifies the operator can be upgraded to a new version without causing downtime for managed Stellar nodes.

### 📋 Context
Upgrading the control plane shouldn't break the data plane. We need to guarantee that new operator versions handle the state of resources created by older versions.

### ✅ Acceptance Criteria
- Create a k6/bash script that performs an 'Old -> New' operator upgrade in a Kind cluster.
- Verify that leader election is handed over smoothly.
- Ensure managed pods are not unnecessarily restarted if their spec remains identical.
- Check that the \`status\` field is updated correctly after the upgrade.

### 📚 Resources
- [k6 Documentation](https://k6.io/docs/)
- [\`tests/e2e_kind.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/tests/e2e_kind.rs)"


# ─── 10 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add Property-Based Metadata Propagation for K8s Labels" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure that all labels applied to a \`StellarNode\` CRD are intelligently propagated to all child resources (Deployments, PVCs, Services).

### 📋 Context
Many organizational workflows depend on specific labels for billing, ownership, or network policy. These must be consistent across the entire resource tree.

### ✅ Acceptance Criteria
- Implement a whitelist/blacklist filter for label propagation.
- Automatically inject \`app.kubernetes.io/*\` labels based on the CRD values.
- Ensure that updates to CRD labels are reflected in child resources during the next reconciliation.
- Add unit tests verifying label inheritance.

### 📚 Resources
- [Kubernetes Labels and Selectors](https://kubernetes.io/docs/concepts/overview/working-with-objects/labels/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)"


# ─── 11 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar-operator' Readiness Probe leveraging Core status" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Enhance the operator's readiness check to ensure it not only 'can run' but 'has successfully connected' to all required dependencies.

### 📋 Context
A green health check often just means the HTTP server is up. We need a 'deep' health check that verifies connectivity to the K8s API and any internal caches.

### ✅ Acceptance Criteria
- Extend the \`/readyz\` endpoint to return 500 if the K8s watch stream is stalled.
- Monitor the latency of internal state updates.
- Export readiness metrics as a gauge.
- Update the Deployment to wait for this probe before allowing secondary leader replicas to shut down.

### 📚 Resources
- [Kubernetes Liveness, Readiness and Startup Probes](https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 12 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar prune' command for history archives" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Provide a utility that helps clean up old history archives from S3/Storage without having to manually parse Stellar Core's archive structure.

### 📋 Context
Over time, history archives grow and can become expensive. Automating the pruning of very old checkpoints (that are no longer needed for catch-up) is valuable.

### ✅ Acceptance Criteria
- Add \`prune-archive\` subcommand.
- Safely identify checkpoints older than a specified retention threshold.
- Execute the prune operation with 'dry-run' protection.
- Document the safety guarantees to prevent data loss.

### 📚 Resources
- [Stellar Core History Archives](https://developers.stellar.org/docs/run-core-node/history-archives)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 13 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add Support for PDB (Pod Disruption Budgets) in Helm Chart" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure that the Stellar-K8s infrastructure is protected during Kubernetes node drains and cluster upgrades.

### 📋 Context
Nodes are often evicted for maintenance. Without a PDB, a cluster could accidentally take down too many validators at once, risking quorum loss or transaction delay.

### ✅ Acceptance Criteria
- Add \`PodDisruptionBudget\` manifests to the Helm chart.
- Support both \`minAvailable\` and \`maxUnavailable\` configurations.
- Default to \`maxUnavailable: 1\` for validator nodes.
- Document how to handle PDBs during emergency cluster maintenance.

### 📚 Resources
- [Kubernetes Pod Disruption Budgets](https://kubernetes.io/docs/concepts/workloads/pods/disruptions/)
- [\`charts/stellar-operator/templates/pdb.yaml\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/charts/stellar-operator/templates/pdb.yaml)"


# ─── 14 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar diff' command to compare CRD vs Actual K8s" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Provide a way for operators to see a live 'diff' of what the operator *thinks* should be deployed versus what is *actually* in the cluster.

### 📋 Context
When things go wrong, it's hard to tell if the operator is failing to apply or if it's applied something that the pod runtime is rejecting.

### ✅ Acceptance Criteria
- Add \`diff\` subcommand.
- Compare the desired state (calculated from CRD) with live resources (fetched from API).
- Output a colored diff (similar to \`kubectl diff\`).
- Include internal fields like ConfigMaps and resource limits.

### 📚 Resources
- [Kubernetes Diff API](https://kubernetes.io/docs/reference/using-api/api-concepts/#dry-run)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 15 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar-operator' Memory Leak Detection in CI" "stellar-wave,enhancement,ci" "### 🔴 Difficulty: High (200 Points)

Add a long-running soak test in CI that monitors the operator's memory usage over 1 hour of constant reconciliation to catch potential leaks.

### 📋 Context
Even in Rust, memory can 'leak' if handles are held too long or global states are abused. This is critical for an operator that runs for months.

### ✅ Acceptance Criteria
- Add a \`soak-test\` workflow in GitHub Actions.
- Run a script that creates/deletes 100 StellarNodes repeatedly.
- Monitor RSS/Heap size using \`ps\` or \`prometheus\`.
- Fail CI if memory grows beyond a specific threshold (e.g., 5MB growth per hour).

### 📚 Resources
- [Valgrind / Memcheck](https://valgrind.org/docs/manual/mc-manual.html)
- [Rust Memory Management](https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html)
- [\`.github/workflows/ci.yml\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/ci.yml)"


# ─── 16 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'Automatic Checkpoint Integrity' check for Archives" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Add a background worker to the operator that periodically verifies that the checkpoints uploaded to S3/GCS are valid and not corrupted.

### 📋 Context
Archives are the last line of defense in disaster recovery. Finding out they are corrupted *during* a recovery is a nightmare.

### ✅ Acceptance Criteria
- Implement an \`archive_checker\` task.
- Download random historical checkpoints and verify their hashes against the ledger.
- Report integrity status as a Prometheus metric.
- Emit a Fatal Event if corruption is detected.

### 📚 Resources
- [Stellar Core Archive Verification](https://developers.stellar.org/docs/run-core-node/history-archives#verifying-archives)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)"


# ─── 17 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add Support for OPA/Gatekeeper Policies for StellarNode" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Provide a set of pre-built Rego policies to restrict malicious or invalid \`StellarNode\` specifications at the admission level.

### 📋 Context
Large teams might have users creating CRDs that request too many resources or use unapproved images. Admission policies enforce organization rules.

### ✅ Acceptance Criteria
- Create a set of \`ConstraintTemplates\` for: Resource limits, approved image registries, and required labels.
- Include these policies in the \`manifests/\` directory.
- Provide a guide on how to install and test these with OPA Gatekeeper.
- Verify policies don't block the operator's own reconciliation.

### 📚 Resources
- [OPA Gatekeeper Documentation](https://open-policy-agent.github.io/gatekeeper/website/docs/)
- [\`config/manifests/gatekeeper/\`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/config/manifests/gatekeeper)"


# ─── 18 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'Service Mesh' mTLS enforcement guide" "stellar-wave,documentation,security" "### 🔴 Difficulty: High (200 Points)

Create a detailed architecture document on how to run Stellar-K8s behind Istio or Linkerd with strict mTLS enabled.

### 📋 Context
For high-compliance environments, internal mTLS (using a service mesh) is often mandatory regardless of individual application support.

### ✅ Acceptance Criteria
- New document \`docs/service-mesh.md\`.
- Cover: Istio sidecar injection, PeerAuthentication policies, and cross-cluster mTLS for Stellar P2P.
- Document any needed 'ServiceEntry' or 'VirtualService' configurations for external peer discovery.

### 📚 Resources
- [Istio mTLS Documentation](https://istio.io/latest/docs/tasks/security/authentication/mtls-migration/)
- [Linkerd mTLS Documentation](https://linkerd.io/2.12/features/automatic-mtls/)"


# ─── 19 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar-operator' Crash Loop Analysis sidecar" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Add a small sidecar to the operator deployment that captures and analyzes crash logs to provide human-readable 'Fix Recommendations'.

### 📋 Context
Stack traces are scary for junior operators. A sidecar that sees 'Connection Refused' and says 'Check your NetworkPolicies' would be high-value.

### ✅ Acceptance Criteria
- Create a lightweight diagnostic sidecar (Rust/Bash).
- Capture logs from the main operator container.
- Use pattern matching to identify common issues (RBAC, API timeouts, DB errors).
- Output the 'Fix Recommendation' to the pod description (via events/annotations).

### 📚 Resources
- [Kubernetes Sidecar Containers](https://kubernetes.io/docs/concepts/workloads/pods/sidecar-containers/)
- [\`src/sidecar.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/sidecar.rs)"


# ─── 20 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add Support for Node Anti-Affinity based on SCP slices" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Implement a mechanism where the operator intelligently spreads validator pods across different hardware nodes based on their quorum set membership.

### 📋 Context
If all validators in a single quorum slice end up on the same physical host, an underlying hardware failure could stall the entire network.

### ✅ Acceptance Criteria
- Implement an 'SCP-Aware' placement logic.
- The operator should inject \`podAntiAffinity\` rules that discourage placing nodes from the same slice on the same \`kubernetes.io/hostname\`.
- Support this configuration via a new \`placement\` block in the CRD.
- Document the impact on network liveness.

### 📚 Resources
- [Kubernetes Pod Anti-Affinity](https://kubernetes.io/docs/concepts/scheduling-eviction/assign-pod-node/#inter-pod-affinity-and-anti-affinity)
- [\`src/crd/types.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/types.rs)"


# ─── 21 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'Stellar-K8s' Documentation Search Engine" "stellar-wave,documentation,dx" "### 🔴 Difficulty: High (200 Points)

Add a search engine (e.g., using Algolia or a local Lunr.js index) to the documentation portal to make finding guides easier.

### 📋 Context
With 70+ issues and dozen of guides, the documentation is becoming dense. Users need a quick way to find 'mTLS rotation' or 'S3 backup config'.

### ✅ Acceptance Criteria
- Integrate a search UI into the documentation site.
- Ensure all Markdown files are indexed correctly.
- Support keyword highlighting and search suggestions.
- Add and document a local 'offline' search tool for the CLI.

### 📚 Resources
- [Algolia Documentation](https://www.algolia.com/doc/)
- [Lunr.js Documentation](https://lunrjs.com/)"


# ─── 22 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar-operator' Dynamic Log Level Control" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Add a feature to change the operator's log level (e.g., from INFO to DEBUG) at runtime without requiring a pod restart.

### 📋 Context
Debugging a live issue often requires more verbose logs, but restarting the operator pod resets the state and might 'fix' the issue being debugged.

### ✅ Acceptance Criteria
- Add a new endpoint to the REST API (\`/config/log-level\`).
- Implement live reloading of the \`tracing-subscriber\` filter.
- Support time-limited debug modes (e.g., 'Enable DEBUG for 5 minutes').
- Secure the endpoint with mTLS or an internal token.

### 📚 Resources
- [Tracing Subscriber Reloading](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/reload/index.html)
- [\`src/rest_api/server.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/rest_api/server.rs)"


# ─── 23 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'Hardware Asset' tracking for Validator Nodes" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Track and expose the underlying hardware generation (e.g., Intel Icelake, Graviton 3) of the nodes running Stellar validators for performance auditing.

### 📋 Context
Validator performance can vary significantly between CPU generations. Operators need this info in their dashboards to identify 'noisy neighbors' or slow hosts.

### ✅ Acceptance Criteria
- The operator should inspect Node labels (\`feature.node.kubernetes.io/*\`).
- Expose the hardware generation as a Prometheus label on node metrics.
- Add an 'Infra Details' section to \`stellar info\`.
- Document how to use this for performance-aware scheduling.

### 📚 Resources
- [Node Feature Discovery (NFD)](https://github.com/kubernetes-sigs/node-feature-discovery)
- [\`src/controller/metrics.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/metrics.rs)"


# ─── 24 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'Stellar-K8s' Post-Mortem Template and Tooling" "stellar-wave,documentation,dx" "### 🔴 Difficulty: High (200 Points)

Create a standardized post-mortem process for outages in managed Stellar infrastructure, including a tool to gather all relevant metrics/logs for a specific window.

### 📋 Context
When an outage happens, the first thing needed is a clean timeline of logs, metrics, and events. Automating this 'Gathering' phase is very high-value.

### ✅ Acceptance Criteria
- Create a \`docs/incident-response/post-mortem.md\` template.
- Implement \`stellar incident-report\` command.
- The command should automatically zip up: operator logs, pod logs, relevant K8s events, and a snapshot of the CRD status for the duration of the incident.
- Include a 'Lessons Learned' section in the final output.

### 📚 Resources
- [Google SRE Book: Post-mortem Culture](https://sre.google/sre-book/postmortem-culture/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


echo ""
echo "🎉 Batch 18 (24 x 200 pts) issues created successfully! Backlog depth++"

print_skip_summary
