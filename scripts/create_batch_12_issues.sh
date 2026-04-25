#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/repo.sh"
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 12, 15 x 200 pts).

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

EXPECTED_ISSUE_COUNT=15
ACTUAL_ISSUE_COUNT=$(grep -c '^create_issue_with_retry' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT. Update EXPECTED_ISSUE_COUNT or fix the script." >&2
  exit 1
fi

echo "Creating Batch 12 (15 x 200 pts) issues with auto-retry..."

# ─── ISSUE 1 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Distributed Tracing with OpenTelemetry (OTel)" \
  "stellar-wave,enhancement,observability" \
  "### 🔴 Difficulty: High (200 Points)

Logging and metrics only show part of the story. We need distributed tracing across the operator, the REST API, and the admission webhook to understand reconcile latency and request flow.

### ✅ Acceptance Criteria
- Add \`tracing-opentelemetry\` and \`opentelemetry-otlp\` dependencies.
- Configure an OTLP exporter to send traces to a collector (e.g., Jaeger or Tempo).
- Instrument the reconciler and the Axum API server.
- Ensure trace context is propagated through Kubernetes events and webhook requests.
- Provide a Grafana dashboard example that shows tracing spans.

### 📚 Resources
- [OpenTelemetry Rust SDK](https://github.com/open-telemetry/opentelemetry-rust)
- [Tracing in Axum](https://docs.rs/axum/latest/axum/middleware/index.html#tracing)"


# ─── ISSUE 2 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Custom Kubernetes Scheduler Plugin for Quorum Proximity" \
  "stellar-wave,enhancement,performance" \
  "### 🔴 Difficulty: High (200 Points)

Standard Kubernetes scheduling doesn't understand SCP quorum sets. We want to ensure nodes in the same quorum set are placed appropriately (e.g., in different failure domains but with low latency).

### ✅ Acceptance Criteria
- Implement a [Kubernetes Scheduler Plugin](https://kubernetes.io/docs/concepts/scheduling-eviction/scheduling-framework/) (using the Go SDK or a Rust equivalent like \`kube-scheduler-rs\`).
- The plugin should read the \`StellarNode\` quorum config and influence the \`Filter\` and \`Score\` phases.
- Prioritize nodes that provide the best latency/redundancy balance for a specific quorum set.
- Provide documentation on how to configure K8s to use this custom scheduler.

### 📚 Resources
- [Kubernetes Scheduling Framework](https://kubernetes.io/docs/concepts/scheduling-eviction/scheduling-framework/)
- [Kube-scheduler-rs](https://github.com/kube-rs/kube-scheduler-rs)"


# ─── ISSUE 3 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Wasm-Powered Admission Controller Layer" \
  "stellar-wave,enhancement,security" \
  "### 🔴 Difficulty: High (200 Points)

To match the philosophy of Soroban, we should allow users to write custom validation policies for \`StellarNode\` resources in WebAssembly.

### ✅ Acceptance Criteria
- Integrate a Wasm runtime (like \`wasmtime\` or \`wasmer\`) into the Mutating/Validating webhook.
- Allow the operator to load user-defined \`.wasm\` policies from a ConfigMap.
- Policies should be able to reject or mutate resources based on custom logic (e.g., enforcing specific image registries or resource limits).
- Provide a simple Rust/Wasm policy example.

### 📚 Resources
- [Wasmtime: Embedding in Rust](https://docs.wasmtime.dev/examples-rust-hello-world.html)
- [Kubernetes Admission Webhooks](https://kubernetes.io/docs/reference/access-authn-authz/extensible-admission-controllers/)"


# ─── ISSUE 4 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Multi-Region Artifact Sync via OCI-based Snapshots" \
  "stellar-wave,enhancement,performance" \
  "### 🔴 Difficulty: High (200 Points)

Sharing ledger snapshots across regions using standard PVCs is difficult. We want to leverage OCI registries (like Docker Hub, GHCR) to store and sync ledger snapshots.

### ✅ Acceptance Criteria
- Implement a module that can package a ledger snapshot into an OCI image layer.
- Update the operator to push these snapshots to a registry and pull them to bootstrap new nodes in different clusters/regions.
- Automate this process via a Job triggered by the operator.
- Ensure credentials for the registry are handled via K8s Secrets.

### 📚 Resources
- [OCI Distribution Spec](https://github.com/opencontainers/distribution-spec)
- [ORAS (OCI Registry As Storage)](https://oras.land/)"


# ─── ISSUE 5 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Advanced SCP Quorum Analysis & Reliability Dashboard" \
  "stellar-wave,observability,reliability" \
  "### 🔴 Difficulty: High (200 Points)

A validator is only as strong as its quorum. We need a dashboard that visualizes the entire network's quorum health from the operator's perspective.

### ✅ Acceptance Criteria
- Build a custom dashboard (or a Prometheus exporter) that calculates:
  - \"Critical Nodes\": Nodes whose failure would break the quorum.
  - Quorum Set Overlaps.
  - Real-time SCP consensus latency per node.
- Use the data to update the \`StellarNodeStatus\` with a \`QuorumFragility\` score.

### 📚 Resources
- [StellarBeat API](https://api.stellarbeat.io/v1/)
- [SCP Quorum Sets](https://developers.stellar.org/docs/run-core-node/quorums)"


# ─── ISSUE 6 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "CVE-Triggered Automated Image Rollouts" \
  "stellar-wave,enhancement,security" \
  "### 🔴 Difficulty: High (200 Points)

Security is paramount for financial infrastructure. We want the operator to automatically update node images when a critical CVE is detected in the current version.

### ✅ Acceptance Criteria
- Integrate with a vulnerability scanner API (e.g., Trivy or Snyk).
- The operator should periodically check the current image version for vulnerabilities.
- If a \"Critical\" CVE is found and a patch version is available, the operator should automatically update the \`StellarNode\` spec and trigger a rollout.
- Include a safety gate (e.g., an annotation to opt-in/out).

### 📚 Resources
- [Trivy: Vulnerability Scanner](https://github.com/aquasecurity/trivy)
- [Snyk API](https://snyk.docs.apiary.io/)"


# ─── ISSUE 7 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Service Mesh Integration: Istio/Linkerd mTLS Enforcement" \
  "stellar-wave,enhancement,security" \
  "### 🔴 Difficulty: High (200 Points)

Move beyond basic K8s networking and integrate with a Service Mesh for advanced traffic control and mTLS.

### ✅ Acceptance Criteria
- Provide \`PeerAuthentication\` and \`DestinationRule\` manifests for Istio.
- Ensure the operator is compatible with sidecar injection.
- Implement circuit breaking and retries for cross-node communication via the mesh config.
- Verify through E2E tests that all traffic is encrypted and authenticated via the mesh.

### 📚 Resources
- [Istio PeerAuthentication](https://istio.io/latest/docs/reference/config/security/peer-authentication/)
- [Linkerd mTLS Guide](https://linkerd.io/2.12/features/automatic-mtls/)"


# ─── ISSUE 8 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Automated Performance Regression Testing as-a-Service" \
  "stellar-wave,testing,performance" \
  "### 🔴 Difficulty: High (200 Points)

We need to ensure that no PR degrades the performance of the Stellar nodes or the operator itself.

### ✅ Acceptance Criteria
- Implement a GitHub Action or a separate controller that spins up a \`kind\` cluster on every PR.
- Run a standardized load test using \`k6\` and compare the results (TPS/Latency) with a baseline in the \`main\` branch.
- Fail the CI if the performance drops below a certain threshold.
- Post a summary comment on the PR with a performance comparison table.

### 📚 Resources
- [k6: Load Testing for Kubernetes](https://k6.io/docs/using-k6-on-kubernetes/testing-k8s-applications/)
- [GitHub Actions: Pull Request Comments](https://github.com/actions/github-script)"


# ─── ISSUE 9 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "FinOps: Resource-to-Cost Mapping for Stellar Infrastructure" \
  "stellar-wave,enhancement,performance" \
  "### 🔴 Difficulty: High (200 Points)

Help operators understand the cost of their infrastructure.

### ✅ Acceptance Criteria
- Implement a controller that integrates with cloud pricing APIs (AWS, GCP, Azure).
- Annotate \`StellarNode\` resources with their estimated monthly cost based on spec requirements (CPU/RAM/Storage).
- Export this data as Prometheus metrics to visualize \"Cost per Ledger\" or \"Cost per Transaction\".

### 📚 Resources
- [Infracost API](https://www.infracost.io/docs/integrations/api/)
- [Kube-cost](https://www.kubecost.com/)"


# ─── ISSUE 10 (200 pts) ───────────────────────────────────────────────────────
create_issue_with_retry \
  "State-Machine Model Checking for the Reconciler" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

Using formal methods like TLA+ or model checking tools to prove the correctness of the reconciliation logic.

### ✅ Acceptance Criteria
- Create a TLA+ model of the \`StellarNode\` reconciler.
- Prove that the reconciler always eventually reaches a stable state (liveness) and never enters an invalid state (safety).
- Document findings and any edge cases discovered during modeling.

### 📚 Resources
- [TLA+ Home Page](https://lamport.azurewebsites.net/tla/tla.html)
- [Learn TLA+](https://learntla.com/)"


# ─── ISSUE 11 (200 pts) ───────────────────────────────────────────────────────
create_issue_with_retry \
  "Native HashiCorp Vault Integration for Stellar Secret Management" \
  "stellar-wave,enhancement,security" \
  "### 🔴 Difficulty: High (200 Points)

Go beyond basic KMS and integrate deeply with Vault for dynamic secret injection.

### ✅ Acceptance Criteria
- Support the Vault Agent sidecar pattern.
- Implement a custom \`VaultSecretRef\` in the CRD.
- Ensure the operator can automatically rotate secrets in Vault and trigger a restart of the concerned nodes.
- Provide a full tutorial and manifests for a production-ready Vault setup.

### 📚 Resources
- [Vault Agent Auto-Auth](https://developer.hashicorp.com/vault/docs/agent/autoauth)
- [\`src/crd/types.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/types.rs)"


# ─── ISSUE 12 (200 pts) ───────────────────────────────────────────────────────
create_issue_with_retry \
  "Automated Disaster Recovery (DR) Drill Orchestrator" \
  "stellar-wave,enhancement,reliability" \
  "### 🔴 Difficulty: High (200 Points)

DR is only useful if it's tested. We want the operator to automatically run \"DR Drills\".

### ✅ Acceptance Criteria
- Add a \`drDrillSchedule\` to the CRD.
- Periodically trigger a fake failover (by killing the primary or simulating network latency).
- Measure the Time to Recovery (TTR).
- Verify the standby successfully took over and the application stayed alive.
- Generate a report after the drill.

### 📚 Resources
- [Chaos Engineering: Principles and Practice](https://principlesofchaos.org/)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)"


# ─── ISSUE 13 (200 pts) ───────────────────────────────────────────────────────
create_issue_with_retry \
  "Stellar-K8s Operator Performance Dashboard (Custom UI)" \
  "stellar-wave,observability,dx" \
  "### 🔴 Difficulty: High (200 Points)

A custom web-based UI for the operator (likely using Rust/Leptos).

### ✅ Acceptance Criteria
- Build a lightweight web dashboard that visualizes the managed CRDs.
- Show live logs and status conditions in a pretty format.
- Allow simple actions like \"Trigger Manual Snapshot\" or \"Restart Node\" from the UI.
- Secure the UI using OIDC/Kubernetes RBAC.

### 📚 Resources
- [Leptos: Full-stack Rust Framework](https://leptos.dev/)
- [Kubernetes Dashboard Architecture](https://github.com/kubernetes/dashboard)"


# ─── ISSUE 14 (200 pts) ───────────────────────────────────────────────────────
create_issue_with_retry \
  "One-Click Local Simulator: 'stellar simulator up'" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

Make it trivial to start a local testing environment.

### ✅ Acceptance Criteria
- Enhance the \`stellar-operator\` binary with a \`simulator\` command.
- It should spin up a local \`k3s\` or \`kind\` cluster, install the operator, and deploy a 3-node validator network automatically.
- Provide a simple CLI output showing the cluster health and local endpoints.

### 📚 Resources
- [Kind: Kubernetes in Docker](https://kind.sigs.k8s.io/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── ISSUE 15 (200 pts) ───────────────────────────────────────────────────────
create_issue_with_retry \
  "Automated Horizon to Soroban-RPC Migration Path" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

Help users move to the latest technology.

### ✅ Acceptance Criteria
- Implement a migration controller that can automatically convert a \`nodeType: Horizon\` node to a \`nodeType: SorobanRpc\` node.
- Handle data migration and configuration updates.
- Ensure the migration is zero-downtime by running nodes in parallel during the transition.

### 📚 Resources
- [Stellar Horizon to Soroban-RPC Migration Guide](https://developers.stellar.org/docs/smart-contracts/getting-started/soroban-rpc)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)"


echo ""
echo "🎉 Batch 12 (15 x 200 pts) elite issues created successfully!"

print_skip_summary
