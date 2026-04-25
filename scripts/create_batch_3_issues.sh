#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=lib/repo.sh
source "$(dirname "$0")/lib/repo.sh"

# Stellar-K8s Wave Issue Creation Script - BATCH 3 (High Complexity)
# Issues #22 - #24

show_help() {
  cat <<EOF
Usage: $(basename "$0") [-h|--help]

Creates GitHub issues for Stellar-K8s Wave (Batch 3, issues #22-#24).

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

EXPECTED_ISSUE_COUNT=3
ACTUAL_ISSUE_COUNT=$(grep -c '^gh issue create' "$0")
if [ "$ACTUAL_ISSUE_COUNT" -ne "$EXPECTED_ISSUE_COUNT" ]; then
  echo "ERROR: Expected $EXPECTED_ISSUE_COUNT issue create calls, found $ACTUAL_ISSUE_COUNT. Update EXPECTED_ISSUE_COUNT or fix the script." >&2
  exit 1
fi

echo "Creating Batch 3 (200 points) issues..."

# 22. Automated PVC Snapshots/Backups (High - 200 Points)
gh issue create --repo "$REPO" \
  --title "Implement Automated PVC Snapshots/Backups for StellarNode" \
  --body "### 🔴 Difficulty: High (200 Points)

Stellar nodes store critical data in PVCs. To ensure disaster recovery, the operator should manage automated volume snapshots (using Kubernetes \`VolumeSnapshot\` API) or scheduled backups to S3/GCS.

### ✅ Acceptance Criteria
- Add \`backup: Option<BackupConfig>\` to \`StellarNodeSpec\`.
- Implement a controller/job that triggers a \`VolumeSnapshot\` based on a Cron schedule.
- Handle cleanup of old snapshots based on a retention policy.
- Provide a restoration path (creating a new node from a specified snapshot).

### 📚 Resources
- [Kubernetes Volume Snapshots](https://kubernetes.io/docs/concepts/storage/volume-snapshots/)
- [Stellar Core Data Management](https://developers.stellar.org/docs/run-core-node/prerequisites#storage)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)
" --label "stellar-wave,reliability,architecture"

# 23. ServiceMonitor & HPA for Horizon (High - 200 Points)
gh issue create --repo "$REPO" \
  --title "Implement ServiceMonitor & HPA for Horizon Auto-Scaling" \
  --body "### 🔴 Difficulty: High (200 Points)

Horizon nodes often experience variable traffic. The operator should support Horizontal Pod Autoscaling (HPA) based on Prometheus metrics (e.g., requests-per-second) to scale replicas dynamically.

### ✅ Acceptance Criteria
- Add \`autoscaling: Option<AutoscalingConfig>\` to \`StellarNodeSpec\`.
- Generate a \`ServiceMonitor\` (Prometheus Operator) for easy scraping.
- Generate an \`HorizontalPodAutoscaler\` (v2) resource that targets the Horizon Deployment.
- Ensure the HPA can use custom metrics via the Prometheus Adapter.

### 📚 Resources
- [Horizontal Pod Autoscaling](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/)
- [Prometheus Operator: ServiceMonitor](https://github.com/prometheus-operator/prometheus-operator/blob/main/Documentation/user-guides/getting-started.md)
- [\`src/controller/metrics.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/metrics.rs)
" --label "stellar-wave,observability,kubernetes"

# 24. Ingress & Cert-Manager Integration (High - 200 Points)
gh issue create --repo "$REPO" \
  --title "Implement Ingress & Cert-Manager Integration for Public APIs" \
  --body "### 🔴 Difficulty: High (200 Points)

Exposing Horizon or Soroban RPC to the internet requires secure TLS termination and domain management. The operator should automate Ingress creation and mTLS/TLS setup via Cert-Manager.

### ✅ Acceptance Criteria
- Add \`ingress: Option<IngressConfig>\` to \`StellarNodeSpec\`.
- Generate a Kubernetes \`Ingress\` resource with configurable hosts and paths.
- Add annotations for \`cert-manager.io\` to automatically provision Let's Encrypt certificates.
- Support TLS secret management for secure HTTPS access.

### 📚 Resources
- [Kubernetes Ingress](https://kubernetes.io/docs/concepts/services-networking/ingress/)
- [Cert-Manager Documentation](https://cert-manager.io/docs/usage/ingress/)
- [\`src/controller/resources.rs\`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/src/controller/resources.rs)
" --label "stellar-wave,kubernetes,architecture"

echo "Done! Batch 3 issues created (#22-#24)."
