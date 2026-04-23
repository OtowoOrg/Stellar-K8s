#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

# Source shared retry/backoff and dry-run helper.
# shellcheck source=scripts/retry_helper.sh
source "$(dirname "$0")/retry_helper.sh"

echo "Resuming Batch 9 (final 3 issues)..."

# Alias kept for readability; delegates to the shared helper.
create_issue_with_retry() { create_issue "$1" "$2" "$3"; }

# ─── ISSUE 5 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement State-Machine Fuzzer for the Reconciler" \
  "stellar-wave,testing,reliability" \
  "### 🔴 Difficulty: High (200 Points)

To guarantee the operator never panics under extreme or malformed conditions, we need property-based testing and fuzzing for the core reconciler state machine.

### ✅ Acceptance Criteria
- Integrate \`cargo-fuzz\` or \`proptest\` into the workspace.
- Create a test target that generates random mutations of \`StellarNodeSpec\` and random sequences of Kubernetes mock events (Node added, Pod deleted, ConfigMap modified).
- Assure that feeding these rapid, conflicting events into the \`reconcile\` function:
  1. Never causes a Rust panic.
  2. Eventually converges to an error state or a resolved healthy state.
- Document how to run the fuzzer locally in the README or a designated doc.

### 📚 Resources
- [\`cargo-fuzz\`](https://rust-fuzz.github.io/book/cargo-fuzz.html)
- [\`proptest\`](https://altsysrq.github.io/proptest-book/intro.html)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)
"

# ─── ISSUE 6 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Implement Zero-Downtime Ledger Snapshots via CSI VolumeSnapshots" \
  "stellar-wave,enhancement,kubernetes,performance" \
  "### 🔴 Difficulty: High (200 Points)

Syncing a new Stellar Validator from a history archive can take hours or days. We want to support nearly instant bootstrapping of new nodes by taking live disk snapshots using [CSI VolumeSnapshots](https://kubernetes.io/docs/concepts/storage/volume-snapshots/).

### ✅ Acceptance Criteria
- Add a new CRD or update \`StellarNode\` to support a \`snapshotSchedule\` and \`restoreFromSnapshot\` field.
- When an operator schedules a snapshot, it must:
  1. Briefly lock/flush the Stellar database gracefully (if required for DB consistency).
  2. Emit a \`VolumeSnapshot\` Kubernetes resource targeting the node's PVC.
  3. Resume normal operations.
- When a new node is created with \`restoreFromSnapshot\`, its PVC must be built from the specified VolumeSnapshot instead of starting empty.
- Provide full documentation and a YAML example.

### 📚 Resources
- [Volume Snapshots](https://kubernetes.io/docs/concepts/storage/volume-snapshots/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)
"

# ─── ISSUE 7 ───────────────────────────────────────────────────────────────────
create_issue_with_retry \
  "Create an Operator Lifecycle Manager (OLM) bundle" \
  "stellar-wave,enhancement,dx,kubernetes" \
  "### 🔴 Difficulty: High (200 Points)

To make Stellar-K8s discoverable and easily installable on enterprise Kubernetes distributions (like OpenShift), we should package the operator as an OLM (Operator Lifecycle Manager) Bundle, suitable for publishing to OperatorHub.io.

### ✅ Acceptance Criteria
- Generate a \`ClusterServiceVersion\` (CSV) YAML file describing the operator, its permissions, and managed CRDs.
- Generate the correct \`bundle/\` directory structure containing the metadata, CRDs, and CSV.
- Provide a \`Makefile\` target (\`make bundle\`) that packages the operator using the Operator SDK (\`operator-sdk generate bundle\`).
- Verify the bundle builds successfully and passes \`operator-sdk bundle validate\`.
- Add documentation on how to deploy using OLM.

### 📚 Resources
- [Operator Lifecycle Manager](https://olm.operatorframework.io/)
- [OperatorHub.io](https://operatorhub.io/)
"

echo ""
echo "🎉 All final 3 Batch 9 (200 pts) issues created!"
