# Automated Upgrade Workflow

Safe, automated upgrades for the Stellar-K8s operator with pre-upgrade validation, backup creation, health gates, and automatic rollback on failure.

---

## Overview

The upgrade orchestrator (`src/controller/upgrade_orchestrator.rs`) implements a five-phase workflow:

```
Phase 1: Pre-flight validation
    ↓ (fail → abort, no changes made)
Phase 2: Capture current state + create backup
    ↓
Phase 3: Apply new operator image
    ↓
Phase 4: Health gate — wait for operator + all nodes to become Ready
    ↓ (timeout → automatic rollback)
Phase 5: Cleanup annotations + emit success event
```

If the health gate fails, the orchestrator automatically rolls back to the previous image and emits a `Warning` Kubernetes Event.

---

## Pre-Upgrade Validation Checks

Before any changes are made, the orchestrator verifies:

| Check | Description | Failure Action |
|-------|-------------|----------------|
| Target image non-empty | `target_image` must be set | Abort |
| Operator Deployment exists | `stellar-operator` Deployment must be present | Abort |
| No upgrade in progress | `stellar.org/upgrade-in-progress` annotation must be absent | Abort |
| All nodes non-degraded | No `StellarNode` in `Failed` or `Error` phase | Abort |

All checks are read-only. If any check fails, the upgrade is aborted with no changes to the cluster.

---

## Backup Creation

Before applying the upgrade, the orchestrator exports the current state to a ConfigMap:

```
stellar-system/upgrade-backup-<YYYYMMDD-HHMMSS>
```

The ConfigMap contains:
- Timestamp of the backup
- Current operator image tag
- Full `spec` of every `StellarNode` resource

To list backups:

```bash
kubectl get configmap -n stellar-system -l stellar.org/backup-type=pre-upgrade
```

To restore from a backup (manual):

```bash
# View the backup
kubectl get configmap -n stellar-system upgrade-backup-20260529-120000 -o jsonpath='{.data.backup\.json}' | jq .

# Re-apply a node spec from the backup
kubectl apply -f - <<EOF
apiVersion: stellar.org/v1alpha1
kind: StellarNode
...
EOF
```

---

## Running an Upgrade

### Via the CLI (Recommended)

```bash
# Dry-run: validate without making changes
stellar-operator upgrade \
  --target-image ghcr.io/otowoorg/stellar-k8s:v0.2.0 \
  --dry-run

# Execute the upgrade
stellar-operator upgrade \
  --target-image ghcr.io/otowoorg/stellar-k8s:v0.2.0

# Skip backup (not recommended for production)
stellar-operator upgrade \
  --target-image ghcr.io/otowoorg/stellar-k8s:v0.2.0 \
  --skip-backup
```

### Via Helm

```bash
# Upgrade the Helm release (the operator handles the rest automatically)
helm upgrade stellar-operator stellar-k8s/stellar-operator \
  --namespace stellar-system \
  --set image.tag=v0.2.0 \
  --wait \
  --timeout 10m
```

### Via kubectl (Manual)

```bash
# 1. Annotate the Deployment to signal upgrade start
kubectl annotate deployment stellar-operator \
  -n stellar-system \
  stellar.org/upgrade-in-progress=true \
  stellar.org/previous-image=$(kubectl get deployment stellar-operator \
    -n stellar-system \
    -o jsonpath='{.spec.template.spec.containers[0].image}')

# 2. Update the image
kubectl set image deployment/stellar-operator \
  stellar-operator=ghcr.io/otowoorg/stellar-k8s:v0.2.0 \
  -n stellar-system

# 3. Wait for rollout
kubectl rollout status deployment/stellar-operator -n stellar-system --timeout=5m

# 4. Verify nodes are healthy
kubectl stellar status -n stellar

# 5. Clear the annotation
kubectl annotate deployment stellar-operator \
  -n stellar-system \
  stellar.org/upgrade-in-progress- \
  stellar.org/previous-image-
```

---

## Health Checks During Upgrade

The orchestrator polls two health gates after applying the new image:

### Gate 1: Operator Ready

Polls the operator `Deployment` until `readyReplicas >= 1`. Default timeout: **300 seconds**.

```bash
# Monitor manually
kubectl rollout status deployment/stellar-operator -n stellar-system -w
```

### Gate 2: StellarNode Health

Polls all `StellarNode` resources until all are in `Running` or `Ready` phase. Default timeout: **600 seconds**.

```bash
# Monitor manually
kubectl get stellarnodes -A -w
kubectl stellar status -n stellar
```

---

## Automatic Rollback

If either health gate times out, the orchestrator:

1. Patches the operator Deployment back to the previous image.
2. Clears the `stellar.org/upgrade-in-progress` annotation.
3. Emits a `Warning` Kubernetes Event with reason `UpgradeRolledBack`.

```bash
# Check for rollback events
kubectl get events -n stellar-system --field-selector reason=UpgradeRolledBack

# Check for rollback failure events (requires manual intervention)
kubectl get events -n stellar-system --field-selector reason=UpgradeRollbackFailed
```

### Rollback Failure

If the rollback itself fails (e.g., the previous image is no longer available), the orchestrator emits a `UpgradeRollbackFailed` event. In this case:

1. Check the operator logs:
   ```bash
   kubectl logs -n stellar-system deploy/stellar-operator | grep "rollback"
   ```
2. Manually restore the previous image:
   ```bash
   kubectl set image deployment/stellar-operator \
     stellar-operator=<previous-image> \
     -n stellar-system
   ```
3. If the previous image is unavailable, restore from the pre-upgrade backup ConfigMap.

---

## Upgrade Status Reporting

The orchestrator emits Kubernetes Events for all upgrade outcomes:

| Event Reason | Type | Description |
|---|---|---|
| `UpgradeSucceeded` | Normal | Upgrade completed; all nodes healthy |
| `UpgradeRolledBack` | Warning | Health gate failed; rolled back to previous image |
| `UpgradeRollbackFailed` | Warning | Rollback failed; manual intervention required |

```bash
# View all upgrade events
kubectl get events -n stellar-system \
  --field-selector involvedObject.name=stellar-operator \
  --sort-by='.lastTimestamp'
```

---

## CI/CD Integration

The upgrade workflow is tested in CI via the `upgrade-test` job in `.github/workflows/ci.yml`. It runs:

1. Unit tests for the upgrade orchestrator (`upgrade_orchestrator::tests`)
2. Unit tests for the PVC autoscaler (`pvc_autoscaler::tests`)

For end-to-end upgrade testing in a Kind cluster, use the upgrade load test:

```bash
k6 run benchmarks/k6/upgrade-load-test.js \
  -e OPERATOR_URL=https://stellar-operator.example.com
```

---

## Configuration Reference

| Parameter | Default | Description |
|-----------|---------|-------------|
| `target_image` | (required) | Full image reference to upgrade to |
| `operator_namespace` | `stellar-system` | Namespace of the operator Deployment |
| `operator_ready_timeout_secs` | `300` | Seconds to wait for operator to become Ready |
| `node_ready_timeout_secs` | `600` | Seconds to wait for all StellarNodes to become Ready |
| `skip_backup` | `false` | Skip pre-upgrade backup (not recommended) |
| `dry_run` | `false` | Validate only; make no changes |

---

## Troubleshooting

### Upgrade Stuck in Progress

If the `stellar.org/upgrade-in-progress` annotation is present but no upgrade is running:

```bash
# Remove the stale annotation
kubectl annotate deployment stellar-operator \
  -n stellar-system \
  stellar.org/upgrade-in-progress- \
  stellar.org/previous-image-
```

### Nodes Not Becoming Ready After Upgrade

1. Check node status:
   ```bash
   kubectl get stellarnodes -A
   kubectl describe stellarnode <name> -n stellar
   ```
2. Check operator logs for reconciliation errors:
   ```bash
   kubectl logs -n stellar-system deploy/stellar-operator | grep -E "ERROR|WARN"
   ```
3. If a node is stuck in `Pending`, check for resource constraints:
   ```bash
   kubectl describe pod -n stellar -l app=stellar-validator
   ```

### Pre-flight Fails: Node in Error State

Resolve the failing node before upgrading:

```bash
# Check which node is failing
kubectl get stellarnodes -A

# Describe the failing node
kubectl describe stellarnode <name> -n stellar

# Check the node's pod logs
kubectl stellar logs <name> -n stellar
```

---

## See Also

- [Hitless Upgrade](hitless-upgrade.md)
- [Canary Deployments](canary-deployments.md)
- [Disaster Recovery](dr-failover.md)
- [Health Checks](health-checks.md)
