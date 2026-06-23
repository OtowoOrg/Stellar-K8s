#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

echo "Creating Batch 23 (12 x 100 pts, 8 x 200 pts) issues..."

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

# --- 100 POINT ISSUES (1-12) ---

create_issue_with_retry "Add Detailed Logging for CRD Validation Failures" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Improve the operator's internal logging to capture the specific reasons why a \`StellarNode\` CRD failed validation, making it easier to debug from the operator logs.

### ✅ Acceptance Criteria
- Log the specific validation error message when an admission request is rejected.
- Include the resource name and namespace in the log entry.
- Ensure sensitive data is not logged during validation."

create_issue_with_retry "Update Architecture Diagram in Documentation" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Refresh the high-level architecture diagram in \`docs/architecture.md\` to include recent additions like the REST API, sidecars, and monitoring stack.

### ✅ Acceptance Criteria
- Create a new Mermaid or SVG diagram showing the current component interactions.
- Ensure the diagram is clear and matches the current codebase structure.
- Update the descriptive text accompanying the diagram."

create_issue_with_retry "Add Glossary of Terms to Project Documentation" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Create a \`docs/glossary.md\` file that defines common terms used in the Stellar-K8s project (e.g., Quorum Set, SCP, Reconciler, Horizon).

### ✅ Acceptance Criteria
- Include at least 20 key terms and their definitions.
- Cross-link glossary terms in other documentation pages.
- Ensure the language is accessible to new contributors."

create_issue_with_retry "Implement --version Flag for 'stellar' CLI" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Add a standard \`--version\` flag to the \`stellar\` CLI that prints the current binary version and build date.

### ✅ Acceptance Criteria
- Output should follow the format: \`stellar-cli vX.Y.Z (Build Date: YYYY-MM-DD)\`.
- Ensure the version is correctly injected during the build process.
- Support both \`-v\` and \`--version\`."

create_issue_with_retry "Add Code Coverage Reporting to CI Pipeline" "stellar-wave,enhancement,ci" "### 🟢 Difficulty: Low (100 Points)

Integrate a code coverage tool (e.g., \`tarpaulin\`) into the GitHub Actions workflow and upload reports to Codecov or a similar service.

### ✅ Acceptance Criteria
- Add a new \`coverage\` job to the CI workflow.
- Generate a summary report in every Pull Request.
- Fail CI if coverage drops below a certain threshold (optional)."

create_issue_with_retry "Create Development Environment Setup Script for macOS" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Provide a bash script (\`scripts/setup-mac.sh\`) that automates the installation of all necessary development dependencies for macOS users.

### ✅ Acceptance Criteria
- Install Homebrew, Rust, K8s CLI tools, and GitHub CLI.
- Verify the installation and provide instructions for manual steps.
- Document the script in \`CONTRIBUTING.md\`."

create_issue_with_retry "Add Metadata Labels to Helm Chart for Resource Grouping" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Update the Helm chart templates to include consistent \`app.kubernetes.io/part-of\` and \`app.kubernetes.io/managed-by\` labels on all resources.

### ✅ Acceptance Criteria
- Ensure all resources have the same 'parent' labels.
- Verify label consistency via \`helm template\`.
- Update the documentation to show how to use these labels for filtering."

create_issue_with_retry "Improve Contribution Guide with Clear PR Requirements" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Refine \`CONTRIBUTING.md\` to provide a clear checklist for Pull Requests, including requirements for tests, documentation, and linting.

### ✅ Acceptance Criteria
- Add a 'PR Checklist' section.
- Provide examples of good commit messages.
- Clearly state the branching and merging strategy."

create_issue_with_retry "Add Syntax Highlighting for TOML in Documentation" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Audit the documentation and ensure that all TOML configuration examples use the correct code fence for syntax highlighting (\`\`\`toml).

### ✅ Acceptance Criteria
- Update all \`Stellar Core\` and \`Horizon\` config snippets.
- Verify that the documentation site renders the highlights correctly.
- Ensure consistency across all pages."

create_issue_with_retry "Implement 'stellar update-check' Command in CLI" "stellar-wave,enhancement,dx" "### 🟢 Difficulty: Low (100 Points)

Add a command that manually checks for a new version of the operator or CLI and prints instructions on how to upgrade.

### ✅ Acceptance Criteria
- Fetch the latest release from the GitHub API.
- Compare with local version and print the result.
- Provide a direct link to the release notes."

create_issue_with_retry "Add Resource Request/Limit Defaults for Diagnostic Sidecar" "stellar-wave,enhancement,performance" "### 🟢 Difficulty: Low (100 Points)

Ensure that the diagnostic sidecar container always has reasonable default resource requests and limits to prevent it from consuming too much cluster memory.

### ✅ Acceptance Criteria
- Set defaults to 50m CPU and 64Mi Memory.
- Allow these defaults to be overridden in the CRD.
- Verify the limits are correctly applied in the generated pod spec."

create_issue_with_retry "Create Release Note Template for New Versions" "stellar-wave,documentation,dx" "### 🟢 Difficulty: Low (100 Points)

Provide a standardized markdown template (\`.github/RELEASE_TEMPLATE.md\`) to be used when creating new releases on GitHub.

### ✅ Acceptance Criteria
- Include sections for: Features, Bug Fixes, Breaking Changes, and Contributors.
- Provide instructions on how to use the template.
- Ensure the template is visually appealing and informative."

# --- 200 POINT ISSUES (13-20) ---

create_issue_with_retry "Implement Automated Quorum Integrity Check for Storage Snapshots" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Develop a background worker that verifies the integrity of S3/GCS snapshots by spinning up a temporary validator and ensuring it can reach consensus using the snapshot data.

### ✅ Acceptance Criteria
- Automate the 'Restore and Test' cycle for random snapshots.
- Report PASS/FAIL status as a Prometheus metric.
- Alert on any snapshot that fails to produce a consistent ledger."

create_issue_with_retry "Develop Stellar-K8s Load Balancer for Multi-Region Peer Discovery" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Implement a custom load balancer (or ExternalDNS configuration) that intelligently directs peering traffic to the geographically closest available Stellar node.

### ✅ Acceptance Criteria
- Support multi-region routing based on latency.
- Automate the creation of regional SRV records.
- Document the impact on SCP message propagation time."

create_issue_with_retry "Implement Zero-Downtime Schema Migration for Horizon Database" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Provide a mechanism within the operator to perform Horizon database schema migrations (using \`horizon db migrate\`) without taking the API offline.

### ✅ Acceptance Criteria
- Use a 'Blue/Green' database strategy or temporary shadow tables.
- Ensure compatibility with active Horizon instances during the migration.
- Automate the rollback if the migration fails."

create_issue_with_retry "Build Real-time SCP Analytics Dashboard using OpenSearch" "stellar-wave,enhancement,observability" "### 🔴 Difficulty: High (200 Points)

Integrate with OpenSearch to provide a real-time analytics dashboard that visualizes SCP message volume, quorum slice health, and node participation rates.

### ✅ Acceptance Criteria
- Deploy an OpenSearch/Fluent-bit stack as an optional addon.
- Create pre-built dashboards for network-wide monitoring.
- Document the storage and retention policies."

create_issue_with_retry "Implement Automated Node-Drain Handling for Spot Instances" "stellar-wave,enhancement,reliability" "### 🔴 Difficulty: High (200 Points)

Ensure the operator gracefully handles termination notices from cloud provider spot/pre-emptible instances by migrating Stellar nodes before the instance is killed.

### ✅ Acceptance Criteria
- Monitor cloud-provider termination metadata endpoints.
- Trigger a 'Maintenance Mode' and graceful shutdown on notice.
- Automatically reschedule the node on a stable instance."

create_issue_with_retry "Develop Stellar-K8s Operator SDK for Custom Plugins" "stellar-wave,enhancement,dx" "### 🔴 Difficulty: High (200 Points)

Create a set of Rust traits and libraries that allow third-party developers to build custom 'Reconciliation Hooks' or 'Sidecar Injectors' for the operator.

### ✅ Acceptance Criteria
- Provide a clear, documented API for operator plugins.
- Include example plugins for custom logging and monitoring.
- Support dynamic loading or compilation of plugins."

create_issue_with_retry "Implement Multi-Layered Caching for Soroban RPC using local-SSD" "stellar-wave,enhancement,performance" "### 🔴 Difficulty: High (200 Points)

Optimize Soroban RPC performance by implementing a multi-layered cache that uses in-memory (Redis) for hot data and local-SSD for larger working sets.

### ✅ Acceptance Criteria
- Support local-SSD ephemeral storage for caching.
- Implement intelligent cache eviction policies.
- Benchmark the improvement in WASM execution time."

create_issue_with_retry "Build Compliance Audit Exporter for Operator Internal State" "stellar-wave,enhancement,security" "### 🔴 Difficulty: High (200 Points)

Develop a tool that exports the operator's internal state, configuration history, and reconciliation logs into a standardized format for security audits.

### ✅ Acceptance Criteria
- Support PDF and JSON export formats.
- Include a summary of all administrative actions taken.
- Ensure the export is signed and tamper-evident."

echo ""
echo "🎉 Batch 23 (20 issues) created successfully! 12x100, 8x200 points delivered."
