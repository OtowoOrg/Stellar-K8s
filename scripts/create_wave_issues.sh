#!/bin/bash
# Stellar-K8s Wave Issue Creation Script
# Uses gh CLI to create issues defined in WAVE_ISSUES.md

# Source shared retry/backoff and dry-run helper.
# shellcheck source=scripts/retry_helper.sh
source "$(dirname "$0")/retry_helper.sh"

# Helper to create label if not exists
create_label() {
  gh label create "$1" --color "$2" --description "$3" || true
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
create_issue \
  "Add unit tests for StellarNodeSpec validation" \
  "stellar-wave,good-first-issue,testing" \
  "The StellarNodeSpec::validate() function currently checks for missing configurations. We need comprehensive unit tests to ensure it correctly accepts valid configs and rejects invalid ones (e.g., Validator with >1 replica).

**Acceptance Criteria:**
- Create src/crd/tests.rs (or add to stellar_node.rs)
- Test cases for: valid validator, missing validator config, multi-replica validator (fail), missing horizon config."

# 2. Implement Display trait for StellarNetwork
create_issue \
  "Implement Display trait for StellarNetwork" \
  "stellar-wave,good-first-issue,rust" \
  "Currently, StellarNetwork relies on Debug or serde for string representation. Implementing std::fmt::Display will allow for cleaner logging and status messages.

**Acceptance Criteria:**
- Implement Display for StellarNetwork enum.
- Update logs in reconciler.rs to use the new Display implementation."

# 3. Add GitHub Action for Cargo Audit
create_issue \
  "Add GitHub Action for Cargo Audit" \
  "stellar-wave,ci,security" \
  "We need to ensure our dependencies are secure. Add a step to the CI pipeline to run cargo audit.

**Acceptance Criteria:**
- Update .github/workflows/ci.yml.
- Add a job that installs and runs cargo-audit.
- Fail build on vulnerabilities."

# 4. Expose Ledger Sequence in Prometheus Metrics
create_issue \
  "Expose Ledger Sequence in Prometheus Metrics" \
  "stellar-wave,observability,feature" \
  "The operator exposes basic metrics, but we need to track the ledger_sequence from the node status.

**Acceptance Criteria:**
- Add a stellar_node_ledger_sequence gauge metric in src/controller/metrics.rs (needs to be created).
- Update the metric value during the reconciliation loop.
- Ensure it is exported on the metrics port."

# 5. Add retentionPolicy support for specific Storage Classes
create_issue \
  "Add retentionPolicy support for specific Storage Classes" \
  "stellar-wave,kubernetes,feature" \
  "Extend the StorageConfig struct to allow specifying a custom volumeBindingMode or other storage-class specific parameters via annotations.

**Acceptance Criteria:**
- Add annotations: Option<BTreeMap<String, String>> to StorageConfig.
- Propagate these annotations to the created PVC in resources.rs."

# 6. Implement Suspended State correctly for Validators
create_issue \
  "Implement Suspended State correctly for Validators" \
  "stellar-wave,bug,logic" \
  "Currently, setting suspended: true scales replicas to 0. For Validators (StatefulSets), this works, but we should also ensure the Service is untouched so peer discovery (if external) remains valid, or decide if it should be removed.

**Acceptance Criteria:**
- discuss desired behavior for suspended validators.
- Implement logic to perhaps label the node as offline in Stellar terms if possible, or ensure the StatefulSet scales to 0 cleanly without error logs."

# 7. Create a Grafana Dashboard JSON for Stellar Nodes
create_issue \
  "Create a Grafana Dashboard JSON for Stellar Nodes" \
  "stellar-wave,observability,documentation" \
  "Create a standard Grafana dashboard visualization for the metrics exported by the operator (and the Stellar nodes themselves if scraped).

**Acceptance Criteria:**
- Create monitoring/grafana-dashboard.json.
- Panels for: Node availability, CPU/Memory usage, Ledger sequence (if available), Peer count."

# 8. Implement Soroban Captive Core Configuration Generator
create_issue \
  "Implement Soroban Captive Core Configuration Generator" \
  "stellar-wave,soroban,feature" \
  "Soroban RPC needs a Captive Core config. Instead of passing a raw string, we should generate the TOML configuration from structured fields in the CRD (e.g., network_passphrase, history_archive_urls).

**Acceptance Criteria:**
- Create a builder struct for Captive Core config.
- Generate the TOML file and inject it into the ConfigMap.
- Update StellarNodeSpec to optionally take structured config instead of raw string."

# 9. Add Automated History Archive Health Check with Retry
create_issue \
  "Add Automated History Archive Health Check with Retry" \
  "stellar-wave,reliability,rust" \
  "Before starting a validator, the operator should verify that the configured history_archive_urls are reachable.

**Acceptance Criteria:**
- Implement an async check in the reconciliation loop (only on startup/update).
- If unreachable, emit a Kubernetes Event (Warning) and block start until reachable (or exponential backoff).
- Use reqwest or hyper to ping the archive root."

# 10. Implement Leader Election for High Availability Operator
create_issue \
  "Implement Leader Election for High Availability Operator" \
  "stellar-wave,architecture,kubernetes" \
  "To run multiple replicas of the stellar-operator itself, we need leader election to prevent split-brain reconciliation.

**Acceptance Criteria:**
- Use kube-rs's coordination.k8s.io leader election pattern.
- Only the active leader should run the reconciliation loop.
- Standby instances should just serve the read-only API (if safe) or wait."

echo "Done! Issues created."
