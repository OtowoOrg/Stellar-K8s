#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
source "$(dirname "$0")/lib/repo.sh"

# Stellar-K8s Wave Issue Creation Script - BATCH 4
# 6 High (200 pts), 2 Medium (150 pts), 2 Trivial (100 pts)

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 4, mixed difficulty).

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

echo "Creating Batch 4 (Mixed) issues..."

# --- HIGH (200 pts) ---

# 29. Chaos Mesh Integration (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Integrate Chaos Mesh for Network Partition Testing" \
  --body "### 🔴 Difficulty: High (200 Points)

To ensure the operator handles fragile network conditions gracefully, we need to integrate Chaos Mesh. This task involves creating automated chaos scenarios to test node recovery during network partitions.

### ✅ Acceptance Criteria
- Create \`tests/chaos/\` directory with Chaos Mesh manifests (NetworkChaos).
- Implement a test script that triggers a partition and verifies the Operator's recovery logic.
- Document the \"Failure Mode and Effects Analysis\" (FMEA) for StellarNode.

### 📚 Resources
- [Chaos Mesh Documentation](https://chaos-mesh.org/docs/simulate-network-chaos-on-kubernetes/)
- [Stellar Core Recovery Logic](https://developers.stellar.org/docs/run-core-node/prerequisites)
- [\`tests/chaos/\`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/tests/chaos)
" --label "stellar-wave,reliability,architecture"

# 30. Dynamic Peer Discovery (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Dynamic Peer Discovery Controller" \
  --body "### 🔴 Difficulty: High (200 Points)

Currently, peers are defined statically. We need a controller that dynamically discovers other \`StellarNode\` resources in the cluster and updates the \`KNOWN_PEERS\` configuration in real-time.

### ✅ Acceptance Criteria
- Implement a watcher for \`StellarNode\` resources.
- Automatically update a shared \`ConfigMap\` with the latest peer IPs/Ports.
- Trigger a rolling update or signal the Stellar process to refresh configuration.

### 📚 Resources
- [Stellar Core Peers Config](https://github.com/OtowoOrg/Stellar-K8s/blob/main/docs/stellar-core_example.cfg)
- [kube-rs Runtime Watcher](https://kube.rs/controllers/watcher/)
- [\`src/controller/peer_discovery.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/peer_discovery.rs)
" --label "stellar-wave,architecture,logic"

# 31. Multi-Cluster Support (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Add Multi-Cluster Orchestration Support" \
  --body "### 🔴 Difficulty: High (200 Points)

Large Stellar deployments should span multiple Kubernetes clusters. This task involves updating the operator to support cross-cluster communication and synchronization.

### ✅ Acceptance Criteria
- Add \`cluster: String\` field to \`StellarNodeSpec\`.
- Support ExternalName services or service-mesh (Submariner/Istio) for cross-cluster DNS.
- Implement logic to handle node latency thresholds between clusters.

### 📚 Resources
- [Submariner Multi-cluster Networking](https://submariner.io/)
- [Stellar Network Topologies](https://developers.stellar.org/docs/run-core-node/network-topologies)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)
" --label "stellar-wave,architecture,kubernetes"

# 32. Auto-Remediation for Stale Ledgers (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Auto-Remediation for Stale/Desynced Nodes" \
  --body "### 🔴 Difficulty: High (200 Points)

If a node gets stuck or significantly behind the network, it may need an automated restart or a fresh sync. The operator should detect this and perform safe remediation.

### ✅ Acceptance Criteria
- Detect \"stale\" state (ledger height not increasing for X minutes).
- Implement automated remediation steps: Restart -> Clear DB -> Fresh Sync.
- Emit Kubernetes Events for every automated action taken.

### 📚 Resources
- [Monitoring Stellar Core](https://developers.stellar.org/docs/run-core-node/monitoring)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)
" --label "stellar-wave,reliability,logic"

# 33. Cloud KMS/HSM Integration (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Cloud KMS/HSM Integration for Node Keys" \
  --body "### 🔴 Difficulty: High (200 Points)

Storing node keys in plain Kubernetes Secrets is not sufficient for high-security environments. Integrate with cloud-native KMS (AWS KMS, Google Cloud KMS) or HSMs.

### ✅ Acceptance Criteria
- Support \`keySource: KMS\` in the spec.
- Implement an InitContainer that fetches and decrypts keys from a Vault/KMS.
- Ensure keys never touch the disk in plaintext.

### 📚 Resources
- [AWS KMS for Kubernetes](https://aws.amazon.com/premiumsupport/knowledge-center/eks-kms-secrets-encryption/)
- [Stellar Node Security](https://developers.stellar.org/docs/run-core-node/security-best-practices)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)
" --label "stellar-wave,security,architecture"

# 34. OpenTelemetry Tracing (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Add OpenTelemetry Tracing Support" \
  --body "### 🔴 Difficulty: High (200 Points)

Debugging complex operator logic requires distributed tracing. Implement OpenTelemetry support throughout the controller and API.

### ✅ Acceptance Criteria
- Integrate \`opentelemetry-rs\`.
- Trace reconciliation loops and resource patching actions.
- Export traces to a configurable OTLP endpoint (Jaeger/Tempo).

### 📚 Resources
- [OpenTelemetry Rust](https://github.com/open-telemetry/opentelemetry-rust)
- [Distributed Tracing in K8s](https://kubernetes.io/docs/concepts/cluster-administration/system-logs/#distributed-tracing)
- [\`src/telemetry.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/telemetry.rs)
" --label "stellar-wave,observability,rust"

# --- MEDIUM (150 pts) ---

# 35. mTLS for Node-to-Node Communication (Medium - 150 pts)
gh issue create --repo "$REPO" \
  --title "Implement mTLS for Internal Node Communication" \
  --body "### 🟡 Difficulty: Medium (150 Points)

Secure the traffic between Stellar nodes and the Operator REST API using mutual TLS (mTLS).

### ✅ Acceptance Criteria
- Automate certificate distribution to pods.
- Enable mTLS verification in the \`rest_api\` module.
- Provide a CLI flag to enable/disable strict mTLS.

### 📚 Resources
- [mTLS Explained](https://www.cloudflare.com/learning/access-management/what-is-mutual-tls/)
- [\`src/rest_api/\`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/src/rest_api)
" --label "stellar-wave,security,feature"

# 36. Canary Rollouts with Traffic Weighting (Medium - 150 pts)
gh issue create --repo "$REPO" \
  --title "Support Canary Rollouts with Traffic Weighting" \
  --body "### 🟡 Difficulty: Medium (150 Points)

When upgrading Horizon or Soroban RPC, we should support canary deployments where only a percentage of traffic hits the new version.

### ✅ Acceptance Criteria
- Add \`strategy: Canary\` to the spec with a \`weight\` field.
- Update Ingress annotations to support traffic splitting (Nginx/Istio/Traefik).
- Implement automated rollback if health checks fail.

### 📚 Resources
- [Canary Deployments on Kubernetes](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/#canary-deployment)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)
" --label "stellar-wave,kubernetes,feature"

# --- TRIVIAL (100 pts) ---

# 37. CLI Version and Info Subcommands (Trivial - 100 pts)
gh issue create --repo "$REPO" \
  --title "Add 'version' and 'info' subcommands to CLI" \
  --body "### 🟢 Difficulty: Trivial (100 Points)

Provide users with a way to check the operator version, build date, and basic cluster status via the binary.

### ✅ Acceptance Criteria
- Implement \`version\` subcommand using \`clap\`.
- Implement \`info\` subcommand showing current managed Node count.
- Print build metadata (Git SHA, Rust version).

### 📚 Resources
- [Clap (Rust) Documentation](https://docs.rs/clap/latest/clap/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)
" --label "stellar-wave,good-first-issue,rust"

# 38. Improved CRD Validation Formatting (Trivial - 100 pts)
gh issue create --repo "$REPO" \
  --title "Improve CRD Validation Error Formatting" \
  --body "### 🟢 Difficulty: Trivial (100 Points)

Current validation errors are raw strings. Improve the formatting in Kubernetes Events and logs to be more user-friendly.

### ✅ Acceptance Criteria
- Use a structured error format for validation failures.
- Group multiple validation errors into a single Kubernetes Event.
- Add clear \"How-to-fix\" suggestions in messages.

### 📚 Resources
- [Rust Anyhow/Thiserror](https://github.com/dtolnay/anyhow)
- [\`src/error.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/error.rs)
" --label "stellar-wave,good-first-issue,logic"

echo "Done! Batch 4 issues created (#29-#38)."
