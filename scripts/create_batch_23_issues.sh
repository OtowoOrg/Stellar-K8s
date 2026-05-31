#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

echo "Creating Batch 23 (12 x 100 pts) issues..."

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

echo ""
echo "🎉 Batch 23 (12 x 100 pts) issues created successfully! DX and Docs backlog improved."
