#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
source "$(dirname "$0")/lib/repo.sh"

# Update Stellar Wave Issues with Points, Criteria, and Resources
# Updates issues #2 through #11

echo "Updating Stellar Wave issues..."

# Issue 2: Unit Tests (Trivial - 100 Points)
gh issue edit 2 --repo "$REPO" \
  --title "Add unit tests for StellarNodeSpec validation" \
  --body "### 🟢 Difficulty: Trivial (100 Points)

The \`StellarNodeSpec::validate()\` function currently checks for missing configurations. We need comprehensive unit tests to ensure it correctly accepts valid configs and rejects invalid ones (e.g., Validator with >1 replica).

### ✅ Acceptance Criteria
- Create \`src/crd/tests.rs\` (or add to \`stellar_node.rs\`)
- Test cases for: valid validator, missing validator config, multi-replica validator (fail), missing horizon config.

### 📚 Resources
- [Rust Testing Guide](https://doc.rust-lang.org/book/ch11-01-writing-tests.html)
- [Stellar-K8s CRD Definition](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/stellar_node.rs)
"

# Issue 3: Ready Replicas Status (Trivial - 100 Points)
# REPLACING "Display Trait" with meaningful logic fix
gh issue edit 3 --repo "$REPO" \
  --title "Implement correct readyReplicas status reporting" \
  --body "### 🟢 Difficulty: Trivial (100 Points)

Currently, the operator reports \`replicas\` from the spec, but does not report the actual number of **ready** replicas from the underlying Deployment or StatefulSet. This information is critical for checking rollout status.

### ✅ Acceptance Criteria
- Modify \`reconciler.rs\` to fetch the status of the created Deployment (RPC) or StatefulSet (Validator).
- Extract \`status.readyReplicas\`.
- Populate \`StellarNodeStatus.ready_replicas\` with this real-time value.

### 📚 Resources
- [k8s-openapi DeploymentStatus](https://docs.rs/k8s-openapi/latest/k8s_openapi/api/apps/v1/struct.DeploymentStatus.html)
- [kube-rs API](https://docs.rs/kube/latest/kube/)
"

# Issue 4: Cargo Audit (Trivial - 100 Points)
gh issue edit 4 --repo "$REPO" \
  --title "Add GitHub Action for Cargo Audit" \
  --body "### 🟢 Difficulty: Trivial (100 Points)

We need to ensure our dependencies are secure. Add a step to the CI pipeline to run \`cargo audit\`.

### ✅ Acceptance Criteria
- Update \`.github/workflows/ci.yml\`.
- Add a job that installs and runs \`cargo-audit\`.
- Fail build on vulnerabilities.

### 📚 Resources
- [RustSec: cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit)
- [GitHub Actions Documentation](https://docs.github.com/en/actions)
"

# Issue 5: Ledger Sequence Metrics (Medium - 150 Points)
gh issue edit 5 --repo "$REPO" \
  --title "Expose Ledger Sequence in Prometheus Metrics" \
  --body "### 🟡 Difficulty: Medium (150 Points)

The operator exposes basic metrics, but we need to track the \`ledger_sequence\` from the node status to monitor sync progress via Prometheus.

### ✅ Acceptance Criteria
- Add a \`stellar_node_ledger_sequence\` gauge metric in \`src/controller/metrics.rs\` (needs to be created).
- Update the metric value during the reconciliation loop.
- Ensure it is exported on the metrics port.

### 📚 Resources
- [prometheus-client crate](https://docs.rs/prometheus-client/latest/prometheus_client/)
- [Stellar Node Monitoring](https://developers.stellar.org/docs/run-core-node/monitoring)
"

# Issue 6: Retention Policy (Medium - 150 Points)
gh issue edit 6 --repo "$REPO" \
  --title "Add retentionPolicy support for specific Storage Classes" \
  --body "### 🟡 Difficulty: Medium (150 Points)

Extend the \`StorageConfig\` struct to allow specifying a custom \`volumeBindingMode\` or other storage-class specific parameters via annotations.

### ✅ Acceptance Criteria
- Add \`annotations: Option<BTreeMap<String, String>>\` to \`StorageConfig\`.
- Propagate these annotations to the created PVC in \`resources.rs\`.

### 📚 Resources
- [Kubernetes Persistent Volumes](https://kubernetes.io/docs/concepts/storage/persistent-volumes/)
- [kube-rs API docs](https://docs.rs/kube/latest/kube/)
"

# Issue 7: Suspended State (Medium - 150 Points)
gh issue edit 7 --repo "$REPO" \
  --title "Implement Suspended State correctly for Validators" \
  --body "### 🟡 Difficulty: Medium (150 Points)

Currently, setting \`suspended: true\` scales replicas to 0. For Validators (StatefulSets), this works, but we should also ensure the Service is untouched so peer discovery (if external) remains valid, or decide if it should be removed.

### ✅ Acceptance Criteria
- Discuss/Define desired behavior for suspended validators.
- Implement logic to perhaps label the node as 'offline' in Stellar terms if possible, or ensure the StatefulSet scales to 0 cleanly without error logs.

### 📚 Resources
- [Kubernetes StatefulSets](https://kubernetes.io/docs/concepts/workloads/controllers/statefulset/)
- [Stellar Node Lifecycle](https://developers.stellar.org/docs/run-core-node/prerequisites)
"

# Issue 8: Grafana Dashboard (Medium - 150 Points)
gh issue edit 8 --repo "$REPO" \
  --title "Create a Grafana Dashboard JSON for Stellar Nodes" \
  --body "### 🟡 Difficulty: Medium (150 Points)

Create a standard Grafana dashboard visualization for the metrics exported by the operator (and the Stellar nodes themselves if scraped).

### ✅ Acceptance Criteria
- Create \`monitoring/grafana-dashboard.json\`.
- Panels for: Node availability, CPU/Memory usage, Ledger sequence (if available), Peer count.

### 📚 Resources
- [Grafana Dashboard Basics](https://grafana.com/docs/grafana/latest/dashboards/)
- [Stellar Core Metrics Info](https://developers.stellar.org/docs/run-core-node/monitoring#metrics)
"

# Issue 9: Soroban Config (High - 200 Points)
gh issue edit 9 --repo "$REPO" \
  --title "Implement Soroban Captive Core Configuration Generator" \
  --body "### 🔴 Difficulty: High (200 Points)

Soroban RPC needs a Captive Core config. Instead of passing a raw string, we should generate the TOML configuration from structured fields in the CRD (e.g., \`network_passphrase\`, \`history_archive_urls\`).

### ✅ Acceptance Criteria
- Create a builder struct for Captive Core config.
- Generate the TOML file and inject it into the ConfigMap.
- Update \`StellarNodeSpec\` to optionally take structured config instead of raw string.

### 📚 Resources
- [Soroban Captive Core Architecture](https://developers.stellar.org/docs/data-availability/captive-core)
- [Stellar Core Configuration](https://github.com/stellar/stellar-core/blob/master/docs/software/admin.md#configuration)
"

# Issue 10: History Archive Check (High - 200 Points)
gh issue edit 10 --repo "$REPO" \
  --title "Add Automated History Archive Health Check with Retry" \
  --body "### 🔴 Difficulty: High (200 Points)

Before starting a validator, the operator should verify that the configured \`history_archive_urls\` are reachable.

### ✅ Acceptance Criteria
- Implement an async check in the reconciliation loop (only on startup/update).
- If unreachable, emit a Kubernetes Event (Warning) and block start until reachable (or exponential backoff).
- Use \`reqwest\` or \`hyper\` to ping the archive root.

### 📚 Resources
- [History Archive Documentation](https://developers.stellar.org/docs/run-core-node/publishing-history-archives)
- [kube-rs Events API](https://docs.rs/kube/latest/kube/api/struct.Event.html)
"

# Issue 11: Leader Election (High - 200 Points)
gh issue edit 11 --repo "$REPO" \
  --title "Implement Leader Election for High Availability Operator" \
  --body "### 🔴 Difficulty: High (200 Points)

To run multiple replicas of the \`stellar-operator\` itself, we need leader election to prevent split-brain reconciliation.

### ✅ Acceptance Criteria
- Use \`kube-rs\`'s \`coordination.k8s.io\` leader election pattern.
- Only the active leader should run the reconciliation loop.
- Standby instances should just serve the read-only API (if safe) or wait.

### 📚 Resources
- [kube-rs Leader Election Guide](https://kube.rs/controllers/leader-election/)
- [Kubernetes Leases](https://kubernetes.io/docs/concepts/architecture/leases/)
"

echo "Issues updated successfully."
