#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
source "$(dirname "$0")/lib/repo.sh"

# Stellar-K8s Wave Issue Creation Script - BATCH 6
# 10 Elite Engineering Issues (200 pts each)

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 6, 10 x 200 pts).

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

# Helper to create label if not exists
create_label() {
  gh label create --repo "$REPO" "$1" --color "$2" --description "$3" || true
}

echo "Ensuring labels exist..."
create_label "stellar-wave" "1d76db" "Stellar Wave Program"
create_label "architecture" "0e8a16" "Architecture design"
create_label "reliability" "d93f0b" "Reliability and stability"
create_label "security" "d73a4a" "Security related"
create_label "kubernetes" "326ce5" "Kubernetes related"
create_label "performance" "bfd4f2" "Performance optimizations"
create_label "automation" "ffb3b3" "Automated workflows"

echo "Creating Batch 6 (Elite) issues..."

# 1. Cross-Region Multi-Cluster Disaster Recovery (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Cross-Region Multi-Cluster Disaster Recovery" \
  --body "### 🔴 Difficulty: High (200 Points)

Standard backups are not enough. This task involves building a controller that manages a 'hot standby' node in a completely different Kubernetes cluster (and region), ensuring minimal RTO/RPO for the Stellar network.

### ✅ Acceptance Criteria
- Implement cross-cluster state synchronization logic.
- Automated failover mechanism using external DNS (Route53/Cloudflare).
- Verify data consistency during regional partition scenarios.

### 📚 Resources
- [Stellar High Availability](https://developers.stellar.org/docs/run-core-node/monitoring#high-availability)
- [Multi-cluster K8s with Submariner](https://submariner.io/)
" --label "stellar-wave,architecture,reliability" || echo "Failed issue 1"

# 2. MetalLB/BGP Anycast for Node Discovery (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Integrate MetalLB/BGP Anycast for Global Node Discovery" \
  --body "### 🔴 Difficulty: High (200 Points)

To make Stellar nodes truly resilient, we should announce node IPs via BGP Anycast. This allows the network to route traffic to the nearest healthy node automatically.

### ✅ Acceptance Criteria
- Integration with MetalLB or Cilium BGP Control Plane.
- Automated IP allocation and BGP advertisement per StellarNode.
- Support for health-aware route withdrawal.

### 📚 Resources
- [MetalLB BGP Mode](https://metallb.universe.tf/concepts/bgp/)
- [Cilium BGP Control Plane](https://docs.cilium.io/en/stable/network/bgp-control-plane/)
" --label "stellar-wave,kubernetes,reliability" || echo "Failed issue 2"

# 3. CloudNativePG (Postgres Operator) Integration (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Automate High-Availability DBs via CloudNativePG Integration" \
  --body "### 🔴 Difficulty: High (200 Points)

Currently, DB management is manual or basic. This task involves integrating the Stellar-K8s operator with the CloudNativePG operator to manage self-healing, HA Postgres clusters for Horizon and Core.

### ✅ Acceptance Criteria
- Provision \`Cluster\` resources (CNPG) instead of simple Deployments.
- Automated backup/restore integration with CNPG Barman.
- Connection pooling support (PgBouncer).

### 📚 Resources
- [CloudNativePG Documentation](https://cloudnative-pg.io/documentation/1.22/)
" --label "stellar-wave,architecture,automation" || echo "Failed issue 3"

# 4. Hardware Security Module (HSM) Native Support (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Native Hardware Security Module (HSM) Support" \
  --body "### 🔴 Difficulty: High (200 Points)

Validators require the highest level of key security. This task implements native integration for cloud HSMs (AWS CloudHSM, Azure Dedicated HSM) to sign transactions without keys ever leaving the secure module.

### ✅ Acceptance Criteria
- Sidecar/InitContainer implementation for PKCS#11 integration.
- Secure key-loading into Stellar Core memory from HSM.
- Automated HSM health monitoring.

### 📚 Resources
- [Stellar Core Security](https://developers.stellar.org/docs/run-core-node/security-best-practices)
" --label "stellar-wave,security,architecture" || echo "Failed issue 4"

# 5. Automated Performance Regression Testing (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Automated Performance Regression Test Suite" \
  --body "### 🔴 Difficulty: High (200 Points)

As the operator grows, performance can degrade. Implement an automated benchmarking suite that measures TPS, latency, and resource consumption for every new release.

### ✅ Acceptance Criteria
- Load-testing framework integration (k6 or Locust).
- Comparative analysis of metrics between release versions.
- Block CI/CD if performance regressions > 10%.

### 📚 Resources
- [k6 for Kubernetes](https://k6.io/docs/testing-guides/running-k6-on-kubernetes/)
" --label "stellar-wave,performance,testing" || echo "Failed issue 5"

# 6. Wasm-based Validating Admission Webhook (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Wasm-powered Validating Admission Webhook" \
  --body "### 🔴 Difficulty: High (200 Points)

Standard validation is static. Implement a webhook that allows users to provide custom Wasm-based validation logic for their \`StellarNode\` resources (e.g., complex infrastructure constraints).

### ✅ Acceptance Criteria
- Rust-based admission controller integration.
- Wasm runtime (Wasmtime) integration to execute external plugins.
- Secure plugin isolated execution environment.

### 📚 Resources
- [Kubernetes Admission Controllers](https://kubernetes.io/docs/reference/access-authn-authz/admission-controllers/)
" --label "stellar-wave,architecture,rust" || echo "Failed issue 6"

# 7. Zero-Knowledge Telemetry Proxy (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Zero-Knowledge Telemetry Proxy" \
  --body "### 🔴 Difficulty: High (200 Points)

Nodes need to report health, but privacy is key. Implement a proxy that scrubs sensitive metadata (IPs, cluster names) from telemetry before sending it to public dashboards or monitoring servers.

### ✅ Acceptance Criteria
- Integration with OpenTelemetry Collector for scrubbing.
- Differential privacy implementation for reported counts.
- End-to-end encryption for telemetry data.

### 📚 Resources
- [Stellar Dashboard](https://dashboard.stellar.org/)
" --label "stellar-wave,security,observability" || echo "Failed issue 7"

# 8. Automated Rolling Security Patching logic (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Automated Rolling Security Patching" \
  --body "### 🔴 Difficulty: High (200 Points)

When CVEs are found in standard images, the operator should automatically trigger a rolling update to the patched version, but only after passing automated smoke tests on a canary node.

### ✅ Acceptance Criteria
- Integration with image registry scanners (Trivy/Grype).
- Automated version increment and rollout logic.
- Rollback logic if the patched version impacts consensus health.

### 📚 Resources
- [ArgoCD Rollouts](https://argoproj.github.io/argo-rollouts/)
" --label "stellar-wave,security,automation" || echo "Failed issue 8"

# 9. Stellar Core Horizontal 'Read-Only' Scaling (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Horizontal Scaling for Read-Only Nodes" \
  --body "### 🔴 Difficulty: High (200 Points)

While validators are sensitive, read-only nodes can be scaled horizontally. Implement a separate controller/logic to manage auto-scaling pools of Read-Only Stellar nodes.

### ✅ Acceptance Criteria
- Separate Spec for Read-Only replica pools.
- Weighted load-balancing between fresh nodes and lagging nodes.
- Automated shard-balancing for very large history archives.

### 📚 Resources
- [Stellar Core Scaling](https://developers.stellar.org/docs/run-core-node/monitoring#scaling)
" --label "stellar-wave,architecture,performance" || echo "Failed issue 9"

# 10. Custom Scheduler for Data Proximity (High - 200 pts)
gh issue create --repo "$REPO" \
  --title "Implement Custom Scheduler for Data/Peer Proximity" \
  --body "### 🔴 Difficulty: High (200 Points)

Latency between peers is critical. Implement a custom Kubernetes scheduler (or scheduler-plugin) that places pods based on network proximity to other high-value peers.

### ✅ Acceptance Criteria
- Scheduler plugin implementation (Rust/GO).
- Latency-aware node selection for pod placement.
- Integration with Kubernetes Network Topology API.

### 📚 Resources
- [Kubernetes Scheduler Plugins](https://kubernetes.io/docs/concepts/scheduling-eviction/scheduling-framework/)
" --label "stellar-wave,architecture,performance" || echo "Failed issue 10"

echo "Done! Batch 6 Elite issues created."
