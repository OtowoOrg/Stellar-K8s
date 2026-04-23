#!/bin/bash
set -e

# Stellar-K8s Wave Issue Creation Script - BATCH 2
# Issues #12 - #21

# Source shared retry/backoff and dry-run helper.
# shellcheck source=scripts/retry_helper.sh
source "$(dirname "$0")/retry_helper.sh"

echo "Creating Batch 2 of Stellar Wave issues..."

# 12. Add Resource Limit validation (Trivial - 100 Points)
create_issue \
  "Add Resource Limit validation (CPU/Memory)" \
  "stellar-wave,good-first-issue,kubernetes" \
  "### 🟢 Difficulty: Trivial (100 Points)

Currently, the operator allows setting CPU/Memory requests and limits without validating them. We need to ensure that requests <= limits to prevent Kubernetes scheduling errors.

### ✅ Acceptance Criteria
- Update src/crd/stellar_node.rs validation logic.
- Reject specs where requested resources exceed limits.
- Add unit tests for this validation.

### 📚 Resources
- [Kubernetes Resource Management](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/)
"

# 13. Implement validate() for NodePort range (Trivial - 100 Points)
create_issue \
  "Implement validation for custom NodePort range" \
  "stellar-wave,good-first-issue,kubernetes" \
  "### 🟢 Difficulty: Trivial (100 Points)

When a user specifies a NodePort in the service config, we should validate that it falls within the standard Kubernetes range (30000-32767) unless otherwise configured, to provide early feedback.

### ✅ Acceptance Criteria
- Add validation in StellarNodeSpec::validate() for NodePort fields.
- Throw a meaningful error if the port is out of range.

### 📚 Resources
- [Kubernetes Service NodePort](https://kubernetes.io/docs/concepts/services-networking/service/#type-nodeport)
"

# 14. Add topologySpreadConstraints support (Trivial - 100 Points)
create_issue \
  "Add topologySpreadConstraints support to Pod template" \
  "stellar-wave,kubernetes,feature" \
  "### 🟢 Difficulty: Trivial (100 Points)

To ensure high availability, users should be able to specify topologySpreadConstraints to spread Stellar pods across different Availability Zones (AZs) or nodes.

### ✅ Acceptance Criteria
- Add topologySpreadConstraints field to the Pod template in StellarNodeSpec.
- Propagate this field to the generated Deployment/StatefulSet in resources.rs.

### 📚 Resources
- [Kubernetes Pod Topology Spread Constraints](https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/)
"

# 15. Implement standard Kubernetes Conditions in Status (Medium - 150 Points)
create_issue \
  "Implement standard Kubernetes Conditions in Status" \
  "stellar-wave,architecture,logic" \
  "### 🟡 Difficulty: Medium (150 Points)

Instead of a single Phase string, the operator should use the standard Kubernetes Conditions pattern (e.g., Ready, Progressing, Degraded) to provide more granular status information.

### ✅ Acceptance Criteria
- Update StellarNodeStatus to include a conditions vector.
- Implement a helper to update conditions (TransitionTime, Status, Reason, Message).
- Update the reconciler to report 'Ready' condition when all sub-resources are healthy.

### 📚 Resources
- [Kubernetes API Conventions: Conditions](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md#typical-status-properties)
- [kube-rs Conditions Guide](https://kube.rs/controllers/conditions/)
"

# 16. Add support for Sidecar containers (Medium - 150 Points)
create_issue \
  "Add support for Sidecar containers in StellarNode" \
  "stellar-wave,kubernetes,feature" \
  "### 🟡 Difficulty: Medium (150 Points)

Users may need to run sidecar containers (like log forwarders, monitoring agents, or proxies) alongside the main Stellar container.

### ✅ Acceptance Criteria
- Add sidecars: Option<Vec<Container>> to StellarNodeSpec.
- Merge these containers into the generated Pod spec in resources.rs.
- Ensure volumes can be shared between the main container and sidecars.

### 📚 Resources
- [Kubernetes Sidecar Containers](https://kubernetes.io/docs/concepts/workloads/pods/sidecar-containers/)
"

# 17. Implement 'Maintenance Mode' flag (Medium - 150 Points)
create_issue \
  "Implement 'Maintenance Mode' flag" \
  "stellar-wave,logic,feature" \
  "### 🟡 Difficulty: Medium (150 Points)

When performing manual operations on a node, it is useful to have a Maintenance Mode that keeps the Service and PVC but scales the workload temporarily or labels it to prevent the operator from fighting manual changes.

### ✅ Acceptance Criteria
- Add maintenanceMode: bool to StellarNodeSpec.
- When active, the reconciler should skip Apply steps for the workload but keep status reporting active.

### 📚 Resources
- [Kubernetes Operator Lifecycle](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/)
"

# 18. Add Prometheus Rule generation (Medium - 150 Points)
create_issue \
  "Add Prometheus Rule generation for Alerting" \
  "stellar-wave,observability,feature" \
  "### 🟡 Difficulty: Medium (150 Points)

The operator should optionally generate a PrometheusRule custom resource (if Prometheus Operator is present) to alert on node crashes or sync issues.

### ✅ Acceptance Criteria
- Add alerting: bool to StellarNodeSpec.
- If enabled, create a ConfigMap or PrometheusRule containing standard alerts (NodeDown, HighMemory, etc.).

### 📚 Resources
- [Prometheus Operator: Monitoring Mixins](https://github.com/prometheus-operator/kube-prometheus/tree/main/jsonnet/kube-prometheus/rules)
"

# 19. Implement 'Auto-Sync Health' check for Horizon (High - 200 Points)
create_issue \
  "Implement 'Auto-Sync Health' check for Horizon" \
  "stellar-wave,reliability,rust" \
  "### 🔴 Difficulty: High (200 Points)

Horizon nodes can take time to ingest and catch up. The operator should query the Horizon /health or /metrics endpoint to verify it is fully caught up before marking the node as Ready.

### ✅ Acceptance Criteria
- Add an HTTP client to the reconciler (e.g., reqwest).
- Query the pod's local IP on the health port.
- Block the transition to Ready status until the node reports it is synced.

### 📚 Resources
- [Horizon API Reference](https://developers.stellar.org/docs/data-availability/horizon/api-reference)
"

# 20. Support for External Postgres Databases (High - 200 Points)
create_issue \
  "Support for External Postgres Databases" \
  "stellar-wave,architecture,feature" \
  "### 🔴 Difficulty: High (200 Points)

For production, users often prefer managed databases (RDS, Cloud SQL, CockroachDB). The operator should allow passing external DB connection strings via Secrets.

### ✅ Acceptance Criteria
- Add database: ExternalDatabaseConfig to StellarNodeSpec.
- Support fetching credentials from an existing Secret (secretKeyRef).
- Inject these as environment variables into the Stellar/Horizon containers.

### 📚 Resources
- [Stellar Core Database Config](https://github.com/stellar/stellar-core/blob/master/docs/software/admin.md#database)
- [Kubernetes Secrets](https://kubernetes.io/docs/concepts/configuration/secret/)
"

# 21. Implement Automated Database Migrations for Horizon (High - 200 Points)
create_issue \
  "Implement Automated Database Migrations for Horizon" \
  "stellar-wave,reliability,automation" \
  "### 🔴 Difficulty: High (200 Points)

When upgrading Horizon, the database schema often needs a migration. The operator should automatically run an InitContainer or Job to perform horizon db init or horizon db upgrade before starting the main process.

### ✅ Acceptance Criteria
- Add logic to detect version changes.
- Launch a one-time Job or InitContainer to run migration commands.
- Block the main container startup until the migration success is confirmed.

### 📚 Resources
- [Horizon DB Management](https://developers.stellar.org/docs/data-availability/horizon/admin#database-management)
- [Kubernetes Init Containers](https://kubernetes.io/docs/concepts/workloads/pods/init-containers/)
"

echo "Done! Batch 2 issues created (12-21)."
