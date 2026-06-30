# Orphaned Resource Auditor

## Overview

After uninstalling the Stellar operator (e.g. via `helm uninstall`), Kubernetes resources that were previously managed by the operator may be left behind. This happens when:

- Finalizers were not processed before the operator was removed.
- The operator was deleted forcibly (`kubectl delete --force`).
- The `retentionPolicy` was set to `Retain` for PersistentVolumeClaims.
- A partial/failed uninstall left some resources in place.

The **Orphan Auditor** (`OrphanAuditor`) scans namespaces for ConfigMaps, Services, and PersistentVolumeClaims that carry the `app.kubernetes.io/managed-by=stellar-operator` label but whose owning `StellarNode` resource no longer exists. It produces a structured report that can be rendered as a table or JSON for scripting.

---

## Usage Examples

### Via kubectl-stellar plugin

```bash
# Audit all orphaned resources in the 'stellar' namespace
kubectl stellar audit orphans --namespace stellar

# Audit all namespaces
kubectl stellar audit orphans --all-namespaces

# Output as JSON (for scripting / CI)
kubectl stellar audit orphans --namespace stellar --format json

# Output as a human-readable table (default)
kubectl stellar audit orphans --namespace stellar --format table
```

### Via the operator CLI

```bash
# Audit a specific namespace
stellar-operator audit-orphans --namespace stellar

# Audit every namespace in the cluster
stellar-operator audit-orphans --all-namespaces

# Write JSON output to a file
stellar-operator audit-orphans --namespace stellar --format json > orphan-report.json
```

### In Rust code (library usage)

```rust
use stellar_k8s::controller::{OrphanAuditor, format_report_table, format_report_json};

let client = kube::Client::try_default().await?;
let auditor = OrphanAuditor::new(client).with_cluster_name("prod-us-east-1");

// Audit a single namespace
let report = auditor.audit_namespace("stellar").await?;
println!("{}", format_report_table(&report));

// Audit all namespaces
let reports = auditor.audit_all_namespaces().await?;
for report in &reports {
    if report.summary.total_orphaned > 0 {
        println!("{}", format_report_table(report));
    }
}
```

---

## Output Formats

### Table (default)

```
Orphan Audit Report — namespace: stellar | cluster: prod-us-east-1 | at: 2026-06-30T02:00:00Z
Total orphaned: 3  (ConfigMaps: 1, Services: 1, PVCs: 1)

KIND                       NAME                                     NAMESPACE              AGE(s)  REASON
----------------------------------------------------------------------------------------------------------------------------------
ConfigMap                  my-validator-config                      stellar                  3600  owning StellarNode 'my-validator' no longer exists
Service                    my-validator-svc                         stellar                  3600  owning StellarNode 'my-validator' no longer exists
PersistentVolumeClaim      data-my-validator-0                      stellar                  3600  owning StellarNode 'my-validator' no longer exists
```

Column descriptions:

| Column    | Description                                              |
|-----------|----------------------------------------------------------|
| KIND      | Kubernetes resource kind                                 |
| NAME      | Resource name                                            |
| NAMESPACE | Namespace the resource resides in                        |
| AGE(s)    | Seconds since the resource was created                   |
| REASON    | Why the resource is considered orphaned                  |

### JSON

```json
{
  "timestamp": "2026-06-30T02:00:00Z",
  "cluster_name": "prod-us-east-1",
  "namespace": "stellar",
  "orphaned_resources": [
    {
      "kind": "ConfigMap",
      "name": "my-validator-config",
      "namespace": "stellar",
      "labels": {
        "app.kubernetes.io/managed-by": "stellar-operator",
        "app.kubernetes.io/instance": "my-validator"
      },
      "age_seconds": 3600,
      "reason": "owning StellarNode 'my-validator' no longer exists"
    }
  ],
  "summary": {
    "total_orphaned": 1,
    "orphaned_config_maps": 1,
    "orphaned_services": 0,
    "orphaned_pvcs": 0
  }
}
```

---

## How to Interpret Results

### `reason` field values

| Reason | Meaning |
|--------|---------|
| `owning StellarNode '<name>' no longer exists` | The StellarNode that owned this resource has been deleted. The resource was not cleaned up. |
| `no owning StellarNode could be identified` | The resource carries the `managed-by=stellar-operator` label but has no `ownerReference` pointing to a `StellarNode`, and no `app.kubernetes.io/instance` label. This may indicate a manually created resource or a labelling inconsistency. |

### When to act

- **ConfigMaps**: Safe to delete after verifying you no longer need the configuration they hold (e.g. `stellar-core.cfg` overrides).
- **Services**: Safe to delete. DNS entries pointing at these services will stop resolving after deletion.
- **PersistentVolumeClaims**: Exercise caution. PVCs may retain valuable ledger data. Confirm the data is no longer needed, or back up the underlying PersistentVolume before deleting.

---

## Verification Steps

After running the audit and cleaning up orphaned resources, verify the cleanup succeeded:

```bash
# 1. Re-run the audit — it should report zero orphans
kubectl stellar audit orphans --namespace stellar

# 2. Confirm no managed-by label remains on ConfigMaps
kubectl get configmaps -n stellar -l app.kubernetes.io/managed-by=stellar-operator

# 3. Confirm no managed-by label remains on Services
kubectl get services -n stellar -l app.kubernetes.io/managed-by=stellar-operator

# 4. Confirm no managed-by label remains on PVCs
kubectl get pvc -n stellar -l app.kubernetes.io/managed-by=stellar-operator
```

All three commands should return `No resources found` after a successful cleanup.

---

## Detection Logic

The auditor uses the following logic to classify a resource as orphaned:

1. List all resources in the target namespace with label `app.kubernetes.io/managed-by=stellar-operator`.
2. For each resource, determine the owning `StellarNode` by:
   - Checking `metadata.ownerReferences` for an entry with `kind: StellarNode`.
   - If no ownerReference is found, fall back to the `app.kubernetes.io/instance` label or the `stellar.org/node-name` label.
3. If the identified owner name is **not** found among currently-existing `StellarNode` resources in the same namespace — or if no owner can be identified — the resource is marked orphaned.

> **Note:** The auditor is read-only. It never deletes or modifies resources. All cleanup must be performed manually or via a dedicated cleanup command.

---

## Resource Kinds Audited

| Kind | API | Label filter |
|------|-----|-------------|
| ConfigMap | `core/v1` | `app.kubernetes.io/managed-by=stellar-operator` |
| Service | `core/v1` | `app.kubernetes.io/managed-by=stellar-operator` |
| PersistentVolumeClaim | `core/v1` | `app.kubernetes.io/managed-by=stellar-operator` |

Additional resource kinds (Deployments, StatefulSets, Secrets) may be added in future releases.

---

## Related Documentation

- [Finalizers](../src/controller/finalizers.rs) — How the operator normally cleans up resources on deletion.
- [diff-utility.md](diff-utility.md) — Debug live state vs. desired state divergence.
- [archive-pruning.md](archive-pruning.md) — Manage history archive storage costs.
- [pod-disruption-budget.md](pod-disruption-budget.md) — PDB configuration for maintenance safety.
