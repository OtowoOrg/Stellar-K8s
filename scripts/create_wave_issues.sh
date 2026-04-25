#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
source "$(dirname "$0")/lib/repo.sh"

# Stellar-K8s Wave Issue Creation Script
# Uses gh CLI to create issues defined in WAVE_ISSUES.md

# Helper to create label if not exists
create_label() {
  gh label create --repo "$REPO" "$1" --color "$2" --description "$3" || true
}

echo "Ensuring labels exist..."
create_label "stellar-wave" "1d76db" "Stellar Wave Program"
create_label "good-first-issue" "7057ff" "Good for newcomers"
create_label "testing" "C2E0C6" "Tests"
create_label "rust" "DEA584" "Rust related"
create_label "ci" "0075ca" "CI/CD"
create_label "security" "d73a4a" "Security related"
create_label "observability" "C2E0C6" "Metrics and logs"
create_label "feature" "a2eeef" "New feature"
create_label "kubernetes" "326ce5" "Kubernetes related"
create_label "bug" "d73a4a" "Something isn't working"
create_label "logic" "5319e7" "Business logic"
create_label "documentation" "0075ca" "Improvements or additions to documentation"
create_label "soroban" "7F129E" "Soroban smart contracts"
create_label "reliability" "d93f0b" "Reliability and stability"
create_label "architecture" "0e8a16" "Architecture design"

echo "Creating Stellar Wave issues..."

# 1. Add unit tests for StellarNodeSpec validation
gh issue create --repo "$REPO" \
  --title "Add unit tests for StellarNodeSpec validation" \
  --body "The \`StellarNodeSpec::validate()\` function currently checks for missing configurations. We need comprehensive unit tests to ensure it correctly accepts valid configs and rejects invalid ones (e.g., Validator with >1 replica).

**Acceptance Criteria:**
- Create \`src/crd/tests.rs\` (or add to \`stellar_node.rs\`)
- Test cases for: valid validator, missing validator config, multi-replica validator (fail), missing horizon config.

### 📚 Resources
- [\`src/crd/stellar_node.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/stellar_node.rs)
- [Rust Unit Testing Guide](https://doc.rust-lang.org/book/ch11-01-writing-tests.html)" \
  --label "stellar-wave,good-first-issue,testing" || echo "Failed to create issue 1"

# 2. Implement Display trait for StellarNetwork
gh issue create --repo "$REPO" \
  --title "Implement Display trait for StellarNetwork" \
  --body "Currently, \`StellarNetwork\` relies on \`Debug\` or \`serde\` for string representation. Implementing \`std::fmt::Display\` will allow for cleaner logging and status messages.

**Acceptance Criteria:**
- Implement \`Display\` for \`StellarNetwork\` enum.
- Update logs in \`reconciler.rs\` to use the new Display implementation.

### 📚 Resources
- [Rust Display Trait Documentation](https://doc.rust-lang.org/std/fmt/trait.Display.html)
- [\`src/crd/types.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/crd/types.rs)" \
  --label "stellar-wave,good-first-issue,rust" || echo "Failed to create issue 2"

# 3. Add GitHub Action for Cargo Audit
gh issue create --repo "$REPO" \
  --title "Add GitHub Action for Cargo Audit" \
  --body "We need to ensure our dependencies are secure. Add a step to the CI pipeline to run \`cargo audit\`.

**Acceptance Criteria:**
- Update \`.github/workflows/ci.yml\`.
- Add a job that installs and runs \`cargo-audit\`.
- Fail build on vulnerabilities.

### 📚 Resources
- [Cargo Audit GitHub Action](https://github.com/rustsec/audit-check)
- [\`.github/workflows/ci.yml\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/ci.yml)" \
  --label "stellar-wave,ci,security" || echo "Failed to create issue 3"

# 4. Expose Ledger Sequence in Prometheus Metrics
gh issue create --repo "$REPO" \
  --title "Expose Ledger Sequence in Prometheus Metrics" \
  --body "The operator exposes basic metrics, but we need to track the \`ledger_sequence\` from the node status.

**Acceptance Criteria:**
- Add a \`stellar_node_ledger_sequence\` gauge metric in \`src/controller/metrics.rs\` (needs to be created).
- Update the metric value during the reconciliation loop.
- Ensure it is exported on the metrics port.

### 📚 Resources
- [Prometheus Gauges](https://prometheus.io/docs/concepts/metric_types/#gauge)
- [\`src/controller/metrics.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/metrics.rs)" \
  --label "stellar-wave,observability,feature" || echo "Failed to create issue 4"

# 5. Add retentionPolicy support for specific Storage Classes
gh issue create --repo "$REPO" \
  --title "Add retentionPolicy support for specific Storage Classes" \
  --body "Extend the \`StorageConfig\` struct to allow specifying a custom \`volumeBindingMode\` or other storage-class specific parameters via annotations.

**Acceptance Criteria:**
- Add \`annotations: Option<BTreeMap<String, String>>\` to \`StorageConfig\`.
- Propagate these annotations to the created PVC in \`resources.rs\`.

### 📚 Resources
- [Kubernetes PersistentVolumeClaims](https://kubernetes.io/docs/concepts/storage/persistent-volumes/#persistentvolumeclaims)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)" \
  --label "stellar-wave,kubernetes,feature" || echo "Failed to create issue 5"

# 6. Implement Suspended State correctly for Validators
gh issue create --repo "$REPO" \
  --title "Implement Suspended State correctly for Validators" \
  --body "Currently, setting \`suspended: true\` scales replicas to 0. For Validators (StatefulSets), this works, but we should also ensure the Service is untouched so peer discovery (if external) remains valid, or decide if it should be removed.

**Acceptance Criteria:**
- discuss desired behavior for suspended validators.
- Implement logic to perhaps label the node as 'offline' in Stellar terms if possible, or ensure the StatefulSet scales to 0 cleanly without error logs.

### 📚 Resources
- [Kubernetes StatefulSet Scaling](https://kubernetes.io/docs/concepts/workloads/controllers/statefulset/#scaling)
- [\`src/controller/reconciler.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/reconciler.rs)" \
  --label "stellar-wave,bug,logic" || echo "Failed to create issue 6"

# 7. Create a Grafana Dashboard JSON for Stellar Nodes
gh issue create --repo "$REPO" \
  --title "Create a Grafana Dashboard JSON for Stellar Nodes" \
  --body "Create a standard Grafana dashboard visualization for the metrics exported by the operator (and the Stellar nodes themselves if scraped).

**Acceptance Criteria:**
- Create \`monitoring/grafana-dashboard.json\`.
- Panels for: Node availability, CPU/Memory usage, Ledger sequence (if available), Peer count.

### 📚 Resources
- [Grafana Dashboard JSON Model](https://grafana.com/docs/grafana/latest/dashboards/json-model/)
- [\`monitoring/\`](https://github.com/OtowoOrg/Stellar-K8s/tree/main/monitoring)" \
  --label "stellar-wave,observability,documentation" || echo "Failed to create issue 7"

# 8. Implement Soroban Captive Core Configuration Generator
gh issue create --repo "$REPO" \
  --title "Implement Soroban Captive Core Configuration Generator" \
  --body "Soroban RPC needs a Captive Core config. Instead of passing a raw string, we should generate the TOML configuration from structured fields in the CRD (e.g., \`network_passphrase\`, \`history_archive_urls\`).

**Acceptance Criteria:**
- Create a builder struct for Captive Core config.
- Generate the TOML file and inject it into the ConfigMap.
- Update \`StellarNodeSpec\` to optionally take structured config instead of raw string.

### 📚 Resources
- [Soroban RPC Configuration](https://developers.stellar.org/docs/smart-contracts/getting-started/soroban-rpc)
- [Rust TOML Crate](https://docs.rs/toml/latest/toml/)" \
  --label "stellar-wave,soroban,feature" || echo "Failed to create issue 8"

# 9. Add Automated History Archive Health Check with Retry
gh issue create --repo "$REPO" \
  --title "Add Automated History Archive Health Check with Retry" \
  --body "Before starting a validator, the operator should verify that the configured \`history_archive_urls\` are reachable.

**Acceptance Criteria:**
- Implement an async check in the reconciliation loop (only on startup/update).
- If unreachable, emit a Kubernetes Event (Warning) and block start until reachable (or exponential backoff).
- Use \`reqwest\` or \`hyper\` to ping the archive root.

### 📚 Resources
- [Stellar History Archives](https://developers.stellar.org/docs/run-core-node/history-archives)
- [Reqwest Documentation](https://docs.rs/reqwest/latest/reqwest/)" \
  --label "stellar-wave,reliability,rust" || echo "Failed to create issue 9"

# 10. Implement Leader Election for High Availability Operator
gh issue create --repo "$REPO" \
  --title "Implement Leader Election for High Availability Operator" \
  --body "To run multiple replicas of the \`stellar-operator\` itself, we need leader election to prevent split-brain reconciliation.

**Acceptance Criteria:**
- Use \`kube-rs\`'s \`coordination.k8s.io\` leader election pattern.
- Only the active leader should run the reconciliation loop.
- Standby instances should just serve the read-only API (if safe) or wait.

### 📚 Resources
- [kube-rs Leader Election](https://kube.rs/controllers/leader-election/)
- [\`src/main.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/main.rs)" \
  --label "stellar-wave,architecture,kubernetes" || echo "Failed to create issue 10"

echo "Done! Issues created."
