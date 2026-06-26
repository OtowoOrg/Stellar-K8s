# Stellar Operator Upgrade Guide

This guide provides instructions for upgrading the stellar-operator Helm chart between versions.

## Table of Contents

- [Pre-Upgrade Checklist](#pre-upgrade-checklist)
- [Upgrade Process](#upgrade-process)
- [Version-Specific Notes](#version-specific-notes)
- [Rollback Procedure](#rollback-procedure)
- [Troubleshooting](#troubleshooting)

## Pre-Upgrade Checklist

Before upgrading, ensure you have completed the following:

### 1. Backup Current State

```bash
# Backup Helm release values
helm get values stellar-operator -n stellar-system > backup-values.yaml

# Backup CRD definitions
kubectl get crd stellarnodes.stellar.org -o yaml > backup-stellarnode-crd.yaml
kubectl get crd stellarbenchmarks.stellar.org -o yaml > backup-benchmark-crd.yaml

# Backup all StellarNode resources
kubectl get stellarnodes --all-namespaces -o yaml > backup-stellarnodes.yaml

# Backup operator configuration
kubectl get configmap -n stellar-system -l app.kubernetes.io/name=stellar-operator -o yaml > backup-configmaps.yaml
```

### 2. Review Release Notes

Check the CHANGELOG.md for breaking changes and new features in the target version.

### 3. Check Active Nodes

```bash
# List all active StellarNodes
kubectl get stellarnodes --all-namespaces

# Check node health status
kubectl get stellarnodes --all-namespaces -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.status.phase}{"\n"}{end}'
```

### 4. Verify Cluster Health

```bash
# Check operator deployment status
kubectl get deployment stellar-operator -n stellar-system

# Check for pending PVCs
kubectl get pvc --all-namespaces --field-selector=status.phase=Pending

# Verify no nodes are in Failed state
kubectl get stellarnodes --all-namespaces -o json | jq -r '.items[] | select(.status.phase == "Failed") | .metadata.name'
```

## Upgrade Process

### Standard Upgrade (Recommended)

The Helm chart includes pre-upgrade hooks that automatically perform safety checks:

```bash
# Update Helm repository
helm repo update

# Upgrade with automatic pre-checks (hooks enabled by default)
helm upgrade stellar-operator stellar/stellar-operator \
  -n stellar-system \
  -f your-values.yaml \
  --wait \
  --timeout 10m
```

The pre-upgrade hook will:
- Backup current CRD definitions
- Check for active StellarNodes
- Validate operator deployment status
- Check for breaking CRD schema changes
- Verify node health status
- Check for pending PVCs
- Create an upgrade checkpoint

### Manual Upgrade (Advanced)

If you prefer to run checks manually or disable hooks:

```bash
# Disable hooks and upgrade
helm upgrade stellar-operator stellar/stellar-operator \
  -n stellar-system \
  -f your-values.yaml \
  --set hooks.preUpgrade.enabled=false \
  --wait \
  --timeout 10m
```

### Upgrade with Custom Values

```bash
# Upgrade using production values example
helm upgrade stellar-operator stellar/stellar-operator \
  -n stellar-system \
  -f charts/stellar-operator/examples/values-production.yaml \
  --wait
```

### Dry Run (Test Before Applying)

```bash
# Perform a dry-run to see what would change
helm upgrade stellar-operator stellar/stellar-operator \
  -n stellar-system \
  -f your-values.yaml \
  --dry-run \
  --debug
```

## Version-Specific Notes

### Upgrading to v0.2.0

**Breaking Changes:**
- CRD schema updated with new validation rules
- `spec.network` field now required for all StellarNodes
- Deprecated `spec.legacyConfig` field removed

**Migration Steps:**
1. Update all StellarNode manifests to include `spec.network`
2. Remove any usage of `spec.legacyConfig`
3. Run upgrade with hooks enabled

```bash
# Update existing nodes before upgrade
kubectl get stellarnodes --all-namespaces -o json | \
  jq '.items[] | select(.spec.network == null) | .metadata.name' | \
  xargs -I {} kubectl patch stellarnode {} --type=merge -p '{"spec":{"network":"mainnet"}}'
```

### Upgrading to v0.1.5

**New Features:**
- Enhanced Prometheus metrics (Issue #757)
- Comprehensive Grafana dashboards (Issue #754)
- Helm hooks for safe upgrades (Issue #756)

**Migration Steps:**
1. Import new Grafana dashboards from `monitoring/` directory
2. Update Prometheus scrape configs if using custom configuration
3. Enable hooks in values.yaml: `hooks.preUpgrade.enabled: true`

### Upgrading from v0.1.0 to v0.1.x

**Changes:**
- New CRD fields added (backward compatible)
- Additional RBAC permissions required
- New metrics endpoints

**Migration Steps:**
No manual intervention required. Upgrade proceeds automatically.

## Rollback Procedure

If the upgrade fails or causes issues, you can rollback to the previous version:

### Automatic Rollback

```bash
# Rollback to previous release
helm rollback stellar-operator -n stellar-system

# Rollback to specific revision
helm rollback stellar-operator 3 -n stellar-system
```

### Manual Rollback

```bash
# List release history
helm history stellar-operator -n stellar-system

# Identify the revision to rollback to
# Rollback to that revision
helm rollback stellar-operator <revision> -n stellar-system --wait
```

### Restore from Backup

If Helm rollback fails, restore from backups:

```bash
# Restore CRDs
kubectl apply -f backup-stellarnode-crd.yaml

# Restore StellarNode resources
kubectl apply -f backup-stellarnodes.yaml

# Restore ConfigMaps
kubectl apply -f backup-configmaps.yaml

# Reinstall previous chart version
helm install stellar-operator stellar/stellar-operator \
  --version <previous-version> \
  -n stellar-system \
  -f backup-values.yaml
```

## Post-Upgrade Verification

After upgrading, verify the installation:

```bash
# Run Helm tests
helm test stellar-operator -n stellar-system

# Check operator pod status
kubectl get pods -n stellar-system -l app.kubernetes.io/name=stellar-operator

# Verify CRDs are updated
kubectl get crd stellarnodes.stellar.org -o yaml | grep version

# Check operator logs
kubectl logs -n stellar-system -l app.kubernetes.io/name=stellar-operator --tail=100

# Verify StellarNodes are reconciling
kubectl get stellarnodes --all-namespaces -w
```

## Troubleshooting

### Upgrade Hangs or Times Out

**Symptoms:** Helm upgrade command hangs or times out

**Solutions:**
1. Check operator pod logs for errors
2. Verify webhook is responding (if enabled)
3. Increase timeout: `--timeout 15m`
4. Check for resource constraints

```bash
# Check operator pod events
kubectl describe pod -n stellar-system -l app.kubernetes.io/name=stellar-operator

# Check webhook status
kubectl get validatingwebhookconfigurations | grep stellar
```

### CRD Validation Errors

**Symptoms:** Existing StellarNodes fail validation after upgrade

**Solutions:**
1. Review CRD schema changes in release notes
2. Update StellarNode manifests to match new schema
3. Use `kubectl explain` to see new fields

```bash
# Check CRD schema
kubectl explain stellarnode.spec

# Validate a specific node
kubectl get stellarnode <name> -o yaml | kubectl apply --dry-run=server -f -
```

### Operator Crashes After Upgrade

**Symptoms:** Operator pod in CrashLoopBackOff

**Solutions:**
1. Check logs for error messages
2. Verify RBAC permissions are updated
3. Check for incompatible configuration

```bash
# View crash logs
kubectl logs -n stellar-system -l app.kubernetes.io/name=stellar-operator --previous

# Verify RBAC
kubectl auth can-i --list --as=system:serviceaccount:stellar-system:stellar-operator
```

### Metrics Not Appearing

**Symptoms:** New metrics not visible in Prometheus/Grafana

**Solutions:**
1. Verify Prometheus is scraping the operator
2. Check ServiceMonitor configuration
3. Restart Prometheus to reload config

```bash
# Check if metrics endpoint is responding
kubectl port-forward -n stellar-system svc/stellar-operator 9090:9090
curl http://localhost:9090/metrics | grep stellar_

# Verify ServiceMonitor
kubectl get servicemonitor -n stellar-system
```

### Rollback Fails

**Symptoms:** Helm rollback command fails

**Solutions:**
1. Check Helm release history
2. Manually restore from backups
3. Reinstall from scratch if necessary

```bash
# Check release status
helm status stellar-operator -n stellar-system

# Force delete and reinstall
helm uninstall stellar-operator -n stellar-system
helm install stellar-operator stellar/stellar-operator \
  --version <previous-version> \
  -n stellar-system \
  -f backup-values.yaml
```

## Best Practices

1. **Always test upgrades in a non-production environment first**
2. **Enable pre-upgrade hooks for automatic safety checks**
3. **Backup all resources before upgrading**
4. **Review release notes for breaking changes**
5. **Monitor operator logs during and after upgrade**
6. **Verify all StellarNodes are healthy after upgrade**
7. **Keep Helm release history for easy rollback**
8. **Document any custom configuration changes**

## Support

For upgrade issues or questions:
- GitHub Issues: https://github.com/OtowoOrg/Stellar-K8s/issues
- Documentation: https://github.com/OtowoOrg/Stellar-K8s/tree/main/docs
- Slack: #stellar-k8s on Stellar Community Slack

## Related Documentation

- Helm Chart README
- [Values Schema](./values.schema.json)
- [Example Values](./examples/)
- CHANGELOG
