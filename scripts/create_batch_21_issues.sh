#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

echo "Creating Batch 21 (20 x 200 pts) issues with auto-retry..."

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

# --- 200 POINT ISSUES (1-20) ---

create_issue_with_retry "Implement Zero-Trust Pod Security Standards (PSS) Enforcement" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Enforce 'Restricted' Pod Security Standards across all namespaces managed by the operator to ensure maximum workload isolation.

### 📋 Context
Default Kubernetes security is often too permissive. For financial infrastructure, we must ensure no pods run as root, have privilege escalation, or use host networking.

### ✅ Acceptance Criteria
- Automatically apply \`pod-security.kubernetes.io/enforce: restricted\` labels to managed namespaces.
- Update all resource builders to comply with 'Restricted' profile requirements.
- Implement a validation check that rejects CRDs attempting to bypass these security constraints.
- Document the security posture in \`docs/security/pss.md\`."

create_issue_with_retry "Develop Automated Node-Drain Orchestrator for Stellar Core" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Create a controller that intelligently manages node drains by gracefully migrating Stellar Core pods while maintaining quorum liveness.

### 📋 Context
Standard \`kubectl drain\` can be too aggressive, potentially taking down too many validators at once. We need a 'Stellar-aware' drain process.

### ✅ Acceptance Criteria
- Monitor Kubernetes Node events for \`SchedulingDisabled\`.
- Implement a graceful shutdown sequence that ensures the node has 'Caught up' on a peer before exiting.
- Coordinate with Pod Disruption Budgets to prevent simultaneous outages.
- Emit events when a node migration is successfully managed."

create_issue_with_retry "Implement Cross-Cloud Failover for Stellar Horizon Clusters" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Enable seamless failover of Stellar Horizon API traffic between different cloud providers (e.g., AWS to GCP) during major provider outages.

### 📋 Context
Cloud-level outages happen. To achieve 99.99% availability, the RPC layer must be able to shift traffic to a different cloud infrastructure entirely.

### ✅ Acceptance Criteria
- Integrate with Global Load Balancers (e.g., Cloudflare or F5).
- Automate the synchronization of Horizon DB state across cloud boundaries.
- Implement health checks that trigger the DNS/LB shift.
- Document the multi-cloud recovery plan."

create_issue_with_retry "Build Custom Stellar-K8s Operator Dashboard" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Develop a dedicated web UI for the Stellar-K8s operator to provide a visual overview of fleet health, CRD status, and reconciliation logs.

### 📋 Context
Managing a large fleet via CLI alone is difficult. A dashboard provides a 'Single Pane of Glass' for monitoring and basic administrative tasks.

### ✅ Acceptance Criteria
- Build a React-based frontend integrated with the operator's REST API.
- Visualize the status of all \`StellarNode\` resources.
- Provide a 'Log Viewer' for both the operator and managed pods.
- Implement basic action buttons (Restart, Maintenance Mode, Prune)."

create_issue_with_retry "Implement Automated Quorum Set Rotation and Optimization" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Develop a background worker that analyzes peer performance and automatically suggests (or applies) quorum set updates to improve network latency.

### 📋 Context
Quorum sets are often static. Over time, some peers become slow or unreliable. Dynamic optimization ensures the validator always talks to the best available peers.

### ✅ Acceptance Criteria
- Collect RTT and availability metrics for all configured peers.
- Implement an 'Optimization Engine' that calculates the most efficient quorum topology.
- Support 'Auto-Apply' and 'Manual-Approval' modes for rotation.
- Document the impact on SCP convergence times."

create_issue_with_retry "Develop Stellar-Native Network Policy Generator" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Automatically generate and apply least-privilege Kubernetes NetworkPolicies based on the peer connections defined in the Stellar Core configuration.

### 📋 Context
Manually writing NetworkPolicies for dozens of peers is tedious and error-prone. The operator should 'know' who a node needs to talk to and open only those ports.

### ✅ Acceptance Criteria
- Parse the \`QUORUM_SET\` and \`KNOWN_PEERS\` from the CRD.
- Generate granular egress rules for each peer IP/hostname.
- Block all other non-essential traffic.
- Update policies dynamically as the peer list changes."

create_issue_with_retry "Implement Encrypted Snapshots for DB Volumes using Cloud KMS" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Ensure that all automated database snapshots are encrypted at rest using provider-managed keys (AWS KMS, GCP KMS) to comply with data protection regulations.

### 📋 Context
Backups containing ledger data must be encrypted. Integrating with cloud-native KMS provides a secure and auditable key management strategy.

### ✅ Acceptance Criteria
- Add \`encryptionKeyRef\` support to the snapshot configuration.
- Integrate with cloud APIs to specify the KMS key during snapshot creation.
- Implement a verification check to ensure snapshots are indeed encrypted.
- Document the IAM permissions required for KMS access."

create_issue_with_retry "Build History Archive Pruning Worker with Lifecycle Integration" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Develop a worker that identifies and deletes old, unnecessary history archive files from object storage (S3/GCS) based on user-defined retention policies.

### 📋 Context
History archives can grow to terabytes. Not all historical data is needed for catch-up operations. Automated pruning saves significant storage costs.

### ✅ Acceptance Criteria
- Implement a \`PruningPolicy\` in the CRD (e.g., 'keep last 1 year').
- Safely identify 'Prunable' checkpoints using Stellar Core's archive logic.
- Execute deletions with 'Dry-Run' and 'Safety-Lock' features.
- Integrate with cloud-native bucket lifecycle rules where possible."

create_issue_with_retry "Implement Automated Vulnerability Scanning for all Workload Images" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Integrate with a scanning engine (e.g., Trivy or Grype) to automatically audit all images used by the operator and its managed pods for known CVEs.

### 📋 Context
Running outdated or vulnerable images is a major security risk. We need continuous visibility into the vulnerability status of our fleet.

### ✅ Acceptance Criteria
- Implement a background scanner that checks all running images.
- Report vulnerability findings as Prometheus metrics and Kubernetes Events.
- Provide a CLI command to list all vulnerable pods.
- Alert on 'Critical' vulnerabilities detected in production."

create_issue_with_retry "Develop Stellar-K8s Admission Controller for Resource Compliance" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Implement a Validating Admission Webhook that ensures all \`StellarNode\` resources meet organizational standards for resource limits and annotations.

### 📋 Context
Prevent 'noisy neighbors' by ensuring every pod has defined CPU/Memory requests and limits. Enforce ownership labels for billing.

### ✅ Acceptance Criteria
- Validate that \`resources.limits\` and \`resources.requests\` are always present.
- Enforce a maximum limit on resources per node type.
- Require specific labels (e.g., \`project-id\`, \`owner\`).
- Return clear, helpful error messages on rejection."

create_issue_with_retry "Implement Multi-Architecture (ARM64/AMD64) Support" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Update the operator and all its associated containers (sidecars, diagnostics) to support running on both ARM64 (e.g., Graviton) and AMD64 architectures.

### 📋 Context
ARM64 instances often provide better price-performance for Stellar workloads. We should enable users to mix and match architecture types in their clusters.

### ✅ Acceptance Criteria
- Build and publish multi-arch Docker manifests for all images.
- Update the Helm chart to support \`nodeSelector\` and \`tolerations\` for architecture types.
- Verify the operator functions correctly when its pods are split across architectures.
- Performance benchmark comparison between ARM and AMD."

create_issue_with_retry "Build Real-Time Ledger Analysis Sidecar for Fork Detection" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Create a sidecar container that monitors the local ledger hash and compares it in real-time with multiple public 'anchors' to detect potential network forks.

### 📋 Context
Detecting a fork early is critical for validator operators. A sidecar can provide an independent 'sanity check' against the global network state.

### ✅ Acceptance Criteria
- Implement a lightweight Rust sidecar.
- Periodically fetch the latest ledger hash from the local Core and 3+ public nodes.
- Alert if a divergence persists for more than 3 ledgers.
- Export 'Sync Confidence' as a Prometheus metric."

create_issue_with_retry "Implement Automated Scaling for Soroban RPC based on Gas Trends" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Scale Soroban RPC nodes based on the aggregate gas consumption and transaction volume seen in the last few ledgers, allowing the fleet to handle Soroban load spikes.

### 📋 Context
Soroban transactions have complex resource requirements. Traditional scaling metrics don't account for the 'intensity' of smart contract execution.

### ✅ Acceptance Criteria
- Collect 'Gas Used' metrics from the Horizon/Soroban API.
- Implement a custom scaling algorithm that weights gas consumption.
- Update the HPA configuration to use these trends.
- Test scaling efficiency with a Soroban benchmark suite."

create_issue_with_retry "Develop Stellar-K8s CLI Plugin for Internal DB SQL Execution" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Provide a secure way for authorized operators to execute read-only SQL queries against the internal Postgres databases of managed nodes directly from the CLI.

### 📋 Context
Troubleshooting often requires checking the internal state of the DB. Manually port-forwarding and connecting is tedious.

### ✅ Acceptance Criteria
- Add \`sql\` subcommand to the CLI.
- Automatically manage the port-forwarding and credentials retrieval.
- Enforce read-only access to prevent accidental data corruption.
- Support common output formats (Table, JSON, CSV)."

create_issue_with_retry "Implement Proactive Failure Detection using eBPF" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Use eBPF to monitor the system calls and network behavior of the Stellar Core process to detect 'Silent Failures' or performance bottlenecks that logs miss.

### 📋 Context
Some failures (like slow disk IO or network jitter) don't always show up clearly in application logs. eBPF provides deep visibility into the kernel-app boundary.

### ✅ Acceptance Criteria
- Integrate an eBPF exporter (e.g., Inspektor Gadget or custom BPF).
- Monitor \`write()\` latency to the ledger DB.
- Track TCP retransmits and handshake times for peer connections.
- Correlate eBPF events with application-level performance drops."

create_issue_with_retry "Build Compliance Logging Sink for Audit Trails" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Implement a dedicated logging sink that captures every administrative change made to the \`StellarNode\` CRD and the operator's configuration for auditing purposes.

### 📋 Context
In regulated environments, every change to production infrastructure must be logged and attributable to a user or system.

### ✅ Acceptance Criteria
- Implement a 'Change Log' worker that watches the K8s API for CRD updates.
- Record: Who changed it, what changed (diff), and when.
- Export these logs to a secure, immutable storage backend (e.g., S3 with Object Lock).
- Provide a 'Audit Report' tool in the CLI."

create_issue_with_retry "Implement Automated mTLS Key Rotation for Internal Communication" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Automate the full lifecycle of mTLS keys used for internal communication, including generation, distribution, and 'zero-downtime' rotation.

### 📋 Context
Rotating keys manually is a high-risk operation. The operator should handle the transition by supporting multiple valid keys during the rotation window.

### ✅ Acceptance Criteria
- Implement a 'Dual-Key' rotation strategy.
- Automatically generate new keys and update K8s Secrets.
- Signal pods to reload configurations without restarting where possible.
- Verify certificate validity before decommissioning old keys."

create_issue_with_retry "Develop Stellar-K8s Chaos Engineering Runner" "stellar-wave,testing,reliability" "### 🔴 Difficulty: High (200 Points)

Integrate with a chaos engineering tool (like Chaos Mesh) to run automated 'Destruction Tests' against the cluster, ensuring the reconciler handles extreme failures.

### 📋 Context
We need to know what happens when the network is 50% partitioned or the DB disk is 99% full. Chaos testing builds confidence in system resilience.

### ✅ Acceptance Criteria
- Create a set of \`ChaosExperiment\` templates (Pod Kill, Network Delay, IO Stress).
- Implement a 'Chaos Runner' script that executes these against a Testnet cluster.
- Assert that the operator eventually restores the system to 'Healthy' state.
- Report on 'Recovery Time Objectives' (RTO)."

create_issue_with_retry "Implement Zero-Downtime Storage Migration Tool" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Provide a way to migrate a Stellar node's persistent data between different storage classes (e.g., GP2 to GP3) without taking the node offline.

### 📋 Context
As cloud providers release new storage tiers, operators want to upgrade for better performance/cost. Moving terabytes of data is risky without automation.

### ✅ Acceptance Criteria
- Implement a migration controller that uses 'Volume Snapshots' and 'Data Syncing' sidecars.
- Automate the 'Switchover' phase with minimal (seconds) interruption.
- Ensure data integrity is verified before and after the move.
- Support cross-Availability Zone migrations."

create_issue_with_retry "Build Stellar-K8s Health Check API for Status Pages" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Expose a public-facing (or internal-facing) Health API that provides a high-level summary of the entire fleet's status, suitable for integration with StatusPage.io.

### 📋 Context
Stakeholders need a simple way to see 'Is the network up?' without looking at Prometheus or kubectl.

### ✅ Acceptance Criteria
- Implement a \`/v1/health/summary\` endpoint in the operator.
- Aggregate status across all managed nodes and networks.
- Include metrics like: % of Validators Synced, Average API Latency, and Active Incidents.
- Provide a ready-to-use integration guide for common status page providers."

echo ""
echo "🎉 Batch 21 (20 x 200 pts) issues created successfully! High-impact backlog expanded."
