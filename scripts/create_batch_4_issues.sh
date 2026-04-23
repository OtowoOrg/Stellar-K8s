#!/bin/bash
set -e

# Stellar-K8s Wave Issue Creation Script - BATCH 4
# 6 High (200 pts), 2 Medium (150 pts), 2 Trivial (100 pts)

# Source shared retry/backoff and dry-run helper.
# shellcheck source=scripts/retry_helper.sh
source "$(dirname "$0")/retry_helper.sh"

echo "Creating Batch 4 (Mixed) issues..."

# --- HIGH (200 pts) ---

# 29. Chaos Mesh Integration (High - 200 pts)
create_issue \
  "Integrate Chaos Mesh for Network Partition Testing" \
  "stellar-wave,reliability,architecture" \
  "### 🔴 Difficulty: High (200 Points)

To ensure the operator handles fragile network conditions gracefully, we need to integrate Chaos Mesh. This task involves creating automated chaos scenarios to test node recovery during network partitions.

### ✅ Acceptance Criteria
- Create tests/chaos/ directory with Chaos Mesh manifests (NetworkChaos).
- Implement a test script that triggers a partition and verifies the Operator's recovery logic.
- Document the Failure Mode and Effects Analysis (FMEA) for StellarNode.

### 📚 Resources
- [Chaos Mesh Documentation](https://chaos-mesh.org/docs/simulate-network-chaos-on-kubernetes/)
"

# 30. Dynamic Peer Discovery (High - 200 pts)
create_issue \
  "Implement Dynamic Peer Discovery Controller" \
  "stellar-wave,architecture,logic" \
  "### 🔴 Difficulty: High (200 Points)

Currently, peers are defined statically. We need a controller that dynamically discovers other StellarNode resources in the cluster and updates the KNOWN_PEERS configuration in real-time.

### ✅ Acceptance Criteria
- Implement a watcher for StellarNode resources.
- Automatically update a shared ConfigMap with the latest peer IPs/Ports.
- Trigger a rolling update or signal the Stellar process to refresh configuration.

### 📚 Resources
- [Stellar Core Peers Config](https://github.com/stellar/stellar-core/blob/master/docs/stellar-core_example.cfg)
- [kube-rs Runtime Watcher](https://kube.rs/controllers/watcher/)
"

# 31. Multi-Cluster Support (High - 200 pts)
create_issue \
  "Add Multi-Cluster Orchestration Support" \
  "stellar-wave,architecture,kubernetes" \
  "### 🔴 Difficulty: High (200 Points)

Large Stellar deployments should span multiple Kubernetes clusters. This task involves updating the operator to support cross-cluster communication and synchronization.

### ✅ Acceptance Criteria
- Add cluster: String field to StellarNodeSpec.
- Support ExternalName services or service-mesh (Submariner/Istio) for cross-cluster DNS.
- Implement logic to handle node latency thresholds between clusters.

### 📚 Resources
- [Submariner Multi-cluster Networking](https://submariner.io/)
"

# 32. Auto-Remediation for Stale Ledgers (High - 200 pts)
create_issue \
  "Implement Auto-Remediation for Stale/Desynced Nodes" \
  "stellar-wave,reliability,logic" \
  "### 🔴 Difficulty: High (200 Points)

If a node gets stuck or significantly behind the network, it may need an automated restart or a fresh sync. The operator should detect this and perform safe remediation.

### ✅ Acceptance Criteria
- Detect stale state (ledger height not increasing for X minutes).
- Implement automated remediation steps: Restart -> Clear DB -> Fresh Sync.
- Emit Kubernetes Events for every automated action taken.

### 📚 Resources
- [Monitoring Stellar Core](https://developers.stellar.org/docs/run-core-node/monitoring)
"

# 33. Cloud KMS/HSM Integration (High - 200 pts)
create_issue \
  "Implement Cloud KMS/HSM Integration for Node Keys" \
  "stellar-wave,security,architecture" \
  "### 🔴 Difficulty: High (200 Points)

Storing node keys in plain Kubernetes Secrets is not sufficient for high-security environments. Integrate with cloud-native KMS (AWS KMS, Google Cloud KMS) or HSMs.

### ✅ Acceptance Criteria
- Support keySource: KMS in the spec.
- Implement an InitContainer that fetches and decrypts keys from a Vault/KMS.
- Ensure keys never touch the disk in plaintext.

### 📚 Resources
- [Stellar Node Security](https://developers.stellar.org/docs/run-core-node/security-best-practices)
"

# 34. OpenTelemetry Tracing (High - 200 pts)
create_issue \
  "Add OpenTelemetry Tracing Support" \
  "stellar-wave,observability,rust" \
  "### 🔴 Difficulty: High (200 Points)

Debugging complex operator logic requires distributed tracing. Implement OpenTelemetry support throughout the controller and API.

### ✅ Acceptance Criteria
- Integrate opentelemetry-rs.
- Trace reconciliation loops and resource patching actions.
- Export traces to a configurable OTLP endpoint (Jaeger/Tempo).

### 📚 Resources
- [OpenTelemetry Rust](https://github.com/open-telemetry/opentelemetry-rust)
"

# --- MEDIUM (150 pts) ---

# 35. mTLS for Node-to-Node Communication (Medium - 150 pts)
create_issue \
  "Implement mTLS for Internal Node Communication" \
  "stellar-wave,security,feature" \
  "### 🟡 Difficulty: Medium (150 Points)

Secure the traffic between Stellar nodes and the Operator REST API using mutual TLS (mTLS).

### ✅ Acceptance Criteria
- Automate certificate distribution to pods.
- Enable mTLS verification in the rest_api module.
- Provide a CLI flag to enable/disable strict mTLS.

### 📚 Resources
- [mTLS Explained](https://www.cloudflare.com/learning/access-management/what-is-mutual-tls/)
"

# 36. Canary Rollouts with Traffic Weighting (Medium - 150 pts)
create_issue \
  "Support Canary Rollouts with Traffic Weighting" \
  "stellar-wave,kubernetes,feature" \
  "### 🟡 Difficulty: Medium (150 Points)

When upgrading Horizon or Soroban RPC, we should support canary deployments where only a percentage of traffic hits the new version.

### ✅ Acceptance Criteria
- Add strategy: Canary to the spec with a weight field.
- Update Ingress annotations to support traffic splitting (Nginx/Istio/Traefik).
- Implement automated rollback if health checks fail.

### 📚 Resources
- [Canary Deployments on Kubernetes](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/#canary-deployment)
"

# --- TRIVIAL (100 pts) ---

# 37. CLI Version and Info Subcommands (Trivial - 100 pts)
create_issue \
  "Add 'version' and 'info' subcommands to CLI" \
  "stellar-wave,good-first-issue,rust" \
  "### 🟢 Difficulty: Trivial (100 Points)

Provide users with a way to check the operator version, build date, and basic cluster status via the binary.

### ✅ Acceptance Criteria
- Implement version subcommand using clap.
- Implement info subcommand showing current managed Node count.
- Print build metadata (Git SHA, Rust version).

### 📚 Resources
- [Clap (Rust) Documentation](https://docs.rs/clap/latest/clap/)
"

# 38. Improved CRD Validation Formatting (Trivial - 100 pts)
create_issue \
  "Improve CRD Validation Error Formatting" \
  "stellar-wave,good-first-issue,logic" \
  "### 🟢 Difficulty: Trivial (100 Points)

Current validation errors are raw strings. Improve the formatting in Kubernetes Events and logs to be more user-friendly.

### ✅ Acceptance Criteria
- Use a structured error format for validation failures.
- Group multiple validation errors into a single Kubernetes Event.
- Add clear How-to-fix suggestions in messages.

### 📚 Resources
- [Rust Anyhow/Thiserror](https://github.com/dtolnay/anyhow)
"

echo "Done! Batch 4 issues created (#29-#38)."
