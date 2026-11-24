# Service Ownership Registry

Epic: #1522 · CRD: `ServiceOwnershipRegistry` (`stellar.org/v1alpha1`, cluster-scoped) · Controller: `src/controller/ownership_registry.rs`

The registry maps every running Deployment, StatefulSet and DaemonSet to its
owning team and on-call rotation. The status is derived again from live
workload metadata on every reconcile; it is never taken from a one-time
import.

```yaml
apiVersion: stellar.org/v1alpha1
kind: ServiceOwnershipRegistry
metadata:
  name: cluster
spec:
  excludedNamespaces: [kube-system]
  alertmanagerUrl: http://alertmanager.monitoring:9093
  codeowners: |
    *                 @stellar-k8s-maintainers
    /charts/          @devops-team
  teams:
    - team: platform
      handles: ["@stellar-k8s-maintainers"]
      rotation: platform-primary
      receiver: platform-pager
    - team: devops
      handles: ["@devops-team"]
      rotation: devops-primary
      receiver: devops-pager
      live: true
```

## Resolution order

The first source that names an owner wins:

1. `stellar.org/owner` label
2. `stellar.org/deployed-by-team` annotation, set by the deploy pipeline
3. CODEOWNERS, matched against the `stellar.org/source-path` annotation. As on
   GitHub, the last matching rule wins, and handles map to teams through
   `teams[].handles`.

## Status

- `entries`: resolved owners (team, rotation, receiver, source)
- `unowned`: workloads with no owner
- `stale`: owners that are not a registered team, or whose rotation is not
  `live`
- `history`: ownership changes, bounded by `historyLimit`. Query one workload
  with `history_for`.
- `Ready` condition: `False` with reason `OwnershipGaps` while any workload is
  unowned or stale

In the same reconcile cycle, unowned and stale workloads are posted to
Alertmanager as `WorkloadUnowned` and `WorkloadOwnershipStale` alerts.

## Alert routing

- `attribute_alert` resolves an alert's owner from its `namespace` label and
  its `deployment`, `statefulset` or `daemonset` label.
- `alertmanager_routes` generates one child route per team that matches
  `team="<team>"`. Routing therefore follows the registry's attribution.
