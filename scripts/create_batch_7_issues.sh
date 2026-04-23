#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

# Source shared retry/backoff and dry-run helper.
# shellcheck source=scripts/retry_helper.sh
source "$(dirname "$0")/retry_helper.sh"

echo "Ensuring required labels exist..."
for label in "stellar-wave" "testing" "documentation" "ci" "bug" "enhancement" "good-first-issue" "security" "performance" "reliability" "kubernetes" "observability" "dx"; do
  gh label create "$label" --repo "$REPO" --color "0075ca" 2>/dev/null || true
done

echo "Creating Batch 7 issues..."

create_issue \
  "Verify: Operator binary boots and connects to a live cluster without errors" \
  "stellar-wave,testing,good-first-issue" \
  "### 🟢 Difficulty: Trivial (50 Points)

Verify that the operator binary can be built and successfully connects to a live Kubernetes cluster (e.g., via kind or k3s).

### ✅ Acceptance Criteria
- Run cargo build --release and confirm it succeeds with zero errors.
- Start a local kind cluster and run the operator binary: ./target/release/stellar-operator run.
- Confirm the log line Connected to Kubernetes cluster appears.
- Document any runtime errors or missing env variables in a comment on this issue.

### 📚 Resources
- [kind - Kubernetes in Docker](https://kind.sigs.k8s.io/)
"
echo "✓ Issue 1 created"

create_issue \
  "Verify: kubectl-stellar plugin installs and executes correctly" \
  "stellar-wave,testing,good-first-issue,kubernetes" \
  "### 🟢 Difficulty: Trivial (50 Points)

The repo ships a kubectl-stellar binary. Verify it builds and works as a kubectl plugin.

### ✅ Acceptance Criteria
- Build with: cargo build --release --bin kubectl-stellar
- Copy the binary into your PATH: cp ./target/release/kubectl-stellar /usr/local/bin/
- Run kubectl stellar --help and confirm the help output is displayed.
- Document any sub-commands and their output in a comment.
"
echo "✓ Issue 2 created"

create_issue \
  "Verify: Helm chart lints cleanly and renders valid manifests" \
  "stellar-wave,testing,good-first-issue,kubernetes" \
  "### 🟢 Difficulty: Trivial (50 Points)

Ensure the Helm chart at charts/stellar-operator/ passes both lint and template rendering without warnings.

### ✅ Acceptance Criteria
- Run helm lint charts/stellar-operator/ -- must exit with no errors or warnings.
- Run helm template stellar charts/stellar-operator/ | kubectl apply --dry-run=client -f - -- must succeed.
- Document the rendered output as a comment on this issue.
"
echo "✓ Issue 3 created"

create_issue \
  "Verify: All example YAML manifests apply cleanly in dry-run mode" \
  "stellar-wave,testing,documentation" \
  "### 🟢 Difficulty: Trivial (50 Points)

The examples/ directory contains 11 YAML manifests. Verify they are all valid Kubernetes YAML that can be applied in dry-run mode.

### ✅ Acceptance Criteria
- Run for each manifest: kubectl apply --dry-run=client -f examples/<file>.yaml
- Document any manifests that fail validation with specific errors.
- If any are broken, fix and submit a PR.
"
echo "✓ Issue 4 created"

create_issue \
  "Verify: Prometheus metrics endpoint returns valid data when operator is running" \
  "stellar-wave,testing,observability" \
  "### 🟡 Difficulty: Medium (100 Points)

The operator exposes Prometheus metrics (enabled by the metrics feature). Verify the endpoint is reachable and returns well-formed metric data.

### ✅ Acceptance Criteria
- Build with the metrics feature: cargo build --release --features metrics
- Run the operator against a local kind cluster.
- curl http://localhost:9090/metrics should return valid Prometheus text format.
- Confirm the key metrics (e.g., stellar_reconcile_duration_seconds, error counters) are present.
- Document the full metric list as a comment.
"
echo "✓ Issue 5 created"

create_issue \
  "Verify: REST API server starts and health endpoint returns 200" \
  "stellar-wave,testing,good-first-issue" \
  "### 🟢 Difficulty: Trivial (50 Points)

The operator optionally starts an Axum REST API server. Verify it actually starts and the /healthz endpoint returns a 200 OK.

### ✅ Acceptance Criteria
- Build with: cargo build --release --features rest-api
- Run operator and verify with: curl -v http://localhost:8080/healthz
- Should return HTTP 200 with a JSON body.
- Document the full response as a comment.
"
echo "✓ Issue 6 created"

create_issue \
  "Implement Leader Election to prevent duplicate reconciliation in HA deployments" \
  "stellar-wave,enhancement,reliability" \
  "### 🔴 Difficulty: High (200 Points)

There is a TODO comment in src/main.rs (line 205-207) to re-enable leader election. Without it, running multiple operator replicas will cause duplicate reconciliations and resource conflicts.

### ✅ Acceptance Criteria
- Implement Kubernetes Lease-based leader election using kube's built-in locking primitives.
- Only the leader replica should run the controller reconciliation loop.
- Non-leader replicas should remain healthy (liveness probe returns 200) and be ready to take over.
- Add a /leader endpoint to the REST API that returns whether this replica is the active leader.
- Include an integration test that validates only one instance processes events when two replicas are running.
"
echo "✓ Issue 7 created"

create_issue \
  "Add unit tests for the peer discovery module" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

The src/controller/peer_discovery.rs module has no dedicated unit tests. Peer discovery is a critical path for validator nodes.

### ✅ Acceptance Criteria
- Add a peer_discovery_test.rs file with unit tests covering:
  - Peer list building from StellarNode CRDs
  - DNS lookups for peer addresses (mock the DNS client)
  - The peer scoring/selection algorithm
  - Edge cases: empty peer list, all peers unreachable
- Run cargo test and confirm all new tests pass.
"
echo "✓ Issue 8 created"

create_issue \
  "Add unit tests for the CVE remediation controller" \
  "stellar-wave,testing,security" \
  "### 🔴 Difficulty: High (200 Points)

src/controller/cve.rs and src/controller/cve_reconciler.rs implement automated CVE patching. This is a security-critical path with a test file (cve_test.rs) that likely needs expansion.

### ✅ Acceptance Criteria
- Review the existing src/controller/cve_test.rs and identify coverage gaps.
- Add tests for:
  - Parsing CVE scan results into remediation actions
  - The correct Kubernetes resources being patched/rolled
  - Ensuring vulnerable images are replaced with fixed versions
  - The dry-run path does not mutate any resources
- All tests must pass with cargo test.
"
echo "✓ Issue 9 created"

create_issue \
  "Add unit tests for the disaster recovery (DR) module" \
  "stellar-wave,testing,reliability" \
  "### 🟡 Difficulty: Medium (100 Points)

src/controller/dr.rs is a disaster recovery module with no test coverage. Add tests to validate the DR logic.

### ✅ Acceptance Criteria
- Add a dr_test.rs module (or inline tests in dr.rs) covering:
  - Triggering a DR failover when the primary region fails
  - Confirming backup targets are used in the correct priority order
  - Verifying that no-op is correct when everything is healthy
- All tests pass with cargo test.
"
echo "✓ Issue 10 created"

create_issue \
  "Expand the e2e test suite: apply a real StellarNode CRD and verify reconciliation" \
  "stellar-wave,testing,kubernetes" \
  "### 🔴 Difficulty: High (200 Points)

The existing tests/e2e_kind.rs file provides the scaffold for an e2e test suite, but needs to be expanded to test actual reconciliation of a StellarNode resource.

### ✅ Acceptance Criteria
- The test should:
  1. Start a kind cluster
  2. Install the CRDs from config/crd/
  3. Apply a sample StellarNode manifest
  4. Wait for the operator to create a Deployment and Service
  5. Assert that the StellarNode status transitions to Running
  6. Delete the resource and verify all child resources are cleaned up
- The test must be runnable with: cargo test --test e2e_kind -- --ignored
"
echo "✓ Issue 11 created"

create_issue \
  "Add rust-toolchain.toml to pin the minimum supported Rust version" \
  "stellar-wave,ci,good-first-issue,dx" \
  "### 🟢 Difficulty: Trivial (50 Points)

Currently, there is no rust-toolchain.toml in the repo. This causes CI failures when dependencies require a newer Rust version than what is installed in the environment.

### ✅ Acceptance Criteria
- Create a rust-toolchain.toml file at the repo root pinning the channel to stable and a minimum version (e.g., 1.88).
- Verify that the CI pipeline picks up the toolchain file automatically.
- Push a PR and confirm the lint job in ci.yml is green.
"
echo "✓ Issue 12 created"

create_issue \
  "Add CHANGELOG.md following Keep a Changelog conventions" \
  "stellar-wave,documentation,good-first-issue" \
  "### 🟢 Difficulty: Trivial (50 Points)

The project lacks a CHANGELOG.md, making it hard for users and contributors to track changes across versions.

### ✅ Acceptance Criteria
- Create CHANGELOG.md at the repo root following Keep a Changelog.
- Add an [Unreleased] section and a [0.1.0] section documenting the initial features.
- Link to the changelog from the README.md.
"
echo "✓ Issue 13 created"

create_issue \
  "Write architecture decision record (ADR) for the Wasm admission webhook design" \
  "stellar-wave,documentation" \
  "### 🟡 Difficulty: Medium (100 Points)

The Wasm-powered admission webhook is a sophisticated feature. An ADR explaining why this design was chosen (vs. a native webhook) would help new contributors understand the system.

### ✅ Acceptance Criteria
- Create docs/adr/0001-wasm-admission-webhook.md following the MADR format.
- Cover: context, the decision, consequences, and alternatives considered.
- Reference the existing docs/wasm-webhook.md guide.
- Add a link to docs/adr/README.md (create this index file if it doesn't exist).
"
echo "✓ Issue 14 created"

create_issue \
  "Add a DEVELOPMENT.md guide: local setup, building, and testing" \
  "stellar-wave,documentation,dx" \
  "### 🟡 Difficulty: Medium (100 Points)

New contributors have no single guide for setting up a local development environment. Create one.

### ✅ Acceptance Criteria
- Create DEVELOPMENT.md at the repo root covering:
  - Prerequisites (Rust, Docker, kind, kubectl, helm)
  - Building all binaries (stellar-operator and kubectl-stellar)
  - Running unit tests: cargo test
  - Running the operator locally against a kind cluster
  - Running the e2e tests
  - Useful make targets from Makefile
- Verify all commands in the guide actually work before submitting the PR.
"
echo "✓ Issue 15 created"

create_issue \
  "Add dry-run integration test: verify no Kubernetes resources are mutated when --dry-run is set" \
  "stellar-wave,testing,reliability" \
  "### 🟡 Difficulty: Medium (100 Points)

The operator accepts a --dry-run flag. There is no automated test that verifies this flag actually prevents mutations. This is a correctness regression risk.

### ✅ Acceptance Criteria
- Add a test (in tests/ or src/controller/) that:
  - Starts the operator with dry_run: true
  - Creates a StellarNode resource
  - Confirms that NO child resources (Deployment, Service, etc.) were created
  - Confirms the operator ran without panicking
- All tests pass with cargo test.
"
echo "✓ Issue 16 created"

create_issue \
  "Verify and document the mTLS setup for the REST API" \
  "stellar-wave,security,testing,documentation" \
  "### 🟡 Difficulty: Medium (100 Points)

The operator supports mTLS for its REST API via the --enable-mtls flag. This flow needs to be verified end-to-end and documented.

### ✅ Acceptance Criteria
- Run the operator locally with --enable-mtls.
- Confirm the operator generates CA + server certificates and stores them as Kubernetes Secrets.
- Test a curl client call using the CA cert: curl --cacert ca.crt https://localhost:8443/healthz
- Document the steps in docs/mtls-guide.md.
"
echo "✓ Issue 17 created"

create_issue \
  "Optimize Dockerfile build: switch to cargo-chef workspace-aware caching" \
  "stellar-wave,enhancement,performance" \
  "### 🟡 Difficulty: Medium (100 Points)

The current Dockerfile may not be using cargo-chef optimally for workspaces with multiple binaries (stellar-operator and kubectl-stellar), resulting in longer CI build times.

### ✅ Acceptance Criteria
- Review the current Dockerfile and identify any cache-busting inefficiencies.
- Ensure both stellar-operator and kubectl-stellar binaries are built in the same RUN cargo build --release step.
- Measure and document the before/after build time.
- The final Docker image must contain both binaries.
"
echo "✓ Issue 18 created"

create_issue \
  "Add structured tracing spans to the reconciler using #[instrument]" \
  "stellar-wave,observability,enhancement" \
  "### 🟡 Difficulty: Medium (100 Points)

The reconciler handles many operations but doesn't consistently annotate all sub-functions with #[instrument]. This means OpenTelemetry traces are incomplete and hard to debug.

### ✅ Acceptance Criteria
- Audit src/controller/reconciler.rs for functions missing #[instrument] annotations.
- Add #[instrument(skip(ctx, client), fields(node = %name))] to all major async functions.
- Verify traces appear in a local Jaeger instance.
- Screenshot or document the resulting trace in the PR description.
"
echo "✓ Issue 19 created"

create_issue \
  "Add Grafana dashboard JSON for operator metrics" \
  "stellar-wave,observability,documentation" \
  "### 🟡 Difficulty: Medium (100 Points)

The operator emits Prometheus metrics but there is no pre-built Grafana dashboard to visualize them. Adding one will significantly lower the bar for operating this in production.

### ✅ Acceptance Criteria
- Create monitoring/grafana-dashboard.json with panels for:
  - Reconciliation rate and duration (p50, p95, p99)
  - Error rate by error type
  - Number of managed StellarNodes per type
  - Memory and CPU usage of the operator pod
- The dashboard JSON must be importable directly into Grafana.
- Add a section to README.md on how to import the dashboard.
"
echo "✓ Issue 20 created"

echo ""
echo "🎉 All 20 Batch 7 issues created successfully!"
