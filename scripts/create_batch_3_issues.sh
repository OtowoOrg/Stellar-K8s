#!/bin/bash
set -e

# Stellar-K8s Wave Issue Creation Script - BATCH 3 (High Complexity)
# Issues #22 - #24

# Source shared retry/backoff and dry-run helper.
# shellcheck source=scripts/retry_helper.sh
source "$(dirname "$0")/retry_helper.sh"

echo "Creating Batch 3 (200 points) issues..."

# 22. Automated PVC Snapshots/Backups (High - 200 Points)
create_issue \
  "Implement Automated PVC Snapshots/Backups for StellarNode" \
  "stellar-wave,reliability,architecture" \
  "### 🔴 Difficulty: High (200 Points)

Stellar nodes store critical data in PVCs. To ensure disaster recovery, the operator should manage automated volume snapshots (using Kubernetes VolumeSnapshot API) or scheduled backups to S3/GCS.

### ✅ Acceptance Criteria
- Add backup: Option<BackupConfig> to StellarNodeSpec.
- Implement a controller/job that triggers a VolumeSnapshot based on a Cron schedule.
- Handle cleanup of old snapshots based on a retention policy.
- Provide a restoration path (creating a new node from a specified snapshot).

### 📚 Resources
- [Kubernetes Volume Snapshots](https://kubernetes.io/docs/concepts/storage/volume-snapshots/)
- [Stellar Core Data Management](https://developers.stellar.org/docs/run-core-node/prerequisites#storage)
"

# 23. ServiceMonitor & HPA for Horizon (High - 200 Points)
create_issue \
  "Implement ServiceMonitor & HPA for Horizon Auto-Scaling" \
  "stellar-wave,observability,kubernetes" \
  "### 🔴 Difficulty: High (200 Points)

Horizon nodes often experience variable traffic. The operator should support Horizontal Pod Autoscaling (HPA) based on Prometheus metrics (e.g., requests-per-second) to scale replicas dynamically.

### ✅ Acceptance Criteria
- Add autoscaling: Option<AutoscalingConfig> to StellarNodeSpec.
- Generate a ServiceMonitor (Prometheus Operator) for easy scraping.
- Generate an HorizontalPodAutoscaler (v2) resource that targets the Horizon Deployment.
- Ensure the HPA can use custom metrics via the Prometheus Adapter.

### 📚 Resources
- [Horizontal Pod Autoscaling](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/)
- [Prometheus Operator: ServiceMonitor](https://github.com/prometheus-operator/prometheus-operator/blob/main/Documentation/user-guides/getting-started.md)
"

# 24. Ingress & Cert-Manager Integration (High - 200 Points)
create_issue \
  "Implement Ingress & Cert-Manager Integration for Public APIs" \
  "stellar-wave,kubernetes,architecture" \
  "### 🔴 Difficulty: High (200 Points)

Exposing Horizon or Soroban RPC to the internet requires secure TLS termination and domain management. The operator should automate Ingress creation and mTLS/TLS setup via Cert-Manager.

### ✅ Acceptance Criteria
- Add ingress: Option<IngressConfig> to StellarNodeSpec.
- Generate a Kubernetes Ingress resource with configurable hosts and paths.
- Add annotations for cert-manager.io to automatically provision Let's Encrypt certificates.
- Support TLS secret management for secure HTTPS access.

### 📚 Resources
- [Kubernetes Ingress](https://kubernetes.io/docs/concepts/services-networking/ingress/)
- [Cert-Manager Documentation](https://cert-manager.io/docs/usage/ingress/)
"

echo "Done! Batch 3 issues created (#22-#24)."
