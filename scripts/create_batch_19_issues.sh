#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/repo.sh"
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 19, 12 x 200 pts).

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

EXPECTED_ISSUE_COUNT=12
ACTUAL_ISSUE_COUNT=$(grep -c '^create_issue_with_retry' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT. Update EXPECTED_ISSUE_COUNT or fix the script." >&2
  exit 1
fi

echo "Creating Batch 19 (12 x 200 pts) issues with auto-retry..."

# ─── 1 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement Automated Rollback for Failed Validator Upgrades" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Provide a safety mechanism that automatically reverts a Stellar Core validator pod to its previous image version if an upgrade fails to reach a 'Synced' state within a specified timeout.

### 📋 Context
Upgrading validator nodes carries risk. If a new version of Stellar Core crashes or fails to attain consensus, manual operator intervention is currently required to edit the CRD and roll back. This delay can lead to degraded network participation.

### ✅ Acceptance Criteria
- Extend the \`StellarNodeSpec\` to include a \`rollbackConfiguration\` block (timeout, max retries).
- The reconciler must cache the *previous* known-good \`StellarNodeSpec\` in a K8s Secret or annotation before applying an update.
- Implement a monitor loop that tracks the node's phase post-upgrade.
- If the pod crashes repeatedly or fails to sync within the timeout, the operator must automatically revert the Deployment/StatefulSet to the cached spec.
- Emit a Kubernetes Event of type \`Warning\` indicating an \`UpgradeReverted\` occurred.

### 📚 Resources
- [Kubernetes Rollback Documentation](https://kubernetes.io/docs/concepts/workloads/controllers/deployments/#rolling-back-a-deployment)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)"


# ─── 2 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Hardening: Implement Cilium NetworkPolicies for Zero-Trust Operator Isolation" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Create advanced L4/L7 NetworkPolicies using Cilium to strictly limit the inbound and outbound traffic of both the Stellar Operator and managed Stellar nodes, enforcing a zero-trust model.

### 📋 Context
Default Kubernetes networking allows 'any-to-any' pod communication. In a high-security financial infrastructure, the operator should only communicate with the K8s API server, and validators should only communicate with defined peer IPs and the archive storage endpoints.

### ✅ Acceptance Criteria
- Create CiliumNetworkPolicy (CNP) YAML manifests in the \`charts/\` directory.
- For the Operator: Restrict egress strictly to the Kubernetes API server and configured external webhook endpoints.
- For Validators: Restrict egress to ports 11625 (Stellar P2P) and specific CIDR blocks for S3/GCS archive access.
- For Horizon: Allow ingress only on port 8000 from the designated Ingress controller, and egress to the internal Postgres DB and Core instances.
- Include a testing guide using \`cilium connectivity test\`.

### 📚 Resources
- [Cilium Network Policy Guide](https://docs.cilium.io/en/stable/network/concepts/policy/)
- [Kubernetes Network Policies](https://kubernetes.io/docs/concepts/services-networking/network-policies/)
- [\`charts/stellar-operator/templates/\`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/charts/stellar-operator/templates)"


# ─── 3 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Develop Load Shedding and Circuit Breaker Middleware for the Operator REST API" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Protect the operator's internal Axum REST API from denial-of-service (DoS) or accidental high-volume requests by implementing robust load shedding and circuit-breaking middleware.

### 📋 Context
As the operator's REST API is exposed for debugging, dashboards, and potentially external integrations, a sudden spike in requests (e.g., fetching metrics or status for 1000 nodes simultaneously) could starve the main reconciliation loop of CPU or memory, causing the control plane to stall.

### ✅ Acceptance Criteria
- Integrate \`tower::limit::GlobalConcurrencyLimit\` or similar rate-limiting middleware into the Axum server setup in \`src/rest_api/server.rs\`.
- Implement a circuit breaker that rejects API requests with HTTP 503 (Service Unavailable) if the Kubernetes client reports a high error rate or timeout when interacting with the K8s API server.
- Expose Prometheus metrics tracking rejected requests (\`operator_api_rejected_total\`).
- Ensure the \`/healthz\` and \`/readyz\` endpoints bypass the concurrency limits.

### 📚 Resources
- [Tower Middleware Documentation](https://docs.rs/tower/latest/tower/)
- [Axum Middleware Guide](https://docs.rs/axum/latest/axum/middleware/index.html)
- [\`src/rest_api/server.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/rest_api/server.rs)"


# ─── 4 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar audit' Command to Verify CRD Compliance against Security Baselines" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Provide a CLI tool that scans all deployed \`StellarNode\` resources in a cluster and reports violations against a predefined \"High Security Baseline\" (e.g., missing resource limits, root privileges enabled, plain-text env vars).

### 📋 Context
Security teams need an easy way to verify that infrastructure teams are deploying Stellar nodes securely. Reviewing raw K8s YAML is error-prone. We need a tool that specifically understands the \`StellarNode\` CRD context.

### ✅ Acceptance Criteria
- Add the \`audit\` subcommand to the \`stellar-operator\` binary (or \`kubectl-stellar\` plugin).
- The tool must fetch all \`StellarNode\` instances via the K8s API.
- Implement checks for: 
  1. \`securityContext.runAsNonRoot == true\`
  2. \`resources.limits\` are defined for CPU and Memory
  3. No sensitive data (like DB passwords or validator seeds) are stored directly in the spec instead of using \`Secret\` references.
- Output a colored, human-readable report indicating PASS/FAIL for each rule per node.

### 📚 Resources
- [kube-rs Documentation](https://kube.rs/)
- [Kubernetes Security Context](https://kubernetes.io/docs/tasks/configure-pod-container/security-context/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 5 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Add Support for AWS KMS and HashiCorp Vault for Validator Seed Injection" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Extend the operator to allow validator seeds to be securely referenced directly from AWS KMS or HashiCorp Vault, bypassing standard Kubernetes Secrets entirely.

### 📋 Context
Kubernetes Secrets are only base64 encoded by default. For Tier-1 validators, storing the seed key in etcd—even encrypted—is considered a risk. Direct integration with an external Key Management System ensures the key material is only ever in memory within the Stellar Core pod.

### ✅ Acceptance Criteria
- Update the \`ValidatorConfig\` to support a \`kmsRef\` field (e.g., \`provider: Vault\`, \`path: secret/data/stellar/seed\`).
- If an external KMS is referenced, the operator should NOT attempt to read a K8s secret.
- Instead, the operator should inject a sidecar container (or modify the \`stellar-core\` entrypoint script) that fetches the seed from the KMS at startup using the pod's IAM role or ServiceAccount token (via Vault K8s Auth).
- Provide detailed documentation (\`docs/kms-integration.md\`) outlining the required AWS IAM or Vault policies.

### 📚 Resources
- [HashiCorp Vault Kubernetes Auth](https://developer.hashicorp.com/vault/docs/auth/kubernetes)
- [AWS KMS for Rust SDK](https://github.com/awslabs/aws-sdk-rust)
- [\`src/crd/types.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/types.rs)"


# ─── 6 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement Real-Time Validation Webhook for Quorum Set Safey" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Create a Kubernetes Validating Webhook that prevents users from submitting a \`StellarNode\` CRD update if the proposed Quorum Set configuration is mathematically unsafe (e.g., lacking enough Tier 1 nodes to maintain liveness and safety).

### 📋 Context
Misconfiguring a quorum set can cause a validator to halt or, worse, fork. While Stellar Core performs some checks at runtime, the operator should 'shift-left' and proactively reject K8s manifests that define obviously bad quorum configurations before they are ever applied.

### ✅ Acceptance Criteria
- Implement a new Axum endpoint (e.g., \`/validate-quorum\`) configured as a K8s \`ValidatingWebhookConfiguration\`.
- Parse the \`validator_config.quorum_set\` TOML/JSON string inside the \`StellarNode\` spec during the validation phase.
- Implement basic graph analysis to ensure the quorum set definition doesn't rely entirely on a single point of failure (e.g., a 1-of-1 threshold).
- Return an AdmissionResponse rejecting the Apply/Create operation with a detailed error message if the configuration is deemed unsafe.
- Write unit tests covering various safe and unsafe quorum topologies.

### 📚 Resources
- [Kubernetes Admission Webhooks](https://kubernetes.io/docs/reference/access-authn-authz/extensible-admission-controllers/)
- [Stellar Quorum Set Documentation](https://developers.stellar.org/docs/run-core-node/quorums)
- [\`src/webhook/types.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/webhook/types.rs)"


# ─── 7 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Design and Implement 'Tiered Storage' Strategy for History Archives using K8s Volumes" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Optimize storage costs and performance for Archiving Validators by having the operator provision separate Kubernetes volumes: a high-IOPS SSD volume for the current ledger database, and a cheaper, high-capacity HDD volume for the history archive staging area.

### 📋 Context
Stellar validators that publish history archives need significant storage. Using premium NVMe/SSD storage for terabytes of immutable history archives is an expensive waste of cloud resources, but the core database *requires* high IOPS.

### ✅ Acceptance Criteria
- Modify the \`StorageConfig\` in the \`StellarNode\` CRD to support multiple volume definitions (e.g., \`dbStorage\` vs \`archiveStorage\`).
- The operator must create multiple PVCs with different \`storageClass\` annotations based on this configuration.
- Update the \`ensure_statefulset\` builder logic in \`src/controller/resources.rs\` to mount these distinct PVCs to the correct paths inside the Stellar Core container (e.g., \`/var/lib/stellar/db\` vs \`/var/lib/stellar/history\`).
- Add tests verifying the correct volume mounts are generated.

### 📚 Resources
- [Kubernetes Storage Classes](https://kubernetes.io/docs/concepts/storage/storage-classes/)
- [Kubernetes Persistent Volumes](https://kubernetes.io/docs/concepts/storage/persistent-volumes/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)"


# ─── 8 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Build an Automated 'Chaos Testing' Harness for the Controller" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Introduce a dedicated end-to-end chaos testing suite that randomly kills K8s API servers, operator pods, and managed Stellar node pods to guarantee the reconciler always recovers smoothly without operator intervention.

### 📋 Context
Kubernetes operators must be completely resilient to environmental failures. We need empirical proof that if the K8s API goes down mid-reconciliation, or the operator pod is OOM-killed while creating a StatefulSet, the system naturally self-heals when services are restored.

### ✅ Acceptance Criteria
- Create a new script \`scripts/chaos-test.sh\` that spins up a local \`k3d\` or \`kind\` cluster and deploys the operator.
- Write a test runner (Bash or Rust) that constantly submits updates to a \`StellarNode\` CRD.
- Concurrently use \`kubectl delete pod\` to randomly assassinate the operator pod and the Stellar Core pods every few seconds.
- Assert that after 5 minutes of chaos, once the assassinations stop, the deployed \`StellarNode\` accurately matches the desired state defined in the CRD within 60 seconds.

### 📚 Resources
- [Chaos Mesh Documentation](https://chaos-mesh.org/docs/)
- [Kubernetes Pod Deletion](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/#pod-termination)
- [\`scripts/soak-test.sh\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/soak-test.sh)"


# ─── 9 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement 'stellar topology' Command to visualize the Fleet Deployment Pattern" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Build a CLI tool that queries the Kubernetes cluster and prints out an ASCII-art or Graphviz representation of how Stellar nodes, Horizon servers, and Soroban RPC nodes are connected and distributed across cluster nodes and availability zones.

### 📋 Context
Understanding the blast radius of a K8s node going down or an AWS Availability Zone failing requires piecing together \`kubectl get pods -o wide\` and node labels. A visual representation of the topology specific to Stellar infrastructure is invaluable for DevOps.

### ✅ Acceptance Criteria
- Add the \`topology\` subcommand to the operator or kubectl plugin.
- The command must query K8s to map \`StellarNode\` pods to underlying K8s Nodes, and extract the \`topology.kubernetes.io/zone\` labels.
- Render a hierarchical view showing which Horizon/Soroban pods depend on which Core pods, and what Availability Zone they physically reside in.
- Highlight any Single Points of Failure (e.g., \"Warning: All 3 Horizon Replicas are scheduled in us-east-1a\").

### 📚 Resources
- [Kubernetes Pod Listing API](https://kubernetes.io/docs/reference/kubernetes-api/workload-resources/pod-v1/#list-list-or-watch-objects-of-kind-pod)
- [Well-known Labels, Annotations and Taints](https://kubernetes.io/docs/reference/labels-annotations-taints/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 10 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Integrate Kyverno Policies for Enforcing Stellar-K8s Best Practices" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Develop and package a suite of Kyverno policies specifically designed to enforce operational and security best practices for the \`StellarNode\` CRD.

### 📋 Context
While Validating Webhooks (like Quorum checks) handle complex internal logic, Kyverno provides a cleaner, Kubernetes-native declarative way to enforce broader organizational policies (like requiring specific tags, preventing the use of \`latest\` tags for images, or ensuring persistent volumes are used).

### ✅ Acceptance Criteria
- Create a \`policy/\` directory containing Kyverno \`ClusterPolicy\` YAML definitions.
- Implement policies that:
  1. Disallow the \`latest\` tag in the \`.spec.version\` field to ensure deterministic deployments.
  2. Require a specific set of organizational labels (e.g., \`cost-center\`, \`owner\`) on all \`StellarNode\` manifests.
  3. Ensure that Validator nodes aren't configured to use ephemeral \`emptyDir\` storage.
- Document how to install Kyverno and apply these policies in \`docs/governance.md\`.

### 📚 Resources
- [Kyverno Policies Documentation](https://kyverno.io/docs/writing-policies/)
- [Stellar-K8s Governance Guide](https://github.com/OtowoOrg/Stellar-K8s/blob/main/docs/governance.md)"


# ─── 11 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Implement Advanced 'Blue/Green' Upgrade Strategy for RPC Nodes" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Provide native support within the operator for conducting zero-downtime, blue/green deployments specifically for Horizon and Soroban RPC nodes when updating versions or configurations.

### 📋 Context
A standard Kubernetes Deployment 'RollingUpdate' strategy works, but for critical API infrastructure, operators often prefer to spin up a completely separate 'Green' cluster of RPC nodes, run smoke tests against them, and then rapidly switch traffic at the load balancer/service level, rather than mixing old and new versions during a rollout.

### ✅ Acceptance Criteria
- Extend the CRD with a \`deploymentStrategy\` block for RPC nodes (supporting \`RollingUpdate\` or \`BlueGreen\`).
- When \`BlueGreen\` is selected, modifying the \`version\` should cause the operator to create a completely new Deployment with a unique suffix, while keeping the old one running.
- The operator should wait for the new Deployment to report Ready, and then patch the K8s \`Service\` selector to point to the new pods in a single operation.
- Implement cleanup logic to delete the 'Blue' (old) deployment after a successful switch.

### 📚 Resources
- [Blue/Green Deployment Strategy](https://martinfowler.com/bliki/BlueGreenDeployment.html)
- [Kubernetes Service Selector](https://kubernetes.io/docs/concepts/services-networking/service/#defining-a-service)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)"


# ─── 12 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry "Develop a Comprehensive 'Runbook' Generation Tool for Support Teams" "stellar-wave,documentation,dx" "### 🔴 Difficulty: High (200 Points)

Create a feature that dynamically generates a Markdown or HTML 'Runbook' tailored to the specific configuration of a deployed \`StellarNode\`, providing L1 support teams with instant, context-aware troubleshooting steps.

### 📋 Context
Generic documentation only goes so far. When a specific Validator configured with AWS KMS and S3 archiving goes down, the on-call engineer needs troubleshooting commands tailored to *that exact setup*. 

### ✅ Acceptance Criteria
- Add a \`generate-runbook\` subcommand to the CLI that accepts a \`StellarNode\` name as an argument.
- The tool reads the CRD and generates a customized document containing:
  - Exact \`kubectl\` commands to fetch logs from the specific DB and Core containers.
  - Links or queries to check the specific KMS key status if KMS is configured.
  - S3/GCS CLI commands to verify the specific archive bucket if archiving is enabled.
- The generated runbook must clearly state the network (Mainnet/Testnet) and list the expected peer connections based on the quorum set.

### 📚 Resources
- [Stellar Core Troubleshooting](https://developers.stellar.org/docs/run-core-node/troubleshooting)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)
- [\`src/crd/types.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/types.rs)"


echo ""
echo "🎉 Batch 19 (12 x 200 pts) issues created successfully! Backlog depth++"

print_skip_summary
