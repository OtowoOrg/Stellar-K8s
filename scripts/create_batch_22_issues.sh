#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

echo "Creating Batch 22 (20 x 200 pts, 10 x 100 pts) issues..."

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

create_issue_with_retry "Implement Dynamic Resource Rebalancing for Catch-up Optimization" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Develop a mechanism that temporarily increases CPU/Memory limits for Stellar Core pods while they are in 'Catching Up' mode, and scales them back once they reach 'Synced'.

### 📋 Context
Catching up on historical ledgers is extremely compute-intensive. Once synced, a node's resource needs drop significantly. Dynamic rebalancing optimizes cluster efficiency.

### ✅ Acceptance Criteria
- Monitor the \`stellar_core_sync_state\` metric.
- Automatically update the Pod spec or use Vertical Pod Autoscaler (VPA) 'In-Place' updates.
- Ensure rebalancing doesn't trigger unnecessary pod restarts during critical sync phases.
- Document the cost savings and sync-time improvements."

create_issue_with_retry "Develop Automated DB Vacuuming Orchestrator for Postgres" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Implement a background worker that monitors Postgres bloat and automatically triggers \`VACUUM ANALYZE\` or \`REPACK\` operations during low-traffic periods.

### 📋 Context
Stellar databases grow rapidly and generate significant bloat. Without regular vacuuming, performance degrades and storage is wasted.

### ✅ Acceptance Criteria
- Implement a \`DbMaintenance\` controller.
- Monitor table bloat ratios via SQL queries.
- Schedule maintenance during 'Quiet Windows' defined in the CRD.
- Ensure vacuuming doesn't interfere with active ledger writes."

create_issue_with_retry "Implement Sidecar-based Log Aggregation to S3/GCS" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Add a sidecar to all managed pods that streams logs directly to persistent object storage, ensuring log retention even if the K8s cluster or log aggregator fails.

### 📋 Context
Standard K8s logging (Fluentd/Loki) can lose data during high-stress periods. Direct-to-S3 streaming provides a durable audit trail for financial compliance.

### ✅ Acceptance Criteria
- Create a lightweight logging sidecar (Rust or Golang).
- Support compression and batching to minimize API costs.
- Implement 'Log Rotation' within the sidecar.
- Provide a CLI tool to fetch and search these archived logs."

create_issue_with_retry "Build Stellar-K8s Telemetry Collector using OpenTelemetry" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Integrate the OpenTelemetry (OTel) SDK into the operator and managed sidecars to provide standardized traces and metrics to any OTLP-compliant backend.

### 📋 Context
Prometheus is great for metrics, but we need distributed tracing to understand reconciliation latency and API performance across the fleet.

### ✅ Acceptance Criteria
- Instrument the operator's reconciler with OTel traces.
- Deploy an OTel Collector as part of the Helm chart.
- Export traces to Honeycomb, Jaeger, or Grafana Tempo.
- Add 'Trace ID' to all structured log output."

create_issue_with_retry "Implement Automated Secret Rotation for Database Credentials" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Automate the rotation of Postgres database passwords, ensuring that both the DB and the Stellar Core/Horizon pods are updated without downtime.

### 📋 Context
Hardcoded or long-lived credentials are a security risk. Automated rotation is a requirement for SOC2 and high-security environments.

### ✅ Acceptance Criteria
- Integrate with K8s Secret Store CSI Driver or HashiCorp Vault.
- Implement a rotation controller that updates the DB user and then the Pod environment variables.
- Use a 'Graceful Restart' or 'SIGHUP' reload for the application pods.
- Verify connectivity after each rotation."

create_issue_with_retry "Implement Canary Analysis Engine using Kayenta Integration" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Automate the decision-making process for Canary deployments by integrating with Kayenta to compare performance metrics between 'Baseline' and 'Canary' pods.

### 📋 Context
Manually reviewing Grafana during a Canary release is error-prone. Kayenta provides statistical analysis to ensure the new version isn't regressing performance.

### ✅ Acceptance Criteria
- Define 'Canary Judges' based on Ledger Close Time and API Error Rates.
- Automate the 'Promote' or 'Rollback' action based on the judge's score.
- Create a dashboard showing the 'Canary Health Score'.
- Document the automated rollout pipeline."

create_issue_with_retry "Implement Pod-to-Pod mTLS Enforcement using Linkerd" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Provide a production-ready guide and configuration for running the Stellar-K8s fleet within a Linkerd service mesh with 'Strict' mTLS enabled.

### 📋 Context
Zero-trust networking requires that all pod-to-pod communication be encrypted and authenticated. Linkerd provides this with minimal overhead.

### ✅ Acceptance Criteria
- Support Linkerd 'Inject' annotations in the CRD.
- Configure \`Server\` and \`ServerAuthorization\` resources to restrict traffic.
- Document the impact on P2P performance.
- Provide a 'Mesh Health' dashboard."

create_issue_with_retry "Build Stellar-Native Autoscaler for Horizon (Rate-Limit Based)" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Develop a custom autoscaler that scales Horizon pods based on the frequency of HTTP 429 (Too Many Requests) responses, rather than just CPU usage.

### 📋 Context
Horizon nodes often hit rate limits before they hit CPU limits. Scaling based on 'Load Shedding' metrics is more accurate for API infrastructure.

### ✅ Acceptance Criteria
- Export 429 error rates from the Horizon service.
- Implement a custom HPA controller that targets 429 frequency.
- Support 'Predictive Scaling' based on historical rate-limit spikes.
- Verify scaling behavior under heavy API load simulations."

create_issue_with_retry "Implement Automated Backup Verification via Temporary Clusters" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Automatically verify the integrity of DB backups and history archives by spinning up a temporary, ephemeral pod to perform a test 'Restore' every week.

### 📋 Context
A backup is only as good as its last successful restore. Automated verification ensures that disaster recovery will actually work when needed.

### ✅ Acceptance Criteria
- Implement a \`BackupVerifier\` CronJob.
- The job must pull a random backup, restore it to a temporary PVC, and verify the last ledger sequence.
- Report PASS/FAIL to the operator and emit alerts on failure.
- Clean up all temporary resources after verification."

create_issue_with_retry "Develop Stellar-K8s Plugin for Backstage" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Create a plugin for Spotify's Backstage (Developer Portal) that allows teams to view and manage their Stellar infrastructure directly from the service catalog.

### 📋 Context
Backstage is the standard for internal developer portals. A Stellar-K8s plugin makes infrastructure 'Self-Service' for developers.

### ✅ Acceptance Criteria
- Build a 'Stellar Node' entity provider for Backstage.
- Implement a dashboard showing Sync Status, Version, and Network info.
- Add 'Action' templates for creating new StellarNode resources.
- Support deep links to Grafana and logs."

create_issue_with_retry "Implement Zero-Downtime Migration between Kubernetes Namespaces" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Develop a tool to migrate an entire Stellar infrastructure (Nodes, DBs, Configs) from one K8s namespace to another without dropping peer connections or stopping API traffic.

### 📋 Context
Teams often need to reorganize their clusters. Moving stateful apps between namespaces is normally a disruptive manual process.

### ✅ Acceptance Criteria
- Implement a migration controller that handles resource cloning.
- Use a temporary 'Dual-Namespace' service bridge.
- Automate the PVC migration using \`VolumeSnapshots\`.
- Document the 'Switchover' sequence."

create_issue_with_retry "Build Real-time SCP Analytics Pipeline using Kafka" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Implement a sidecar that captures raw SCP messages and streams them to a Kafka topic for real-time analysis of quorum health and network topology.

### 📋 Context
Deep analysis of SCP behavior requires processing thousands of messages per second. Kafka provides the throughput needed for this level of observability.

### ✅ Acceptance Criteria
- Create a high-throughput SCP stream sidecar.
- Support Avro/Protobuf schema for SCP messages.
- Implement a sample 'Topological Health' consumer.
- Document the Kafka schema and integration points."

create_issue_with_retry "Implement Proactive Disk Scaling using EBS local-path provisioners" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Automatically increase the size of EBS/GCP volumes as the Stellar ledger database grows, preventing 'Disk Full' outages without manual intervention.

### 📋 Context
Stellar ledger growth is unpredictable. Running out of disk space is a fatal error for a validator. Proactive scaling keeps the node running.

### ✅ Acceptance Criteria
- Monitor disk usage percentage on managed PVCs.
- Automatically trigger \`expand-pvc\` when usage exceeds 80%.
- Coordinate with the storage provider's expansion limits.
- Log every expansion event for cost auditing."

create_issue_with_retry "Develop Stellar-K8s CLI for Multi-Cluster Performance Comparison" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Add a command to the CLI that compares performance metrics (TPS, Ledger Time) between two different clusters or two different configurations in real-time.

### 📋 Context
When testing optimizations, operators need an easy way to see if 'Cluster A' is actually performing better than 'Cluster B'.

### ✅ Acceptance Criteria
- Add \`benchmark-compare\` subcommand.
- Connect to two different K8s contexts or Prometheus instances.
- Render a side-by-side comparison table or graph in the terminal.
- Support exporting results to a PDF or HTML report."

create_issue_with_retry "Implement Automated Certificate Authority (CA) Management" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Automate the lifecycle of the Internal Root CA used by the operator for issuing pod certificates, including secure root storage and automated CRL generation.

### 📋 Context
For internal mTLS to be secure, the Root CA must be managed carefully. Automating its rotation and CRL management reduces operational risk.

### ✅ Acceptance Criteria
- Integrate with Vault PKI engine or cert-manager \`SelfSigned\` issuers.
- Automate Root CA rotation every 2-5 years.
- Generate and publish Certificate Revocation Lists (CRLs).
- Document the 'Trust Anchor' distribution process."

create_issue_with_retry "Build Compliance Reporting Dashboard for SOC2/ISO27001" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Develop a dashboard that automatically audits the cluster against SOC2/ISO27001 security controls (Encryption at rest, mTLS, RBAC, Logging) and generates a PDF report.

### 📋 Context
Compliance is a major hurdle for financial institutions. An automated 'Compliance Score' helps teams stay audit-ready.

### ✅ Acceptance Criteria
- Map K8s resource states to specific compliance controls.
- Provide a 'Compliance Gap Analysis' in the UI.
- Generate a time-stamped 'Audit Evidence' report.
- Support custom compliance benchmarks."

create_issue_with_retry "Implement Automated Node Repair for Stalled Validators" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Develop a self-healing controller that detects if a validator node has stalled (stopped closing ledgers) and automatically attempts tiered remediation (Restart Core -> Rebuild DB -> Reschedule Pod).

### 📋 Context
Nodes sometimes get into 'weird' states that logs don't fully explain. Automated 'First Response' repair keeps the network healthy while humans sleep.

### ✅ Acceptance Criteria
- Define 'Stalled' state criteria (e.g., no ledger close for 5 minutes).
- Implement a tiered remediation logic with safety backoffs.
- Avoid 'Repair Loops' if the issue is global/network-wide.
- Alert the operator of every repair action taken."

create_issue_with_retry "Develop Stellar-K8s Simulation Environment for Quorum Testing" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Build a 'Shadow Cluster' feature that allows operators to test quorum changes or upgrades against a simulated network before applying them to production.

### 📋 Context
Changing a quorum set is high-risk. A simulation environment lets you see if the change causes a partition *before* it happens.

### ✅ Acceptance Criteria
- Spin up a parallel 'Shadow' cluster using Kind or K3d.
- Replay recent mainnet traffic (read-only) to the shadow nodes.
- Validate that the proposed configuration reaches consensus.
- Report on 'Quorum Safety Margin'."

create_issue_with_retry "Implement Multi-Layered Caching for Horizon using Redis" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Extend the operator to automatically deploy and configure a Redis-based cache layer for Horizon to improve performance for frequent API requests (e.g., Account lookups).

### 📋 Context
Horizon's performance is often limited by Postgres IO. A distributed cache significantly reduces DB load and latency for common queries.

### ✅ Acceptance Criteria
- Add \`caching\` block to the Horizon spec in the CRD.
- Automatically provision and scale a Redis cluster.
- Configure Horizon to use the cache for ledger/account data.
- Monitor 'Cache Hit Ratio' and export to Grafana."

create_issue_with_retry "Build Stellar-K8s Incident Response Toolkit" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Create a CLI-based toolkit that automates the gathering of 'Forensic Data' (Full logs, DB snapshots, K8s events, eBPF traces) during an active incident.

### 📋 Context
During an outage, speed is critical. Manually gathering all data for post-mortem takes too long. One command should capture the 'State of the World'.

### ✅ Acceptance Criteria
- Add \`incident collect\` subcommand.
- Snapshot the status of all managed resources in the namespace.
- Zip up logs from the last 2 hours for all relevant pods.
- Capture a 'FlameGraph' or 'Trace' if the diagnostic sidecar is present."

# --- 100 POINT ISSUES (21-30) ---

create_issue_with_retry "Improve Help Output for 'stellar topology' Command" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Refine the help text and examples for the \`stellar topology\` command to better explain the various output formats and filtering options.

### ✅ Acceptance Criteria
- Add detailed examples for ASCII vs Graphviz output.
- Document all available filter flags (Namespace, Network, Zone).
- Ensure the help text fits within standard terminal widths."

create_issue_with_retry "Add Shell Completion Support for the 'stellar' CLI" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Implement automated shell completion (Bash, Zsh, Fish) for the \`stellar\` CLI to improve operator speed and discoverability.

### ✅ Acceptance Criteria
- Generate completion scripts using \`clap\` or a similar crate.
- Provide an \`install-completion\` command.
- Verify completion works for subcommands and common flags."

create_issue_with_retry "Update FAQ Section in Documentation" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Audit and update the 'Frequently Asked Questions' section in the documentation to address common issues raised in recent support tickets and GitHub discussions.

### ✅ Acceptance Criteria
- Add 10+ new Q&As covering mTLS, storage expansion, and peer discovery.
- Categorize questions (Security, Performance, Troubleshooting).
- Ensure all links are up to date."

create_issue_with_retry "Add Stellar-K8s Badges to README.md" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Enhance the project README with status badges for CI/CD, Code Coverage, Version, and License.

### ✅ Acceptance Criteria
- Add GitHub Actions status badge.
- Add Codecov coverage badge.
- Add crates.io version and license badges.
- Ensure the README looks professional and informative."

create_issue_with_retry "Implement 'stellar doctor' Command for Environment Verification" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Add a \`doctor\` command that verifies the local environment (gh CLI version, K8s context, RBAC permissions) and warns about missing requirements.

### ✅ Acceptance Criteria
- Check for required CLI tools (\`gh\`, \`kubectl\`, \`helm\`).
- Verify the current K8s context has enough permissions to run the operator.
- Output a clear 'Green/Red' status for each check."

create_issue_with_retry "Support Custom Annotations in Helm Chart for All Resources" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Update the Helm chart to allow users to specify custom annotations for all generated resources (Deployments, Services, PVCs).

### ✅ Acceptance Criteria
- Add \`extraAnnotations\` to the \`values.yaml\`.
- Ensure annotations are propagated to both the metadata and the pod spec template.
- Add a test verifying annotation inheritance."

create_issue_with_retry "Add JSON Output Support to 'stellar audit' Command" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Implement a \`--json\` flag for the \`stellar audit\` command to allow for easy integration with external automated security scanners.

### ✅ Acceptance Criteria
- Create a structured JSON schema for audit results.
- Ensure the JSON output includes all PASS/FAIL details.
- Add a unit test for the JSON serialization logic."

create_issue_with_retry "Improve Installation Guide for Windows (WSL2) Users" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Create a dedicated section in the documentation for installing and running the Stellar-K8s operator on Windows using WSL2.

### ✅ Acceptance Criteria
- Document specific WSL2 networking quirks.
- Provide a step-by-step guide for setting up Docker Desktop or Minikube on WSL2.
- Include troubleshooting steps for common 'Windows-only' issues."

create_issue_with_retry "Add Unit Tests for CLI Argument Parser" "stellar-wave,testing,dx" "### 🟢 Difficulty: Low (100 Points)

Increase the test coverage of the CLI tool by adding comprehensive unit tests for the argument parsing and validation logic.

### ✅ Acceptance Criteria
- Test all subcommands with valid and invalid flags.
- Verify that default values are applied correctly.
- Ensure that the parser handles edge cases (e.g., missing arguments) gracefully."

create_issue_with_retry "Create Community Support Template for GitHub" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Implement a set of GitHub Issue Templates (Bug Report, Feature Request, Support Question) to improve the quality of community contributions.

### ✅ Acceptance Criteria
- Create \`.github/ISSUE_TEMPLATE/\` files.
- Include specific sections for 'Environment Info' and 'Steps to Reproduce'.
- Add a 'Discussion' link for support questions."

echo ""
echo "🎉 Batch 22 (30 issues) created successfully! 20x200, 10x100 points delivered."
