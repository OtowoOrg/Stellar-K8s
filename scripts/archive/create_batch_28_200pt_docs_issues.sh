#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Hard difficulty documentation batch (200 Points each).

Prerequisites:
  - gh CLI installed and authenticated
  - Network access to api.github.com

Optional environment variables:
  REPO                Target repository (default: OtowoOrg/Stellar-K8s)
  DRY_RUN             Set to 1 to print commands without executing

Example:
  REPO=myorg/my-fork DRY_RUN=1 $(basename "$0")
EOF
}

for arg in "$@"; do
  case "$arg" in
    -h|--help) show_help; exit 0 ;;
  esac
done

EXPECTED_ISSUE_COUNT=4
ACTUAL_ISSUE_COUNT=$(grep -c '^gh issue create' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT." >&2
  exit 1
fi

echo "Creating Batch 28 of 4 Hard (200-point) documentation issues..."

# Issue 1: Comprehensive Architecture Documentation
gh issue create --repo "$REPO" \
  --title "Create comprehensive architecture documentation with interactive diagrams" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Design and create comprehensive architecture documentation that covers the entire Stellar-K8s operator system with interactive diagrams, sequence flows, and detailed component descriptions.

### ✅ Acceptance Criteria
- Create detailed architecture overview with system context diagram
- Document all major components and their interactions
- Add interactive C4 model diagrams (Context, Container, Component, Code)
- Create sequence diagrams for key workflows (reconciliation, failover, scaling)
- Document data flow and state management patterns
- Add deployment architecture diagrams for various topologies
- Create decision trees for troubleshooting common issues
- Include performance characteristics and scalability limits
- Add security architecture and threat model documentation
- Create interactive diagrams using Mermaid or PlantUML
- Document integration points with external systems
- Add capacity planning guidelines with sizing recommendations
- Include disaster recovery architecture patterns
- Create video walkthrough of the architecture (optional)
- Ensure all diagrams are version-controlled and maintainable

### 📚 Deliverables
- Architecture documentation in \`docs/architecture/\` directory
- Interactive diagrams embedded in markdown files
- Architecture decision records (ADRs) for major design choices
- Reference architecture examples for common deployment scenarios
- Architecture review checklist for contributors
" --label "stellar-wave,documentation,architecture"

# Issue 2: Advanced Operations Runbook
gh issue create --repo "$REPO" \
  --title "Build comprehensive operations runbook with incident response procedures" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Create a production-grade operations runbook that covers day-to-day operations, incident response, troubleshooting procedures, and operational best practices for running Stellar-K8s in production.

### ✅ Acceptance Criteria
- Create comprehensive day-to-day operations guide
- Document incident response procedures with severity levels
- Add detailed troubleshooting guides for common issues
- Create runbook automation scripts for routine tasks
- Document monitoring and alerting best practices
- Add capacity planning and scaling procedures
- Create backup and disaster recovery procedures
- Document upgrade and rollback procedures
- Add security incident response playbooks
- Create performance tuning and optimization guides
- Document compliance and audit procedures
- Add on-call rotation and escalation procedures
- Create post-incident review templates
- Document SLA/SLO definitions and monitoring
- Add operational metrics and KPI tracking

### 📚 Deliverables
- Operations runbook in \`docs/operations/\` directory
- Incident response playbooks with step-by-step procedures
- Troubleshooting decision trees and flowcharts
- Automation scripts for common operational tasks
- Monitoring dashboard templates and alert configurations
- Operational checklists for various scenarios
- Training materials for operations teams
" --label "stellar-wave,documentation,reliability"

# Issue 3: Developer Onboarding and Contribution Guide
gh issue create --repo "$REPO" \
  --title "Create comprehensive developer onboarding guide with interactive tutorials" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Build a complete developer onboarding experience that takes new contributors from zero to productive, including interactive tutorials, code walkthroughs, and hands-on exercises.

### ✅ Acceptance Criteria
- Create step-by-step onboarding guide for new developers
- Document development environment setup for all platforms (macOS, Linux, Windows/WSL)
- Add interactive code tutorials with runnable examples
- Create architecture deep-dive for each major component
- Document coding standards and best practices
- Add testing strategy guide (unit, integration, e2e)
- Create debugging guide with common pitfalls
- Document CI/CD pipeline and release process
- Add code review guidelines and checklist
- Create contribution workflow with Git best practices
- Document how to add new features end-to-end
- Add performance profiling and optimization guide
- Create security development guidelines
- Add troubleshooting guide for development issues
- Include video tutorials for complex topics (optional)
- Create interactive exercises with solutions
- Document how to run and debug the operator locally
- Add guide for writing and running tests

### 📚 Deliverables
- Developer guide in \`docs/development/\` directory
- Interactive tutorials with code examples
- Development environment setup scripts
- Code walkthrough documentation for key components
- Testing guide with examples
- Contribution templates and checklists
- Video tutorials (optional but recommended)
" --label "stellar-wave,documentation,dx"

# Issue 4: API Reference and Integration Guide
gh issue create --repo "$REPO" \
  --title "Generate comprehensive API reference with integration examples and SDKs" \
  --body "### 🔴 Difficulty: Hard (200 Points)

Create complete API reference documentation with auto-generated specs, integration examples, SDK documentation, and best practices for integrating with Stellar-K8s.

### ✅ Acceptance Criteria
- Auto-generate OpenAPI/Swagger specs for REST APIs
- Create comprehensive CRD API reference with all fields documented
- Add webhook API documentation with request/response examples
- Document metrics API with all available metrics
- Create integration guide for common use cases
- Add code examples in multiple languages (Go, Python, JavaScript)
- Document authentication and authorization mechanisms
- Create SDK documentation if SDKs exist
- Add API versioning and compatibility guide
- Document rate limiting and quotas
- Create API migration guides for version upgrades
- Add GraphQL schema documentation if applicable
- Document WebSocket API for real-time updates
- Create Postman/Insomnia collections for API testing
- Add API security best practices
- Document error codes and troubleshooting
- Create integration testing examples
- Add performance considerations and optimization tips
- Document API deprecation policy

### 📚 Deliverables
- API reference documentation in \`docs/api/\` directory
- Auto-generated OpenAPI specifications
- CRD field reference with validation rules
- Integration examples in multiple languages
- API client libraries or SDK documentation
- Postman/Insomnia collections for testing
- API migration guides between versions
- Security and best practices guide
" --label "stellar-wave,documentation,feature"

echo "✅ Created 4 hard (200-point) documentation issues successfully!"
echo "Batch 28 documentation issues should now be available in the repository."
