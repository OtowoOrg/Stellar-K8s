#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

# Source shared retry/backoff and dry-run helper.
# shellcheck source=scripts/retry_helper.sh
source "$(dirname "$0")/retry_helper.sh"

echo "Creating Batch 8 (10 x 200 pts) issues..."

create_issue \
  "Add comprehensive unit tests for the main reconciler loop" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

src/controller/reconciler.rs is the heart of the operator. It has no dedicated test file. This is the highest-risk untested path in the entire codebase.

### ✅ Acceptance Criteria
- Create src/controller/reconciler_test.rs with tests covering:
  - The apply_stellar_node path when all resources are created fresh
  - The apply_stellar_node path when resources already exist (idempotency)
  - The cleanup_stellar_node path (finalizer removal + child resource deletion)
  - Status transitions: Pending -> Running -> Terminating
  - The error_policy function returns a requeue Action
- Use mocked Kubernetes API calls (no real cluster required).
- All tests pass with cargo test.
"
echo "✓ Issue 1 created"

create_issue \
  "Add unit tests for the automated remediation module" \
  "stellar-wave,testing,reliability,security" \
  "### 🔴 Difficulty: High (200 Points)

src/controller/remediation.rs handles automated incident response for Stellar nodes. It has no test coverage -- a regression here would cause silent failures during incidents.

### ✅ Acceptance Criteria
- Add a remediation_test.rs or inline tests covering:
  - Triggering a remediation action when health check fails
  - Confirming idempotency: re-running remediation on an already-remediated node is a no-op
  - Verifying the cooldown period prevents rapid re-remediation
  - Verifying the correct Kubernetes resource is selected per remediation type
- All tests pass with cargo test.
"
echo "✓ Issue 2 created"

create_issue \
  "Add unit tests for the scheduler scoring algorithm" \
  "stellar-wave,testing,performance" \
  "### 🔴 Difficulty: High (200 Points)

The latency-aware scheduler (src/scheduler/scoring.rs) selects nodes based on a scoring function. This logic needs thorough unit tests to ensure placement decisions are correct.

### ✅ Acceptance Criteria
- Add tests in src/scheduler/scoring.rs (or a separate scoring_test.rs) covering:
  - A node with lower latency scores higher than a node with higher latency
  - Nodes that exceed the latency threshold are excluded entirely
  - Ties are broken deterministically (e.g., alphabetically by node name)
  - An empty node list returns no selection
- All tests pass with cargo test.
"
echo "✓ Issue 3 created"

create_issue \
  "Add unit tests for the Wasm admission webhook validation logic" \
  "stellar-wave,testing,security" \
  "### 🔴 Difficulty: High (200 Points)

The Wasm-powered admission webhook (src/webhook/) validates StellarNode manifests at admission time. Bugs here mean invalid resources can enter the cluster silently.

### ✅ Acceptance Criteria
- Add tests covering the webhook validation logic in src/webhook/types.rs / runtime.rs:
  - A valid StellarNode spec is admitted (returns Allowed: true)
  - A spec with an invalid nodeType is rejected with a descriptive message
  - A spec missing required fields is rejected
  - A Wasm plugin that panics is handled gracefully (operator doesn't crash)
- All tests pass with cargo test --features admission-webhook.
"
echo "✓ Issue 4 created"

create_issue \
  "Add integration tests for the backup scheduler and restore flow" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

src/backup/ implements backup scheduling and restore logic for Stellar node data. There are no tests verifying these flows work end-to-end correctly.

### ✅ Acceptance Criteria
- Add tests in src/backup/ covering:
  - A scheduled backup triggers at the correct interval
  - Backup metadata is correctly serialised and stored
  - A restore operation reads the latest backup and applies it correctly
  - An invalid/corrupt backup is detected and the restore is aborted safely
- Use mocked storage providers (no real S3/GCS required).
- All tests pass with cargo test.
"
echo "✓ Issue 5 created"

create_issue \
  "Add unit tests for the cross-cluster controller" \
  "stellar-wave,testing,reliability,kubernetes" \
  "### 🔴 Difficulty: High (200 Points)

src/controller/cross_cluster.rs manages multi-cluster state synchronization. This module is entirely untested.

### ✅ Acceptance Criteria
- Add tests covering:
  - Successfully registering a remote cluster endpoint
  - Detecting when a remote cluster becomes unreachable
  - Confirming that cross-cluster status is propagated to the StellarNodeStatus
  - The fallback/retry logic when a cross-cluster API call fails
- All tests pass with cargo test.
"
echo "✓ Issue 6 created"

create_issue \
  "Add unit tests for the captive core configuration manager" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

src/controller/captive_core.rs manages Stellar captive core configuration. Captive core is required for Soroban RPC nodes and has no test coverage.

### ✅ Acceptance Criteria
- Add tests covering:
  - Generating a valid stellar-core.cfg from a StellarNode spec
  - Updating the config when the spec changes (ConfigMap update)
  - Verifying that network-specific settings (Mainnet vs Testnet) produce different configs
  - Handling missing optional fields gracefully
- All tests pass with cargo test.
"
echo "✓ Issue 7 created"

create_issue \
  "Add unit tests for the traffic shaping / rate-limiting controller" \
  "stellar-wave,testing,performance,reliability" \
  "### 🔴 Difficulty: High (200 Points)

src/controller/traffic.rs controls traffic policies for Stellar nodes. Without tests, regressions in traffic shaping could silently degrade network performance.

### ✅ Acceptance Criteria
- Add tests covering:
  - Generating correct NetworkPolicy manifests from a StellarNode spec
  - Rate-limit annotations are applied to the correct pods
  - Updating traffic policy when the spec changes
  - Verifying that ingress and egress rules are independent and correct
- All tests pass with cargo test.
"
echo "✓ Issue 8 created"

create_issue \
  "Add unit tests for the archive health checker" \
  "stellar-wave,testing,reliability,observability" \
  "### 🔴 Difficulty: High (200 Points)

src/controller/archive_health.rs checks the health of Stellar history archives. Archive health is critical for validators syncing new nodes. It has zero test coverage.

### ✅ Acceptance Criteria
- Add tests covering:
  - A reachable archive returns a healthy status
  - An unreachable archive is marked as unhealthy and triggers an alert annotation
  - An archive with a stale stellar-history.json is flagged as degraded
  - The check respects a configurable timeout (mock reqwest)
- All tests pass with cargo test.
"
echo "✓ Issue 9 created"

create_issue \
  "Implement automated end-to-end test: deploy Horizon node and verify REST API responds" \
  "stellar-wave,testing,kubernetes" \
  "### 🔴 Difficulty: High (200 Points)

Extend the e2e test suite (tests/e2e_kind.rs) with a full Horizon node lifecycle test. This validates the most common production use case.

### ✅ Acceptance Criteria
- The e2e test should:
  1. Apply the examples/horizon-with-health-check.yaml manifest to a kind cluster
  2. Wait for the operator to reconcile and the pod to become Ready
  3. Port-forward to the Horizon pod and curl http://localhost:8000/ -- must return HTTP 200
  4. Verify the StellarNode status shows phase: Running
  5. Delete the resource and verify pods + services are cleaned up within 60 seconds
- Runnable with: cargo test --test e2e_kind -- --ignored
"
echo "✓ Issue 10 created"

echo ""
echo "🎉 All 10 Batch 8 (200 pts) issues created!"
