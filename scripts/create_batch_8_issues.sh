#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/repo.sh"
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 8, 10 x 200 pts).

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

EXPECTED_ISSUE_COUNT=10
ACTUAL_ISSUE_COUNT=$(grep -c '^gh issue create' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT. Update EXPECTED_ISSUE_COUNT or fix the script." >&2
  exit 1
fi

echo "Creating Batch 8 (10 x 200 pts) issues..."

# ─── ISSUE 1 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add comprehensive unit tests for the main reconciler loop" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

\`src/controller/reconciler.rs\` is the heart of the operator at ~60KB. It has no dedicated test file. This is the highest-risk untested path in the entire codebase.

### ✅ Acceptance Criteria
- Create \`src/controller/reconciler_test.rs\` with tests covering:
  - The \`apply_stellar_node\` path when all resources are created fresh
  - The \`apply_stellar_node\` path when resources already exist (idempotency)
  - The \`cleanup_stellar_node\` path (finalizer removal + child resource deletion)
  - Status transitions: \`Pending\` → \`Running\` → \`Terminating\`
  - The \`error_policy\` function returns a requeue \`Action\`
- Use mocked Kubernetes API calls (no real cluster required).
- All tests pass with \`cargo test\`.

### 📚 Resources
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)
- [kube-rs mock client](https://docs.rs/kube/latest/kube/client/struct.Client.html)
"

# ─── ISSUE 2 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add unit tests for the automated remediation module" \
  "stellar-wave,testing,reliability,security" \
  "### 🔴 Difficulty: High (200 Points)

\`src/controller/remediation.rs\` handles automated incident response for Stellar nodes (restarts, failovers, etc). It has no test coverage — a regression here would cause silent failures during incidents.

### ✅ Acceptance Criteria
- Add a \`remediation_test.rs\` or inline tests covering:
  - Triggering a remediation action when health check fails
  - Confirming idempotency: re-running remediation on an already-remediated node is a no-op
  - Verifying the cooldown period prevents rapid re-remediation
  - Verifying the correct Kubernetes resource (Pod restart vs. Deployment rollout) is selected per remediation type
- All tests pass with \`cargo test\`.

### 📚 Resources
- [\`src/controller/remediation.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/remediation.rs)
"

# ─── ISSUE 3 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add unit tests for the scheduler scoring algorithm" \
  "stellar-wave,testing,performance" \
  "### 🔴 Difficulty: High (200 Points)

The latency-aware scheduler (\`src/scheduler/scoring.rs\`) selects nodes based on a scoring function. This logic needs thorough unit tests to ensure placement decisions are correct.

### ✅ Acceptance Criteria
- Add tests in \`src/scheduler/scoring.rs\` (or a separate \`scoring_test.rs\`) covering:
  - A node with lower latency scores higher than a node with higher latency
  - Nodes that exceed the latency threshold are excluded entirely
  - Ties are broken deterministically (e.g., alphabetically by node name)
  - An empty node list returns no selection
- All tests pass with \`cargo test\`.

### 📚 Resources
- [\`src/scheduler/scoring.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/scheduler/scoring.rs)
- [\`src/scheduler/core.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/scheduler/core.rs)
"

# ─── ISSUE 4 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add unit tests for the Wasm admission webhook validation logic" \
  "stellar-wave,testing,security" \
  "### 🔴 Difficulty: High (200 Points)

The Wasm-powered admission webhook (\`src/webhook/\`) validates \`StellarNode\` manifests at admission time. Bugs here mean invalid resources can enter the cluster silently.

### ✅ Acceptance Criteria
- Add tests covering the webhook validation logic in \`src/webhook/types.rs\` / \`runtime.rs\`:
  - A valid \`StellarNode\` spec is admitted (returns \`Allowed: true\`)
  - A spec with an invalid \`nodeType\` is rejected with a descriptive message
  - A spec missing required fields is rejected
  - A Wasm plugin that panics is handled gracefully (operator doesn't crash)
- All tests pass with \`cargo test --features admission-webhook\`.

### 📚 Resources
- [\`src/webhook/\`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/src/webhook)
"

# ─── ISSUE 5 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add integration tests for the backup scheduler and restore flow" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

\`src/backup/\` implements backup scheduling and restore logic for Stellar node data. There are no tests verifying these flows work end-to-end correctly.

### ✅ Acceptance Criteria
- Add tests in \`src/backup/\` covering:
  - A scheduled backup triggers at the correct interval
  - Backup metadata is correctly serialised and stored
  - A restore operation reads the latest backup and applies it correctly
  - An invalid/corrupt backup is detected and the restore is aborted safely
- Use mocked storage providers (no real S3/GCS required).
- All tests pass with \`cargo test\`.

### 📚 Resources
- [\`src/backup/\`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/src/backup)
"

# ─── ISSUE 6 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add unit tests for the cross-cluster controller" \
  "stellar-wave,testing,reliability,kubernetes" \
  "### 🔴 Difficulty: High (200 Points)

\`src/controller/cross_cluster.rs\` manages multi-cluster state synchronization at ~15KB. This module is entirely untested.

### ✅ Acceptance Criteria
- Add tests covering:
  - Successfully registering a remote cluster endpoint
  - Detecting when a remote cluster becomes unreachable
  - Confirming that cross-cluster status is propagated to the \`StellarNodeStatus\`
  - The fallback/retry logic when a cross-cluster API call fails
- All tests pass with \`cargo test\`.

### 📚 Resources
- [\`src/controller/cross_cluster.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/cross_cluster.rs)
- [\`examples/cross-cluster-direct-ip.yaml\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/cross-cluster-direct-ip.yaml)
"

# ─── ISSUE 7 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add unit tests for the captive core configuration manager" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

\`src/controller/captive_core.rs\` manages Stellar captive core configuration at ~15KB. Captive core is required for Soroban RPC nodes and has no test coverage.

### ✅ Acceptance Criteria
- Add tests covering:
  - Generating a valid \`stellar-core.cfg\` from a \`StellarNode\` spec
  - Updating the config when the spec changes (ConfigMap update)
  - Verifying that network-specific settings (Mainnet vs Testnet) produce different configs
  - Handling missing optional fields gracefully (e.g., \`captive_core_config\` is \`None\`)
- All tests pass with \`cargo test\`.

### 📚 Resources
- [\`src/controller/captive_core.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/captive_core.rs)
"

# ─── ISSUE 8 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add unit tests for the traffic shaping / rate-limiting controller" \
  "stellar-wave,testing,performance,reliability" \
  "### 🔴 Difficulty: High (200 Points)

\`src/controller/traffic.rs\` controls traffic policies for Stellar nodes. Without tests, regressions in traffic shaping could silently degrade network performance.

### ✅ Acceptance Criteria
- Add tests covering:
  - Generating correct \`NetworkPolicy\` manifests from a \`StellarNode\` spec
  - Rate-limit annotations are applied to the correct pods
  - Updating traffic policy when the spec changes
  - Verifying that ingress and egress rules are independent and correct
- All tests pass with \`cargo test\`.

### 📚 Resources
- [\`src/controller/traffic.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/traffic.rs)
"

# ─── ISSUE 9 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Add unit tests for the archive health checker" \
  "stellar-wave,testing,reliability,observability" \
  "### 🔴 Difficulty: High (200 Points)

\`src/controller/archive_health.rs\` checks the health of Stellar history archives (~8KB). Archive health is critical for validators syncing new nodes. It has zero test coverage.

### ✅ Acceptance Criteria
- Add tests covering:
  - A reachable archive returns a healthy status
  - An unreachable archive is marked as unhealthy and triggers an alert annotation
  - An archive with a stale \`.well-known/stellar-history.json\` is flagged as degraded
  - The check respects a configurable timeout (mock \`reqwest\`)
- All tests pass with \`cargo test\`.

### 📚 Resources
- [\`src/controller/archive_health.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/archive_health.rs)
"

# ─── ISSUE 10 ──────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement automated end-to-end test: deploy Horizon node and verify REST API responds" \
  "stellar-wave,testing,kubernetes" \
  "### 🔴 Difficulty: High (200 Points)

Extend the e2e test suite (\`tests/e2e_kind.rs\`) with a full Horizon node lifecycle test. This validates the most common production use case.

### ✅ Acceptance Criteria
- The e2e test should:
  1. Apply the \`examples/horizon-with-health-check.yaml\` manifest to a \`kind\` cluster
  2. Wait for the operator to reconcile and the pod to become \`Ready\`
  3. Port-forward to the Horizon pod and \`curl http://localhost:8000/\` — must return HTTP 200
  4. Verify the \`StellarNode\` status shows \`phase: Running\`
  5. Delete the resource and verify pods + services are cleaned up within 60 seconds
- Runnable with: \`cargo test --test e2e_kind -- --ignored\`

### 📚 Resources
- [\`tests/e2e_kind.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/tests/e2e_kind.rs)
- [\`examples/horizon-with-health-check.yaml\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/horizon-with-health-check.yaml)
"

echo ""
echo "🎉 All 10 Batch 8 (200 pts) issues created!"

print_skip_summary
