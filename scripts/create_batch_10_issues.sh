#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/repo.sh"
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 10, 8 x 200 pts + 2 x 150 pts).

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

echo "Creating Batch 10 (8 x 200 pts, 2 x 150 pts) issues with auto-retry..."

# ─── ISSUE 1 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Vertical Pod Autoscaler (VPA) Integration" \
  "stellar-wave,enhancement,performance" \
  "### 🔴 Difficulty: High (200 Points)

Stellar nodes (especially Validator and Horizon) have highly variable resource needs depending on network activity. We need to integrate with the Kubernetes **Vertical Pod Autoscaler (VPA)** to automatically recommend or apply CPU/Memory adjustments.

### ✅ Acceptance Criteria
- Add a new \`vpa_config\` field to \`StellarNodeSpec\`.
- Update the reconciler to generate and manage \`VerticalPodAutoscaler\` resources (API group \`autoscaling.k8s.io\`) for the managed pods.
- Support both \"Initial\" (recommendation only) and \"Auto\" (automatic restart) update modes.
- Provide a YAML example in \`examples/vpa-scaling.yaml\`.
- Write unit tests in \`src/controller/resources.rs\` (or similar) verifying VPA resource generation.

### 📚 Resources
- [Vertical Pod Autoscaler docs](https://github.com/kubernetes/autoscaler/tree/master/vertical-pod-autoscaler)
- [\`src/crd/stellar_node.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/stellar_node.rs)
"

# ─── ISSUE 2 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Integrate Chaos Engineering (Chaos Mesh) for Operator Resilience" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

To prove this Rust operator is truly \"enterprise-ready\", it must survive catastrophic cluster events. We need to integrate **Chaos Mesh** tests into our CI or verification suite.

### ✅ Acceptance Criteria
- Create a new directory \`tests/chaos/\`.
- Implement a suite of Chaos experiments (using YAML or the Chaos Mesh SDK) that:
  1. Kills the operator pod while it's in the middle of a reconciliation.
  2. Partitions the network between the operator and the K8s API.
  3. Induces high latency on the K8s API.
- Verify that the operator recovers gracefully and eventually converges the \`StellarNode\` state to healthy.
- Document how to run these chaos tests in a local \`kind\` cluster.

### 📚 Resources
- [Chaos Mesh Documentation](https://chaos-mesh.org/)
- [\`tests/e2e_kind.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/tests/e2e_kind.rs)
"

# ─── ISSUE 3 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Advanced VSL Module: Signature Verification and Quorum Set Parsing" \
  "stellar-wave,enhancement,security" \
  "### 🔴 Difficulty: High (200 Points)

The current \`src/controller/vsl.rs\` is a placeholder that only fetches raw text. A production VSL must be parsed and its cryptographic signatures verified to prevent quorum set poisoning.

### ✅ Acceptance Criteria
- Enhance \`fetch_vsl\` to parse the downloaded TOML content into a structured \`QuorumSet\` type.
- Implement ECDSA/ED25519 signature verification for VSL files from trusted Stellar entities.
- Integrate the verified VSL results into the \`stellar-core.cfg\` generation logic.
- Add comprehensive unit tests in \`src/controller/vsl.rs\` with mock signed VSL payloads.

### 📚 Resources
- [Stellar Core Quorum Configuration](https://developers.stellar.org/docs/run-core-node/configuring#quorum-set)
- [\`src/controller/vsl.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/vsl.rs)
"

# ─── ISSUE 4 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Read Pool Optimization: HPA and Load-Balanced Service" \
  "stellar-wave,enhancement,performance" \
  "### 🟡 Difficulty: Medium (150 Points)

The \`read_pool.rs\` module creates a StatefulSet but lacks a stable Service for clients and an HPA for scaling based on demand.

### ✅ Acceptance Criteria
- Update \`src/controller/read_pool.rs\` to generate a Kubernetes \`Service\` (ClusterIP) that points to the pool of read-only replicas.
- Add Support for \`HorizontalPodAutoscaler\` (v2) to scale the read-replica replicas based on CPU/Memory usage.
- Update the \`StellarNodeStatus\` to include the endpoint for the read pool service.
- Verify these resources are correctly cleaned up when \`read_replica_config\` is removed from the spec.

### 📚 Resources
- [\`src/controller/read_pool.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/read_pool.rs)
"

# ─── ISSUE 5 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Network Topology Aware Placement (TopologySpreadConstraints)" \
  "stellar-wave,enhancement,reliability" \
  "### 🟡 Difficulty: Medium (150 Points)

To ensure high availability, Stellar nodes should be spread across different hardware, zones, or regions. We need to implement \`topologySpreadConstraints\` in our resource generation.

### ✅ Acceptance Criteria
- Add \`topology_spread_constraints\` support to the \`StellarNodeSpec\`.
- Ensure the reconciler applies these constraints to the generated \`PodSpec\` for both validators and replica pools.
- Use \`topologyKey: kubernetes.io/hostname\` and \`topologyKey: topology.kubernetes.io/zone\` as defaults.
- Write unit tests in \`src/controller/resources_test.rs\` verifying the constraints are correctly injected into the Pods.

### 📚 Resources
- [Pod Topology Spread Constraints](https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)
"

# ─── ISSUE 6 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "KMS Integration for Vault-backed Secret Management" \
  "stellar-wave,enhancement,security" \
  "### 🔴 Difficulty: High (200 Points)

Storing validator seeds in raw Kubernetes Secrets is acceptable for development, but production requires integration with a Key Management Service (KMS) or HashiCorp Vault.

### ✅ Acceptance Criteria
- Add support for fetching validator seeds from an external KMS (AWS KMS, GCP KMS) or Vault via a sidecar or CSI Secret Store.
- Implement an \`ExternalSecrets\` pattern or integrate with \`secret-store-csi-driver\`.
- Update the \`StellarNode\` CRD to support \`seedSecretRef\` pointing to an external source.
- Provide documentation on how to securely pipe secrets into the operator without them ever being logged in plaintext.

### 📚 Resources
- [Secret Store CSI Driver](https://secrets-store-csi-driver.sigs.k8s.io/)
- [\`src/controller/captive_core.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/captive_core.rs)
"

# ─── ISSUE 7 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Automated Upgrade Strategy: Canary Rollouts for Horizon" \
  "stellar-wave,enhancement,reliability" \
  "### 🔴 Difficulty: High (200 Points)

Upgrading the Horizon API can be risky. We need an automated upgrade strategy that supports Canary rollouts to minimize the blast radius of a bad release.

### ✅ Acceptance Criteria
- Implement a \`CanaryUpgrade\` strategy in the reconciler.
- When the \`version\` in \`StellarNodeSpec\` changes for a Horizon node:
  1. Create a single Pod with the new version (Canary).
  2. Monitor health checks for N minutes.
  3. If healthy, proceed with the full rolling update. If unhealthy, rollback the canary.
- Integrate with the \`traffic.rs\` module to split a small percentage of traffic to the Canary.
- Provide an example YAML showing the canary configuration.

### 📚 Resources
- [Progressive Delivery on Kubernetes](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/#canary-deployment)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)
"

# ─── ISSUE 8 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Stellar Core History Archive Integrity Checker" \
  "stellar-wave,enhancement,reliability,observability" \
  "### 🔴 Difficulty: High (200 Points)

History archives are the backbone of Stellar's decentralized history. We need a controller that periodically verifies the integrity of these archives by checking for stale ledgers or missing checkpoints.

### ✅ Acceptance Criteria
- Add a new \`ArchiveCheck\` routine in the operator (or integrate into \`archive_health.rs\`).
- Periodically (every 1 hour) download the \`stellar-history.json\` from the configured \`historyArchiveUrls\`.
- Compare the ledger sequence in the archive with the actual network state.
- If the archive is lagging significantly, update the \`StellarNodeStatus\` with a \`Degraded\` condition and fire a Prometheus alert.
- Unit test the edge cases (archive unreachable, malformed JSON, sync lag detection).

### 📚 Resources
- [\`src/controller/archive_health.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/archive_health.rs)
- [Stellar History Archives](https://developers.stellar.org/docs/run-core-node/publishing-history-archives/)
"

# ─── ISSUE 9 (200 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Operator Webhook Performance: Load Testing & Latency Benchmarks" \
  "stellar-wave,testing,performance" \
  "### 🔴 Difficulty: High (200 Points)

Rust's primary advantage in Kubernetes operators is low latency for webhooks. We need to quantify this and ensure no regressions occur.

### ✅ Acceptance Criteria
- Implement a performance benchmarking suite for the \`StellarNode\` Validation and Mutation webhooks.
- Use a tool like \`k6\` or \`ghz\` to simulate 100+ concurrent admission requests.
- Measure the Latency (p99) and Throughput.
- Compare the results against a baseline (e.g., a simple Go webhook) if possible.
- The results must be automatically uploaded as a CI artifact and formatted into a Markdown report on the PR.

### 📚 Resources
- [k6 - Load Testing for Kubernetes](https://k6.io/docs/testing-guides/test-kubernetes/)
- [\`src/webhook/server.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/webhook/server.rs)
"

# ─── ISSUE 10 (200 pts) ───────────────────────────────────────────────────────
create_issue_with_retry \
  "Custom Grafana Dashboard for SOROBAN Specific Metrics" \
  "stellar-wave,observability,enhancement" \
  "### 🔴 Difficulty: High (200 Points)

Soroban (the Stellar smart contract platform) introduces new metrics like Host Function execution time, Wasm VM memory usage, and contract invocation rates. We need a specialized dashboard to monitor these.

### ✅ Acceptance Criteria
- Design a JSON Grafana dashboard specifically for Soroban RPC nodes.
- Panels must include:
  - Wasm execution time (histogram)
  - Contract storage fee distribution
  - Resource consumption (CPU/RAM) per contract invocation
  - Success/Failure rate of Soroban transactions
- Save the dashboard in \`monitoring/grafana-soroban.json\`.
- Add a section to the README explaining Soroban-specific observability.

### 📚 Resources
- [Soroban Metrics Documentation](https://soroban.stellar.org/docs/fundamentals-and-concepts/state-of-the-network)
- [\`src/controller/metrics.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/metrics.rs)
"

echo ""
echo "🎉 Batch 10 (8x200, 2x150) issues created successfully!"

print_skip_summary
