#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/repo.sh"
source "$(dirname "$0")/lib/common.sh"

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 14, 5 x 150 pts).

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

EXPECTED_ISSUE_COUNT=5
ACTUAL_ISSUE_COUNT=$(grep -c '^create_issue_with_retry' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT. Update EXPECTED_ISSUE_COUNT or fix the script." >&2
  exit 1
fi

echo "Creating Batch 14 (5 x 150 pts) issues with auto-retry..."

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

print_skip_summary
