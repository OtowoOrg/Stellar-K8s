#!/bin/bash
# Stellar-K8s Wave Issue Creation Script - BATCH 5
# 3 High (200 pts), 4 Medium (150 pts), 3 Trivial (100 pts)

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
create_label "rust" "DEA584" "Rust related"
create_label "ci" "0075ca" "CI/CD"
create_label "observability" "C2E0C6" "Metrics and logs"
create_label "feature" "a2eeef" "New feature"
create_label "kubernetes" "326ce5" "Kubernetes related"
create_label "logic" "5319e7" "Business logic"
create_label "automation" "ffb3b3" "Automated workflows"
create_label "reliability" "d93f0b" "Reliability and stability"
create_label "architecture" "0e8a16" "Architecture design"
create_label "documentation" "0075ca" "Improvements or additions to documentation"

echo "Creating Batch 5 (Advanced) issues..."

# --- HIGH (200 pts) ---

# 43. Automated VSL (Validator Selection List) updates (High - 200 pts)
create_issue \
  "Implement Automated VSL (Validator Selection List) management" \
  "stellar-wave,architecture,automation" \
  "### 🔴 Difficulty: High (200 Points)

Validators need periodic updates to their quorum sets (VSL). This task involves a controller that fetches updated VSLs from a trusted source and updates the node configuration automatically.

### ✅ Acceptance Criteria
- Add vlSource: String to StellarNodeSpec.
- Implement a controller that periodically polls the VL source.
- Safely update the ConfigMap and trigger a config-reload in Stellar Core without downtime.

### 📚 Resources
- [Stellar Quorum and VSL](https://developers.stellar.org/docs/fundamentals/stellar-consensus-protocol#validators)
- [Stellar Core Configuration Reload](https://github.com/stellar/stellar-core/blob/master/docs/software/admin.md#commands)
"

# 44. Decentralized History Archive Backups (High - 200 pts)
create_issue \
  "Automated History Archive backups to Decentralized Storage" \
  "stellar-wave,reliability,architecture" \
  "### 🔴 Difficulty: High (200 Points)

Integrate with decentralized storage providers (Arweave, IPFS, or Filecoin) to create tamper-proof, permanent backups of the Stellar node's History Archives.

### ✅ Acceptance Criteria
- Add decentralizedBackup: Option<ProviderConfig> to the spec.
- Implement an async task/job that uploads new archive segments to the provider.
- Ensure efficient data deduplication or delta-uploads.

### 📚 Resources
- [Stellar History Archives](https://developers.stellar.org/docs/run-core-node/publishing-history-archives)
"

# 45. RPC Auto-Scaler based on Network Load (High - 200 pts)
create_issue \
  "Implement RPC Auto-Scaler based on Stellar Network Load" \
  "stellar-wave,observability,logic" \
  "### 🔴 Difficulty: High (200 Points)

Standard HPA uses CPU/Memory. This task implements a horizontal auto-scaler that scales Horizon/Soroban-RPC replicas based on actual Stellar network traffic or ledger ingestion lag.

### ✅ Acceptance Criteria
- Implement a custom metric provider for Kubernetes.
- Scale pods based on ledger ingestion lag or request throughput.
- Prevent flapping during high network volatility.

### 📚 Resources
- [Stellar Network Dashboard](https://dashboard.stellar.org/)
- [Kubernetes Custom Metrics API](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/#scaling-on-custom-metrics)
"

# --- MEDIUM (150 pts) ---

# 46. Full vs Recent History Presets (Medium - 150 pts)
create_issue \
  "Support 'Full' vs 'Recent' History node presets" \
  "stellar-wave,kubernetes,feature" \
  "### 🟡 Difficulty: Medium (150 Points)

Users should be able to toggle between a Full History node and a Recent History node via a simple enum in the spec.

### ✅ Acceptance Criteria
- Add historyMode: HistoryMode enum to StellarNodeSpec.
- Adjust Postgres storage and configuration flags automatically based on the mode.
- Update PVC sizing recommendations or labels accordingly.

### 📚 Resources
- [Stellar Node Types](https://developers.stellar.org/learn/stellar-core/types-of-nodes)
"

# 47. kubectl-stellar Plugin (Medium - 150 pts)
create_issue \
  "Develop 'kubectl-stellar' CLI plugin" \
  "stellar-wave,rust,feature" \
  "### 🟡 Difficulty: Medium (150 Points)

Create a dedicated Rust-based kubectl plugin (kubectl-stellar) to allow users to interact with nodes using specialized commands like kubectl stellar logs or kubectl stellar sync-status.

### ✅ Acceptance Criteria
- Repository for kubectl-stellar (or integrated into existing bin).
- Support for list, logs, and status subcommands.
- Package as a Krew-compatible plugin.

### 📚 Resources
- [Kubectl Plugins Guide](https://kubernetes.io/docs/tasks/extend-kubectl/kubectl-plugins/)
"

# 48. PodDisruptionBudgets (PDB) Generation (Medium - 150 pts)
create_issue \
  "Automate PodDisruptionBudgets (PDB) generation" \
  "stellar-wave,reliability,kubernetes" \
  "### 🟡 Difficulty: Medium (150 Points)

To prevent unintentional downtime during node maintenance or cluster autoscaling, the operator should automatically manage PDBs for multi-replica nodes.

### ✅ Acceptance Criteria
- Generate a PodDisruptionBudget for any Node with replicas > 1.
- Allow configuring maxUnavailable or minAvailable in the spec.

### 📚 Resources
- [Kubernetes Pod Disruption Budgets](https://kubernetes.io/docs/tasks/run-application/configure-pdb/)
"

# 49. E2E Integration Test Suite (Medium - 150 pts)
create_issue \
  "Implement E2E Integration Test Suite with KinD" \
  "stellar-wave,testing,ci" \
  "### 🟡 Difficulty: Medium (150 Points)

We need automated end-to-end tests that spin up a local Kubernetes cluster (KinD), install the operator, and deploy a Testnet node to verify real-world behavior.

### ✅ Acceptance Criteria
- CI pipeline integration with Kubernetes-in-Docker (KinD).
- Tests covering: Install -> CRUD operations -> Upgrade -> Deletion.
- Use cargo-nextest for fast execution.

### 📚 Resources
- [KinD (Kubernetes in Docker)](https://kind.sigs.k8s.io/)
"

# --- TRIVIAL (100 pts) ---

# 50. Resource Billing Tags (Trivial - 100 pts)
create_issue \
  "Implement Resource Billing/Cost Allocation tags" \
  "stellar-wave,good-first-issue,kubernetes" \
  "### 🟢 Difficulty: Trivial (100 Points)

Add a way for users to specify arbitrary labels/annotations that should be applied to all underlying resources for billing or organizational tracking.

### ✅ Acceptance Criteria
- Add resourceMeta: Option<Metadata> to the spec.
- Propagate these to PVCs, Deployments, Services, and ConfigMaps.

### 📚 Resources
- [Kubernetes Labels and Selectors](https://kubernetes.io/docs/concepts/overview/working-with-objects/labels/)
"

# 51. Reconciler 'Dry Run' mode (Trivial - 100 pts)
create_issue \
  "Implement Reconciler 'Dry Run' mode" \
  "stellar-wave,good-first-issue,logic" \
  "### 🟢 Difficulty: Trivial (100 Points)

Allow the reconciler to run in a Dry Run mode where it only calculates what changes would be made and emits them as events, without actually patching resources.

### ✅ Acceptance Criteria
- Support a --dry-run flag.
- Emit WouldPatch events in the reconciliation loop.

### 📚 Resources
- [Kubernetes Dry Run](https://kubernetes.io/docs/reference/using-api/api-concepts/#dry-run)
"

# 52. Comprehensive rustdoc Coverage (Trivial - 100 pts)
create_issue \
  "Exhaustive rustdoc coverage for public modules" \
  "stellar-wave,good-first-issue,documentation" \
  "### 🟢 Difficulty: Trivial (100 Points)

Improve the internal documentation of the project. Every public and internal module should have clear, useful rustdoc comments and examples.

### ✅ Acceptance Criteria
- Ensure cargo doc produces no warnings.
- Document all core structs and traits in src/controller and src/crd.

### 📚 Resources
- [The Rustdoc Book](https://doc.rust-lang.org/rustdoc/index.html)
"

echo "Done! Batch 5 issues created."
