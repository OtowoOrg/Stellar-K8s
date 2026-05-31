#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

echo "Creating Batch 20 (15x200, 5x150, 5x100 pts) issues..."

function create_issue_with_retry() {
  local title="$1"
  local label="$2"
  local body="$3"
  
  local max_retries=5
  local count=0
  
  while [ $count -lt $max_retries ]; do
    if gh issue create --repo "$REPO" --title "$title" --label "$label" --body "$body"; then
      echo "✓ Issue created: $title"
      return 0
    else
      count=$((count + 1))
      echo "API failed, retrying ($count/$max_retries) in 10 seconds..."
      sleep 10
    fi
  done
  
  echo "Failed to create issue after $max_retries attempts: $title"
  exit 1
}

# --- 200 POINT ISSUES (1-15) ---

create_issue_with_retry "Implement Multi-Cluster Ledger Replication for Disaster Recovery" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Provide a mechanism to replicate ledger data across geographically dispersed Kubernetes clusters to ensure zero data loss in the event of a total region failure.

### 📋 Context
For mission-critical financial infrastructure, relying on a single cloud region or cluster is a single point of failure. We need to automate the cross-cluster replication of both the Postgres DB and the History Archives.

### ✅ Acceptance Criteria
- Implement a \`replicationConfiguration\` in the \`StellarNode\` CRD.
- Support asynchronous replication to a secondary 'Passive' cluster.
- Automate the setup of cross-cluster VPN/Peering requirements via documentation or CRD fields.
- Provide a CLI command to trigger a failover to the secondary cluster."

create_issue_with_retry "Develop Custom Kubernetes Scheduler for Validator Proximity Optimization" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Build a specialized Kubernetes scheduler plugin that prioritizes placing Stellar Core validator pods on nodes with low network latency to their primary quorum peers.

### 📋 Context
SCP consensus depends on rapid message exchange. Placing peer validators on the same rack or availability zone can significantly decrease ledger close times.

### ✅ Acceptance Criteria
- Implement a K8s scheduler plugin (or use Scheduling Gates).
- The scheduler must query peer latency metrics (from Prometheus) to make placement decisions.
- Add a \`proximityAware\` flag to the CRD.
- Document the performance improvements in a benchmark report."

create_issue_with_retry "Auto-Scaling Soroban RPC Pods based on WASM VM Execution Metrics" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Implement a Horizontal Pod Autoscaler (HPA) strategy that scales Soroban RPC nodes based on internal WASM execution time and memory pressure, rather than just generic CPU/Memory.

### 📋 Context
Soroban transactions are execution-heavy. A node might be CPU-idle but throttled by WASM VM limits. Standard HPAs don't capture this.

### ✅ Acceptance Criteria
- Export custom Prometheus metrics for WASM execution latency from the RPC nodes.
- Configure a \`Custom Metrics Adapter\` for Kubernetes.
- Implement an HPA template in the Helm chart that targets these custom metrics.
- Verify scaling behavior under heavy Soroban contract load."

create_issue_with_retry "Integrate ExternalDNS for Automated Stellar Peer Discovery management" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Automate the management of DNS \`A\` and \`SRV\` records for Stellar peers using \`ExternalDNS\`, ensuring that the network remains discoverable as pods are rescheduled.

### 📋 Context
Manual DNS management is error-prone. As validators move between nodes, their public IP or LoadBalancer address changes. ExternalDNS can sync these automatically.

### ✅ Acceptance Criteria
- Add support for \`ExternalDNS\` annotations in the Service and Ingress builders.
- Automatically generate \`_stellar-peering._tcp\` SRV records for each validator.
- Ensure TTLs are kept low for rapid convergence during pod restarts.
- Document the setup for AWS Route53 and Google Cloud DNS."

create_issue_with_retry "Implement Canary Deployment Strategy with Traffic Splitting for Horizon" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Enable the operator to perform Canary releases for Horizon, where a small percentage of API traffic is routed to the new version before a full rollout.

### 📋 Context
Upgrading Horizon can occasionally introduce breaking API changes or performance regressions. Canarying allows testing with real traffic with minimal blast radius.

### ✅ Acceptance Criteria
- Integrate with an Ingress Controller (e.g., Nginx or Istio) that supports traffic weighting.
- Add a \`canary\` block to the RPC node spec in the CRD.
- The operator must manage two parallel deployments and adjust traffic weights via the Ingress/Service.
- Automate the 'Rollback on Error' if 4xx/5xx rates spike in the canary."

create_issue_with_retry "Build an Integrated Performance Benchmarking Suite in the Operator" "stellar-wave,testing,performance" "### 🔴 Difficulty: High (200 Points)

Develop a suite of performance tests that can be triggered directly via the operator to verify cluster throughput (TPS) and latency under various configurations.

### 📋 Context
Operators need to know how many transactions per second their specific hardware can handle. A built-in benchmark tool makes this verification easy.

### ✅ Acceptance Criteria
- Implement a \`StellarBenchmark\` CRD.
- The operator should spin up ephemeral 'load generator' pods.
- Capture and report: Peak TPS, Average Ledger Close Time, and P99 API Latency.
- Store results in a \`BenchmarkReport\` resource or ConfigMap."

create_issue_with_retry "Support Multi-Network Isolation (Mainnet/Testnet) in a Single Cluster" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Provide strict namespace and network isolation for running Mainnet and Testnet nodes within the same Kubernetes cluster without risk of cross-talk.

### 📋 Context
Cost-conscious teams often share clusters. We need to guarantee that a Testnet node can never accidentally connect to a Mainnet peer or share a database.

### ✅ Acceptance Criteria
- Implement strict NetworkPolicies that block all traffic between designated Mainnet and Testnet namespaces.
- Ensure the operator's RBAC is scoped to prevent cross-namespace resource access where not intended.
- Add a 'Network Safety' check in the reconciler.
- Document the isolation architecture."

create_issue_with_retry "Implement Snapshot-Sync Optimization for Rapid Validator Provisioning" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Develop a mechanism to bootstrap new Stellar Core nodes using pre-computed ZFS/LVM snapshots or compressed DB backups for near-instant synchronization.

### 📋 Context
Joining a validator to a mature network can take days of 'Catch-up'. Using snapshots can reduce this to minutes.

### ✅ Acceptance Criteria
- Add \`snapshotRef\` support to the \`StorageConfig\`.
- The operator should automate the mounting of a snapshot volume or the extraction of a backup before starting the core process.
- Implement an 'Auto-Snapshot' worker that creates periodic backups.
- Verify that nodes can reach 'Synced' state within 10 minutes of creation."

create_issue_with_retry "Develop Byzantine-Resilient Multi-Point Monitoring for SCP Health" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Implement a monitoring system that observes the Stellar network from multiple geographically dispersed vantage points to detect local vs. global consensus issues.

### 📋 Context
A node might think it's in consensus, but it's actually partitioned. Monitoring from a single point (the cluster itself) is insufficient.

### ✅ Acceptance Criteria
- Implement a 'Watcher' sidecar that can be deployed in multiple cloud providers.
- Aggregate 'consensus views' from all watchers in a central Prometheus instance.
- Create an alert that triggers if $>20\%$ of watchers see a different ledger hash.
- Document the 'Byzantine Monitoring' setup."

create_issue_with_retry "Implement OIDC Authentication for the Operator REST API" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Secure the operator's internal REST API using OpenID Connect (OIDC), allowing users to authenticate via GitHub, Google, or Okta.

### 📋 Context
Currently, the REST API is mostly unprotected or relies on network-level security. For production use, we need proper Identity-based access control.

### ✅ Acceptance Criteria
- Integrate an OIDC middleware into the Axum server.
- Support JWT validation against standard providers.
- Implement Role-Based Access Control (RBAC) within the API (e.g., 'Reader' vs 'Admin').
- Add OIDC configuration fields to the operator's config file."

create_issue_with_retry "Implement Zero-Knowledge Verification for Encrypted History Backups" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Develop a system to verify the integrity and 'completeness' of encrypted history archives without requiring the operator to possess the decryption keys.

### 📋 Context
We want to store backups in 'Cold Storage' encrypted, but we still need to know they are valid. Zero-knowledge proofs (or similar cryptographic checks) can verify the file structure.

### ✅ Acceptance Criteria
- Implement a verification worker that checks signed manifests.
- Ensure that every checkpoint is present and the chain of hashes is unbroken.
- Fail the 'Archive Health' check if any gap is detected.
- Maintain 'No-Knowledge' of the actual ledger contents."

create_issue_with_retry "Develop Real-Time SCP Topology Visualization Dashboard" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Build a web-based dashboard (or Grafana plugin) that visualizes the real-time Quorum Set graph and the flow of SCP messages between nodes in the cluster.

### 📋 Context
Understanding SCP status is difficult with text logs. A visual graph showing which nodes are 'voting', 'accepting', and 'confirming' is a powerful debugging tool.

### ✅ Acceptance Criteria
- Create a frontend component (React/D3) that renders the quorum graph.
- Implement a WebSocket stream in the operator to push real-time SCP events.
- Highlight 'Stalled' nodes or 'Weak' slices in the UI.
- Package the dashboard as an optional addon for the Helm chart."

create_issue_with_retry "Implement Hot-Upgrade Support for Stellar Core Peer Connections" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Research and implement a way to upgrade the Stellar Core container without dropping active TCP peer connections (e.g., using socket handoff or FD passing).

### 📋 Context
Every restart drops peer connections, requiring a brief period of re-discovery and re-handshaking. For Tier-1 nodes, zero-interruption upgrades are the gold standard.

### ✅ Acceptance Criteria
- Technical feasibility study on FD passing between containers.
- Implementation of a 'Handoff' sidecar if feasible.
- Ensure no consensus messages are missed during the transition.
- Document the 'Hitless Upgrade' process."

create_issue_with_retry "Build Jurisdictional Compliance Orchestrator for Node Placement" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Implement a placement engine that ensures Stellar nodes are physically located in specific geographical jurisdictions to comply with local financial regulations.

### 📋 Context
Some regulators require that financial data (and the nodes processing it) remain within national borders.

### ✅ Acceptance Criteria
- Add a \`jurisdiction\` field to the \`StellarNode\` CRD.
- Map jurisdictions to K8s node labels (e.g., \`topology.kubernetes.io/region\`).
- Enforce placement via \`nodeAffinity\` and \`Tolerations\`.
- Provide a compliance report showing the physical location of all fleet assets."

create_issue_with_retry "Implement Predictive Auto-Scaling using ML on Ledger Volume Trends" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Integrate with a tool like KEDA or a custom ML worker to predict traffic spikes based on historical ledger volume and pre-emptively scale Horizon nodes.

### 📋 Context
Reactive scaling (scaling *after* load hits) is often too slow for the fast-paced Stellar network. Predictive scaling prepares the infrastructure in advance.

### ✅ Acceptance Criteria
- Implement a data collector that stores ledger volume in a time-series DB.
- Use a basic forecasting model (e.g., Prophet or similar) to predict the next hour's load.
- Adjust HPA \`minReplicas\` dynamically based on the forecast.
- Verify that scaling occurs *before* artificial load spikes are applied."

# --- 150 POINT ISSUES (16-20) ---

create_issue_with_retry "Implement Intelligent PVC Pruning on StellarNode Deletion" "stellar-wave,enhancement,reliability" "### 🟡 Difficulty: Medium (150 Points)

Ensure that PersistentVolumeClaims (PVCs) created by the operator are safely and automatically cleaned up when a \`StellarNode\` is deleted, while preventing accidental data loss.

### 📋 Context
Kubernetes often leaves 'orphaned' PVCs behind when a parent resource is deleted. This wastes storage costs. We need a controlled cleanup process.

### ✅ Acceptance Criteria
- Implement a finalizer on the \`StellarNode\` that manages PVC lifecycle.
- Add a \`reclaimPolicy\` to the CRD (e.g., \`Delete\` vs \`Retain\`).
- If \`Delete\` is set, the operator must delete the PVCs after the pods have terminated.
- Add tests for both 'Delete' and 'Retain' scenarios."

create_issue_with_retry "Add Support for Injecting Custom Sidecars via StellarNode CRD" "stellar-wave,enhancement,dx" "### 🟡 Difficulty: Medium (150 Points)

Allow users to define additional sidecar containers in the \`StellarNode\` specification, enabling custom logging, monitoring, or proxy agents.

### 📋 Context
Users often have internal tools (e.g., Splunk forwarders, custom proxies) that they need to run alongside Stellar Core.

### ✅ Acceptance Criteria
- Add a \`sidecars\` field (array of \`Container\` objects) to the CRD.
- The operator must merge these sidecars into the generated StatefulSet/Deployment.
- Support shared volumes between the main container and sidecars.
- Document how to use sidecars for custom log processing."

create_issue_with_retry "Automate Internal mTLS Certificate Rotation with Cert-Manager" "stellar-wave,enhancement,security" "### 🟡 Difficulty: Medium (150 Points)

Integrate with \`cert-manager\` to automatically issue and rotate the mTLS certificates used for internal communication between Stellar Core and Horizon.

### 📋 Context
Manual certificate management is a major operational burden and a security risk if certificates expire. \`cert-manager\` is the industry standard for automation.

### ✅ Acceptance Criteria
- Support \`Issuer\` or \`ClusterIssuer\` references in the CRD.
- The operator should create \`Certificate\` resources for each node.
- Automatically mount the generated secrets into the pods.
- Implement a 'watch' on the secrets to trigger a configuration reload (or pod restart) when certificates are rotated."

create_issue_with_retry "Develop Comprehensive Networking Troubleshooting Guide for K8s" "stellar-wave,documentation,dx" "### 🟡 Difficulty: Medium (150 Points)

Create a detailed, step-by-step guide and a diagnostic script to help users debug common networking issues like 'Connection Refused', 'No Route to Host', and SCP handshake failures.

### 📋 Context
Networking is the most common source of friction when deploying Stellar on K8s. A structured guide reduces support burden.

### ✅ Acceptance Criteria
- New document \`docs/troubleshooting/networking.md\`.
- Cover: Ingress vs LoadBalancer, NetworkPolicies, CNI-specific issues, and Stellar P2P firewalling.
- Include a bash script \`scripts/debug-network.sh\` that checks connectivity from within a pod."

create_issue_with_retry "Implement Customizable Liveness/Readiness Probes in the CRD" "stellar-wave,enhancement,reliability" "### 🟡 Difficulty: Medium (150 Points)

Allow users to override the default Liveness, Readiness, and Startup probes for Stellar and Horizon containers in the CRD.

### 📋 Context
The default probes might be too aggressive or too lenient for certain environments. Users need control over the timeouts and thresholds.

### ✅ Acceptance Criteria
- Add \`livenessProbe\`, \`readinessProbe\`, and \`startupProbe\` blocks to the CRD.
- Validate the input fields (e.g., \`initialDelaySeconds\`, \`periodSeconds\`).
- Ensure the operator respects these overrides when building the StatefulSet/Deployment.
- Add unit tests for the probe builder logic."

# --- 100 POINT ISSUES (21-25) ---

create_issue_with_retry "Enhance CRD Validation Error Messages for Better DX" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Improve the error messages returned by the Validating Webhook to be more human-readable and provide actionable advice when a CRD is misconfigured.

### 📋 Context
Currently, some validation errors are opaque (e.g., 'Internal error'). Users should see 'Error: Quorum set threshold (3) exceeds number of peers (2)'.

### ✅ Acceptance Criteria
- Audit the \`validation\` module and replace generic errors with specific ones.
- Include the field path in the error message.
- Add 'Hint' text to common failures.
- Verify error readability via \`kubectl apply\` tests."

create_issue_with_retry "Add --dry-run Support to all Stellar-K8s CLI Subcommands" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Implement a \`--dry-run\` flag for all CLI subcommands (e.g., \`stellar audit\`, \`stellar topology\`) that shows what actions would be taken without executing them.

### 📋 Context
Operators want to be certain of the impact of a command before running it on a production cluster.

### ✅ Acceptance Criteria
- Add the \`--dry-run\` flag to the global CLI parser.
- Ensure no state-changing API calls are made when the flag is set.
- Print a summary of 'would-be' actions to stdout.
- Add unit tests for the dry-run logic."

create_issue_with_retry "Update Documentation for Local Dev Environment using Minikube" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Update the 'Getting Started' guide to include specific instructions for running the Stellar-K8s operator on Minikube, including driver selection and resource requirements.

### 📋 Context
Minikube is the most popular local K8s tool, but it has specific quirks (like the need for \`minikube tunnel\` for LoadBalancers) that should be documented.

### ✅ Acceptance Criteria
- Update \`docs/getting-started.md\`.
- Include a 'Minikube' section with recommended CPU/Memory settings.
- Document how to handle persistent volumes on Minikube.
- Verify the steps on a fresh Minikube installation."

create_issue_with_retry "Apply Stellar-K8s Branding and Logo to Documentation Site" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Integrate the official Stellar-K8s logo and brand colors (vibrant blue/purple gradients) into the documentation site and CLI help text.

### 📋 Context
A professional appearance builds trust. The current documentation uses default styles.

### ✅ Acceptance Criteria
- Add the logo to the header of the documentation site.
- Update CSS to match the 'Stellar Wave' color palette.
- Add a colored 'Stellar-K8s' banner to the CLI's \`--help\` output.
- Ensure the branding is consistent across all pages."

create_issue_with_retry "Implement CLI Version Check and Upgrade Notification System" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Add a background check to the CLI that notifies the user if a newer version of the operator or CLI is available on GitHub.

### 📋 Context
Users often run outdated versions without realizing it. A gentle notification helps keep the fleet up to date with the latest security fixes.

### ✅ Acceptance Criteria
- Fetch the latest release version from the GitHub API (cached for 24h).
- Compare with the local version.
- If an update is available, print a non-intrusive message to stderr after command execution.
- Include an \`--offline\` flag to disable this check."

echo ""
echo "🎉 Batch 20 (25 issues) created successfully! 15x200, 5x150, 5x100 points delivered."
