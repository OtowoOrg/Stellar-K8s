#!/usr/bin/env bash
set -euo pipefail

REPO="OtowoOrg/Stellar-K8s"

# Source shared retry/backoff and dry-run helper.
# shellcheck source=scripts/retry_helper.sh
source "$(dirname "$0")/retry_helper.sh"

echo "Creating Batch 14 (5 x 150 pts) issues with auto-retry..."

# Alias kept for readability; delegates to the shared helper.
create_issue_with_retry() { create_issue "$1" "$2" "$3"; }

# ─── ISSUE 1 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Storage Optimization: Automated Local NVMe Provisioning Support" \
  "stellar-wave,enhancement,performance" \
  "### 🟡 Difficulty: Medium (150 Points)

Stellar Core is highly sensitive to disk I/O. Using standard EBS/Persistent Disks can lead to sync lag. We need to support local NVMe drives for nodes that require maximum throughput.

### ✅ Acceptance Criteria
- Implement support for a \`LocalStorage\` mode in the \`StellarNode\` CRD.
- The operator should automatically detect if a node has local-path-provisioner or similar capability.
- Configure \`nodeAffinity\` and \`volumeMounts\` to correctly target local discs.
- Provide a benchmark comparison in the docs between standard PVCs and Local NVMe.

### 📚 Resources
- [Kubernetes Local Volumes](https://kubernetes.io/docs/concepts/storage/volumes/#local)
"

# ─── ISSUE 2 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Automated Ledger Snapshot Backups to S3-Compatible Storage" \
  "stellar-wave,enhancement,reliability" \
  "### 🟡 Difficulty: Medium (150 Points)

Relying solely on PVC snapshots isn't enough for true Disaster Recovery. We need a way to push compressed ledger snapshots to offsite S3 buckets.

### ✅ Acceptance Criteria
- Implement a \`BackupSchedule\` in the \`StellarNode\` spec.
- The operator should spin up a CronJob that:
  - Takes a local snapshot of the ledger directory.
  - Compresses it.
  - Uploads it to an S3-compatible backend (AWS, MinIO, GCS).
- Ensure credentials are handled via a K8s Secret.

### 📚 Resources
- [Stellar Core: History Archives documentation](https://developers.stellar.org/docs/run-core-node/configuring/history-archives)
"

# ─── ISSUE 3 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Helm: Integration with External Secrets Operator (ESO)" \
  "stellar-wave,enhancement,security" \
  "### 🟡 Difficulty: Medium (150 Points)

Many production clusters use the [External Secrets Operator](https://external-secrets.io/) to pull secrets from AWS Secrets Manager or Vault. Our Helm chart should support this pattern.

### ✅ Acceptance Criteria
- Add support to the Helm chart for optional \`ExternalSecret\` manifests.
- Map fields like \`validator-seed\` and \`db-password\` to potential external sources.
- Allow switching between standard K8s Secrets and ExternalSecrets via a boolean flag in \`values.yaml\`.

### 📚 Resources
- [External Secrets Operator Docs](https://external-secrets.io/)
"

# ─── ISSUE 4 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "Strict Pod Anti-Affinity and AZ-Aware Scheduling" \
  "stellar-wave,enhancement,reliability" \
  "### 🟡 Difficulty: Medium (150 Points)

To prevent a single node or Availability Zone (AZ) failure from taking down multiple validators, we need to enforce strict anti-affinity rules.

### ✅ Acceptance Criteria
- Update the workload builder to inject \`podAntiAffinity\` by default.
- Ensure that pods with the same \`stellar-network\` label are spread across different nodes and topology domains (AZs).
- Make this configurable (e.g., \`Soft\` vs \`Hard\` anti-affinity).

### 📚 Resources
- [K8s Pod Topology Spread Constraints](https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/)
"

# ─── ISSUE 5 (150 pts) ────────────────────────────────────────────────────────
create_issue_with_retry \
  "kubectl-stellar Plugin: 'explain' command for Stellar error codes" \
  "stellar-wave,enhancement,dx" \
  "### 🟡 Difficulty: Medium (150 Points)

Stellar CLI outputs can be cryptic (e.g., \`tx_bad_auth\`, \`op_no_destination\`). We need a helper command to explain these to K8s operators.

### ✅ Acceptance Criteria
- Add a \`kubectl stellar explain <error-code>\` command.
- The command should fetch (or have a local embed of) common Stellar Core and Horizon error codes/result codes.
- Provide a summary and a link to the official documentation for each code.

### 📚 Resources
- [Stellar Error Codes Reference](https://developers.stellar.org/docs/glossary/errors)
"

echo ""
echo "🎉 Batch 14 (5 x 150 pts) issues created successfully!"
