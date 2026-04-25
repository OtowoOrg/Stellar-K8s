#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/repo.sh"
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 16, 20 x 200 pts).

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

EXPECTED_ISSUE_COUNT=20
ACTUAL_ISSUE_COUNT=$(grep -c '^create_issue_with_retry' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT. Update EXPECTED_ISSUE_COUNT or fix the script." >&2
  exit 1
fi

echo "Creating Batch 16 (20 x 200 pts) issues with auto-retry..."

# ─── 1 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Rustdoc Coverage for All Public API Surfaces" \
  "stellar-wave,documentation,dx" \
  "### 🔴 Difficulty: High (200 Points)

Public APIs without documentation are a barrier to adoption. Every public function, struct, and enum in the crate must have rustdoc comments.

### ✅ Acceptance Criteria
- Run \`cargo doc --no-deps\` and verify zero warnings.
- Add \`#![warn(missing_docs)]\` to \`lib.rs\`.
- Document all public items in \`src/crd/\`, \`src/controller/\`, \`src/rest_api/\`, and \`src/error.rs\`.
- Include usage examples in doc comments for key types.

### 📚 Resources
- [Rustdoc Documentation](https://doc.rust-lang.org/rustdoc/what-is-rustdoc.html)
- [RFC 1574: More API Documentation Guidelines](https://github.com/rust-lang/rfcs/blob/master/text/1574-more-api-documentation-guidelines.md)"


# ─── 2 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement CONTRIBUTING.md with DCO and PR Guidelines" \
  "stellar-wave,documentation,dx" \
  "### 🔴 Difficulty: High (200 Points)

Open-source projects need a clear contribution guide. This is essential for the Drips Wave program.

### ✅ Acceptance Criteria
- Create \`CONTRIBUTING.md\` covering: fork workflow, branch naming, commit conventions, PR template usage, and DCO sign-off.
- Add a \`.github/PULL_REQUEST_TEMPLATE.md\` with a checklist.
- Add an \`ISSUE_TEMPLATE/\` directory with bug report and feature request templates.
- Reference the contribution guide from the README.

### 📚 Resources
- [GitHub: Setting Guidelines for Repository Contributors](https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/setting-guidelines-for-repository-contributors)
- [DCO (Developer Certificate of Origin)](https://developercertificate.org/)"


# ─── 3 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Unit Tests for StellarNodeStatus Condition Helpers" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

The condition helpers in the status module are critical for correct reconciliation but lack thorough test coverage.

### ✅ Acceptance Criteria
- Write unit tests for \`set_condition\`, \`get_condition\`, and \`remove_condition\` helpers.
- Test edge cases: duplicate conditions, empty conditions list, transition time updates.
- Test the deprecated \`with_phase\` constructor.
- Achieve >90% line coverage for the conditions module.

### 📚 Resources
- [Kubernetes API Conventions: Conditions](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md#typical-status-properties)
- [\`src/crd/status.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/status.rs)"


# ─── 4 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Standardize Error Messages with Error Codes and Documentation" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

Error messages from the operator are inconsistent. Each error variant should have a unique code and a link to troubleshooting docs.

### ✅ Acceptance Criteria
- Assign a unique error code to each variant in \`src/error.rs\` (e.g., \`SK8S-001\`, \`SK8S-002\`).
- Create a \`docs/errors.md\` mapping each code to a description and resolution steps.
- Update all error formatting to include the code prefix.
- Add unit tests verifying error code formatting.

### 📚 Resources
- [\`src/error.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/error.rs)
- [Rust Error Handling Best Practices](https://rust-lang-nursery.github.io/rust-cookbook/errors.html)"


# ─── 5 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Makefile with Standard Development Targets" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

Contributors need a single entry point for common development tasks instead of remembering long cargo commands.

### ✅ Acceptance Criteria
- Create a \`Makefile\` with targets: \`build\`, \`test\`, \`lint\`, \`fmt\`, \`docker-build\`, \`helm-lint\`, \`crd-gen\`, \`run-local\`, \`clean\`.
- Each target should print a helpful description when \`make help\` is run.
- Ensure the CI pipeline uses the same Makefile targets for consistency.
- Document the Makefile in the README.

### 📚 Resources
- [GNU Make Manual](https://www.gnu.org/software/make/manual/make.html)
- [A Self-Documenting Makefile](https://marmelab.com/blog/2016/02/29/auto-documented-makefile.html)"


# ─── 6 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Retry Backoff Configuration for the Reconciler" \
  "stellar-wave,enhancement,reliability" \
  "### 🔴 Difficulty: High (200 Points)

The reconciler currently uses a fixed 30-second requeue interval. This should be configurable with exponential backoff for errors.

### ✅ Acceptance Criteria
- Add configurable \`requeue_interval\`, \`error_backoff_base\`, and \`max_backoff\` fields to the operator config.
- Implement exponential backoff with jitter for error retries.
- Keep the fixed interval for healthy reconciliation loops.
- Write unit tests for the backoff calculation logic.

### 📚 Resources
- [Exponential Backoff and Jitter](https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)"


# ─── 7 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Docker Compose Development Environment" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

Not all contributors have a K8s cluster. A Docker Compose setup would lower the barrier to entry for local development.

### ✅ Acceptance Criteria
- Create a \`docker-compose.yml\` that spins up the operator in 'dry-run' mode.
- Include a mock K8s API server or use \`k3s\` in Docker.
- Add a \`docker-compose.dev.yml\` overlay with hot-reloading via \`cargo-watch\`.
- Document the setup in the quickstart guide.

### 📚 Resources
- [Docker Compose Specification](https://docs.docker.com/compose/compose-file/)
- [Local Kubernetes with k3s in Docker](https://hub.docker.com/r/rancher/k3s)"


# ─── 8 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement CLI Flag Validation and Help Text Improvements" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

The CLI (\`stellar-operator\` binary) needs better flag validation and richer help text for operators unfamiliar with the tool.

### ✅ Acceptance Criteria
- Add long descriptions and examples to all \`clap\` arguments.
- Validate mutually exclusive flags at parse time (e.g., \`--scheduler\` and \`--dry-run\`).
- Add a \`--dump-config\` flag that prints the resolved configuration and exits.
- Write unit tests for argument parsing edge cases.

### 📚 Resources
- [Clap (Command Line Argument Parser) Documentation](https://docs.rs/clap/latest/clap/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 9 ────────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Create Sample StellarNode Manifests for All Node Types" \
  "stellar-wave,documentation,dx" \
  "### 🔴 Difficulty: High (200 Points)

Users need ready-to-use example manifests for every supported node type and configuration option.

### ✅ Acceptance Criteria
- Create \`examples/\` directory with manifests for: Validator (mainnet), Validator (testnet), Horizon (with ingress), SorobanRpc (with autoscaling), and a full DR-enabled setup.
- Each manifest should include inline comments explaining every field.
- Add a \`kubectl apply -f examples/\` smoke test to the CI pipeline.
- Link all examples from the README.

### 📚 Resources
- [Kubernetes Object Management](https://kubernetes.io/docs/concepts/overview/working-with-objects/object-management/)
- [\`examples/\`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/examples)"


# ─── 10 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Changelog Generation with conventional-changelog" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

The project needs an automated changelog to track changes across releases.

### ✅ Acceptance Criteria
- Adopt Conventional Commits format for all commit messages.
- Integrate \`git-cliff\` or \`conventional-changelog\` to auto-generate \`CHANGELOG.md\`.
- Add a CI step that validates commit messages against the convention.
- Generate the initial changelog from existing git history.

### 📚 Resources
- [Conventional Commits](https://www.conventionalcommits.org/)
- [git-cliff](https://git-cliff.org/)"


# ─── 11 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Unit Tests for Peer Discovery DNS Resolution Logic" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

The peer discovery module resolves DNS names to build the peer list. This DNS logic needs targeted unit tests.

### ✅ Acceptance Criteria
- Write tests for successful DNS resolution with multiple A records.
- Test fallback behavior when DNS returns NXDOMAIN.
- Test timeout handling for slow DNS responses.
- Mock the DNS resolver using a trait-based abstraction.

### 📚 Resources
- [Trust-DNS Documentation](https://docs.rs/trust-dns-resolver/latest/trust_dns_resolver/)
- [\`src/controller/peer_discovery.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/peer_discovery.rs)"


# ─── 12 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Helm values.yaml JSON Schema Validation" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

Users often misconfigure Helm values. A JSON schema would catch errors at \`helm install\` time.

### ✅ Acceptance Criteria
- Create a \`values.schema.json\` for the Helm chart.
- Define types, required fields, and enums for all configuration options.
- Verify that \`helm lint\` catches invalid values.
- Add the schema validation to the CI pipeline.

### 📚 Resources
- [Helm: Chart Configuration](https://helm.sh/docs/topics/charts/#schema-files)
- [JSON Schema Documentation](https://json-schema.org/)"


# ─── 13 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Security Policy and Vulnerability Disclosure Process" \
  "stellar-wave,documentation,security" \
  "### 🔴 Difficulty: High (200 Points)

A financial infrastructure project must have a clear security policy for responsible disclosure.

### ✅ Acceptance Criteria
- Create \`SECURITY.md\` with a vulnerability disclosure process.
- Set up a security advisory template on GitHub.
- Document the supported versions matrix.
- Add PGP key or security contact email for encrypted reports.

### 📚 Resources
- [GitHub Security Policy](https://docs.github.com/en/code-security/getting-started/adding-a-security-policy-to-your-repository)
- [CVE (Common Vulnerabilities and Exposures)](https://cve.mitre.org/)"


# ─── 14 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Namespace-Scoped vs Cluster-Scoped Operator Mode" \
  "stellar-wave,enhancement,reliability" \
  "### 🔴 Difficulty: High (200 Points)

Some users want the operator to only watch a single namespace. This should be a configurable mode.

### ✅ Acceptance Criteria
- Add a \`--watch-namespace\` CLI flag.
- When set, the operator should only watch \`StellarNode\` resources in that namespace.
- When unset (default), watch all namespaces.
- Update RBAC manifests to support both modes.
- Write integration tests for both modes.

### 📚 Resources
- [Kubernetes RBAC: Roles and RoleBindings](https://kubernetes.io/docs/reference/access-authn-authz/rbac/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 15 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Container Image Pinning and Digest Verification" \
  "stellar-wave,enhancement,security" \
  "### 🔴 Difficulty: High (200 Points)

Using mutable tags like \`latest\` is a security risk. The operator should support image digest pinning.

### ✅ Acceptance Criteria
- Support \`image@sha256:...\` format in the \`StellarNode\` spec \`version\` field.
- Add a webhook validation rule that warns if a mutable tag is used.
- Document best practices for image pinning in production.
- Add unit tests for image reference parsing.

### 📚 Resources
- [Kubernetes Images](https://kubernetes.io/docs/concepts/containers/images/)
- [Docker Image Digest](https://docs.docker.com/engine/reference/commandline/pull/#pull-an-image-by-digest-immutable-identifier)"


# ─── 16 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Dry-Run Mode for the Reconciler" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

The \`--dry-run\` flag exists but needs full implementation. The reconciler should calculate and log all changes without applying them.

### ✅ Acceptance Criteria
- When \`--dry-run\` is set, the reconciler should use server-side dry-run for all API calls.
- Log a diff of what would change (e.g., 'Would create Deployment X', 'Would update Service Y').
- Emit dry-run results as Kubernetes Events.
- Write tests verifying no mutations occur in dry-run mode.

### 📚 Resources
- [Kubernetes Dry-Run](https://kubernetes.io/docs/reference/using-api/api-concepts/#dry-run)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)"


# ─── 17 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Pre-commit Hooks for Code Quality Enforcement" \
  "stellar-wave,enhancement,dx" \
  "### 🔴 Difficulty: High (200 Points)

Catch formatting and lint issues before they reach CI by using pre-commit hooks.

### ✅ Acceptance Criteria
- Create a \`.pre-commit-config.yaml\` with hooks for: \`cargo fmt\`, \`cargo clippy\`, \`cargo test\`, trailing whitespace, and YAML lint.
- Document the setup in \`CONTRIBUTING.md\`.
- Add a CI check that verifies pre-commit hooks are passing.

### 📚 Resources
- [pre-commit framework](https://pre-commit.com/)
- [\`.pre-commit-config.yaml\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.pre-commit-config.yaml)"


# ─── 18 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Operator Startup Self-Test and Diagnostics" \
  "stellar-wave,enhancement,reliability" \
  "### 🔴 Difficulty: High (200 Points)

When the operator starts, it should run a suite of self-checks to verify the cluster is properly configured.

### ✅ Acceptance Criteria
- On startup, verify: CRD is installed, RBAC permissions are sufficient, required namespaces exist, and the leader election lease is accessible.
- Print a clear diagnostic summary to the log.
- If critical checks fail, exit with a descriptive error instead of crashing later.
- Add a \`--preflight-only\` CLI flag that runs checks and exits.

### 📚 Resources
- [Kubernetes API: SelfSubjectAccessReview](https://kubernetes.io/docs/reference/kubernetes-api/authorization-resources/self-subject-access-review-v1/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)"


# ─── 19 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add Kubernetes Event Annotations for Audit Trail" \
  "stellar-wave,enhancement,observability" \
  "### 🔴 Difficulty: High (200 Points)

Every reconciliation action should be recorded as a Kubernetes annotation on the resource for a permanent audit trail.

### ✅ Acceptance Criteria
- Add an annotation \`stellar.org/last-reconcile-time\` updated on each successful reconcile.
- Add \`stellar.org/last-action\` recording what the reconciler did (e.g., 'created-deployment', 'updated-service').
- Add \`stellar.org/operator-version\` to track which operator version last touched the resource.
- Write unit tests for annotation generation.

### 📚 Resources
- [Kubernetes Annotations](https://kubernetes.io/docs/concepts/overview/working-with-objects/annotations/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)"


# ─── 20 ───────────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Create Architecture Decision Records (ADR) Directory" \
  "stellar-wave,documentation,dx" \
  "### 🔴 Difficulty: High (200 Points)

Major design decisions should be recorded for posterity so future contributors understand the rationale behind choices.

### ✅ Acceptance Criteria
- Create a \`docs/adr/\` directory with an ADR template.
- Write ADRs for at least 3 existing decisions: choice of Rust, use of kube-rs finalizers, and the CRD versioning strategy.
- Each ADR should follow the standard format: Title, Status, Context, Decision, Consequences.
- Link the ADR directory from the README.

### 📚 Resources
- [Architecture Decision Records](https://adr.github.io/)
- [\`docs/adr/\`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/docs/adr)"


echo ""
echo "🎉 Batch 16 (20 x 200 pts) issues created successfully!"

print_skip_summary
