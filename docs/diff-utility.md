# Live Diff Utility Guide

This guide explains how to use the `stellar-operator diff` command to compare the operator's desired state with what is actually deployed in the Kubernetes cluster.

## Overview

When troubleshooting operator issues, it can be difficult to determine:
- Is the operator failing to apply changes?
- Did the operator apply something that Kubernetes rejected?
- What exactly differs between what should be deployed and what is deployed?

The `diff` subcommand provides visibility into these differences, similar to `kubectl diff`, but with operator-specific insights including:
- ConfigMap contents (stellar-core.cfg, captive-core.cfg)
- Resource limits and requests
- Service configurations
- StatefulSets/Deployments
- PVCs, HPAs, NetworkPolicies, PDBs, and more

## Features

- **Colored Terminal Output**: Easy-to-read diff with color coding
- **Multiple Output Formats**: Terminal, JSON, and unified diff formats
- **ConfigMap Inspection**: View actual configuration file contents
- **Resource Coverage**: Compares all operator-managed resources
- **Change Detection**: Identifies added, removed, modified, and unchanged resources
- **Scripting Support**: JSON output for automation and integration

## Usage

### Basic Usage

Show differences for a StellarNode:

```bash
stellar-operator diff --name my-validator --namespace stellar
```

This will:
1. Fetch the StellarNode CRD from the cluster
2. Calculate the desired state for all managed resources
3. Fetch the live state from Kubernetes API
4. Display a colored diff showing differences

### Command Options

```
stellar-operator diff [OPTIONS] --name <NAME>

Options:
  -n, --name <NAME>              Name of the StellarNode resource to diff
  -N, --namespace <NAMESPACE>    Kubernetes namespace [default: default]
      --format <FORMAT>          Output format: terminal, json, unified [default: terminal]
      --show-config              Show full ConfigMap contents
      --all-resources            Include all resources (even unchanged ones)
      --summary                  Show only summary, not full diff
      --context <CONTEXT>        Kubernetes context to use
  -h, --help                     Print help
```

## Examples

### Example 1: Basic Diff (Terminal Output)

```bash
stellar-operator diff \
  --name my-validator \
  --namespace stellar-system
```

**Output:**
```
════════════════════════════════════════════════════════════════════════════════
🔍 Diff for StellarNode: stellar-system/my-validator
════════════════════════════════════════════════════════════════════════════════

📊 Summary:
   Total resources:   8
   Unchanged:       6
   To be added:     1
   To be removed:   0
   To be modified:  1

➕ ConfigMap/my-validator-config (added)

✏️  Deployment/my-validator-deployment (modified)
   Changed fields:
     - spec.template.spec.containers[0].resources.limits.cpu
     - spec.template.spec.containers[0].resources.requests.memory

   --- a/Deployment/my-validator-deployment
   +++ b/Deployment/my-validator-deployment
   -  cpu: "2"
   +  cpu: "4"
   -  memory: "2Gi"
   +  memory: "4Gi"

✅ Service/my-validator-service (unchanged)
✅ PVC/my-validator-data (unchanged)
...

════════════════════════════════════════════════════════════════════════════════
```

### Example 2: JSON Output (for Scripting)

```bash
stellar-operator diff \
  --name my-validator \
  --namespace stellar-system \
  --format json
```

**Output:**
```json
{
  "node_name": "my-validator",
  "namespace": "stellar-system",
  "node_exists": true,
  "resources": [
    {
      "kind": "ConfigMap",
      "name": "my-validator-config",
      "namespace": "stellar-system",
      "status": "added",
      "changed_fields": []
    },
    {
      "kind": "Deployment",
      "name": "my-validator-deployment",
      "namespace": "stellar-system",
      "status": "modified",
      "changed_fields": [
        "spec.template.spec.containers[0].resources.limits.cpu",
        "spec.template.spec.containers[0].resources.requests.memory"
      ]
    }
  ],
  "summary": {
    "total": 8,
    "unchanged": 6,
    "added": 1,
    "removed": 0,
    "modified": 1
  }
}
```

### Example 3: Show ConfigMap Contents

View the full stellar-core.cfg configuration:

```bash
stellar-operator diff \
  --name my-validator \
  --namespace stellar-system \
  --show-config
```

**Output:**
```
✏️  ConfigMap/my-validator-config (modified)
   Changed fields:
     - data.stellar-core.cfg

   📄 ConfigMap data:
   ── stellar-core.cfg ──
     # Stellar Core Configuration
     NETWORK_PASSPHRASE="Stellar Testnet ; September 2015"
     PEER_PORT=11625
     HTTP_PORT=11626
     LOG_FILE_PATH="/var/log/stellar-core.log"
     ...
     [QUORUM]
     ...
     (8 more lines)
```

### Example 4: Summary Only

Quick status check without full diff:

```bash
stellar-operator diff \
  --name my-validator \
  --namespace stellar-system \
  --summary
```

**Output:**
```
════════════════════════════════════════════════════════════════════════════════
🔍 Diff for StellarNode: stellar-system/my-validator
════════════════════════════════════════════════════════════════════════════════

📊 Summary:
   Total resources:   8
   Unchanged:       6
   To be added:     1
   To be removed:   0
   To be modified:  1

════════════════════════════════════════════════════════════════════════════════
```

### Example 5: Show All Resources

Include unchanged resources in output:

```bash
stellar-operator diff \
  --name my-validator \
  --namespace stellar-system \
  --all-resources
```

### Example 6: Unified Diff Format

Standard unified diff format (useful for patch tools):

```bash
stellar-operator diff \
  --name my-validator \
  --namespace stellar-system \
  --format unified
```

### Example 7: Different Kubernetes Context

```bash
stellar-operator diff \
  --name my-validator \
  --namespace stellar-system \
  --context production-cluster
```

## Interpreting the Output

### Status Indicators

| Icon | Status | Meaning |
|------|--------|---------|
| ➕ | `added` | Resource will be created |
| ❌ | `removed` | Resource will be deleted |
| ✏️ | `modified` | Resource will be updated |
| ✅ | `unchanged` | Resource matches desired state |

### Color Coding

- **Green**: Resources to be added
- **Red**: Resources to be removed
- **Yellow**: Resources to be modified
- **Gray**: Unchanged resources

### Common Changes

#### Resource Limits Modified

```yaml
Changed fields:
  - spec.template.spec.containers[0].resources.limits.cpu
  - spec.template.spec.containers[0].resources.requests.memory
```

**Cause**: StellarNode spec.resources was updated  
**Action**: Operator will apply new limits on next reconciliation

#### ConfigMap Modified

```yaml
Changed fields:
  - data.stellar-core.cfg
```

**Cause**: Quorum set, network, or validator configuration changed  
**Action**: Pod will restart with new configuration

#### Labels Modified

```yaml
Changed fields:
  - labels.stellar.org/version (missing)
  - labels.app.kubernetes.io/component
```

**Cause**: Operator version upgrade or component type change  
**Action**: Labels will be updated

## Use Cases

### Use Case 1: Troubleshooting Failed Reconciliation

**Problem**: StellarNode status shows errors, but logs are unclear.

**Solution**:
```bash
# Check what the operator is trying to apply
stellar-operator diff --name failing-node --namespace stellar

# Look for validation errors in the diff
# e.g., invalid resource specs, missing required fields
```

### Use Case 2: Verifying Configuration Changes

**Problem**: You updated the StellarNode spec but aren't sure what will change.

**Solution**:
```bash
# Before applying changes, run diff to see impact
stellar-operator diff --name my-node --namespace stellar --show-config

# Review the changes, then monitor reconciliation
kubectl describe stellarnode my-node -n stellar
```

### Use Case 3: Debugging Pod Scheduling Issues

**Problem**: Pods are pending or failing to schedule.

**Solution**:
```bash
# Check if resource requests are too high
stellar-operator diff --name my-node --namespace stellar --format json | \
  jq '.resources[] | select(.kind == "Deployment") | .changed_fields'

# Compare with cluster capacity
kubectl describe nodes | grep -A 5 "Allocated resources"
```

### Use Case 4: Audit Trail

**Problem**: Need to document what changed between deployments.

**Solution**:
```bash
# Save diff output for audit
stellar-operator diff \
  --name my-node \
  --namespace stellar \
  --format json > diff-$(date +%Y%m%d-%H%M%S).json

# Or unified format for patch files
stellar-operator diff \
  --name my-node \
  --namespace stellar \
  --format unified > changes.patch
```

### Use Case 5: CI/CD Validation

**Problem**: Want to validate changes before applying to production.

**Solution**:
```yaml
# In CI/CD pipeline
- name: Validate StellarNode changes
  run: |
    DIFF_OUTPUT=$(stellar-operator diff \
      --name prod-validator \
      --namespace production \
      --format json)
    
    # Check for dangerous changes
    echo "$DIFF_OUTPUT" | jq -e '
      .summary.removed == 0 and
      .resources[].changed_fields[] | 
      contains("resources.limits") | not
    '
```

## Managed Resources

The diff utility compares the following operator-managed resources:

| Resource | Kind | Description |
|----------|------|-------------|
| Configuration | ConfigMap | stellar-core.cfg, captive-core.cfg, environment variables |
| Workload (Validator) | StatefulSet | Validator pods with persistent storage |
| Workload (Horizon/RPC) | Deployment | Stateless Horizon or Soroban RPC pods |
| Networking | Service | ClusterIP, LoadBalancer, or NodePort services |
| Storage | PersistentVolumeClaim | Data volumes for validators |
| Scaling | HorizontalPodAutoscaler | Auto-scaling configuration |
| Security | NetworkPolicy | Pod network isolation rules |
| Availability | PodDisruptionBudget | Voluntary disruption protection |
| Ingress | Ingress | HTTP/HTTPS routing (if configured) |

## Integration with Monitoring

### Prometheus Metrics

Track diff statistics:

```promql
# Count of modified resources
stellar_operator_diff_modified_resources{namespace, node_name}

# Count of added resources
stellar_operator_diff_added_resources{namespace, node_name}

# Time since last diff check
stellar_operator_diff_last_check_timestamp
```

### Alerting Rules

Example alerts for diff monitoring:

```yaml
# Alert if resources are consistently out of sync
- alert: StellarNodeDriftDetected
  expr: |
    stellar_operator_diff_modified_resources > 0
  for: 15m
  labels:
    severity: warning
  annotations:
    summary: "StellarNode {{ $labels.node_name }} has drifted from desired state"
    description: "{{ $value }} resources are modified and pending reconciliation"

# Alert if critical resources are missing
- alert: StellarNodeResourceMissing
  expr: |
    stellar_operator_diff_added_resources{kind="ConfigMap"} > 0
  for: 5m
  labels:
    severity: critical
  annotations:
    summary: "Critical ConfigMap missing for {{ $labels.node_name }}"
```

## Troubleshooting

### Issue: "StellarNode not found"

**Cause**: The specified StellarNode doesn't exist in the namespace.

**Solution**:
```bash
# Verify the StellarNode exists
kubectl get stellarnodes -n <namespace>

# Check namespace
kubectl get namespaces
```

### Issue: Permission Denied

**Cause**: Insufficient RBAC permissions to read resources.

**Solution**: Ensure the service account has:
```yaml
- apiGroups: [""]
  resources: ["configmaps", "services", "persistentvolumeclaims"]
  verbs: ["get", "list"]
- apiGroups: ["apps"]
  resources: ["deployments", "statefulsets"]
  verbs: ["get", "list"]
```

### Issue: No Differences Shown But Pod Is Wrong

**Cause**: The diff shows metadata-level changes, not full spec comparison.

**Solution**:
```bash
# Use --show-config for ConfigMaps
stellar-operator diff --name my-node --show-config

# Or use kubectl for detailed pod spec
kubectl get pod <pod-name> -o yaml > live.yaml
# Compare with desired state manually
```

### Issue: JSON Output Too Large

**Cause**: Full resource specs are verbose.

**Solution**:
```bash
# Use --summary for quick checks
stellar-operator diff --name my-node --summary

# Or filter JSON output
stellar-operator diff --name my-node --format json | \
  jq '.summary'
```

## Best Practices

### DO ✅

- Run diff before making configuration changes
- Use `--show-config` to review ConfigMap changes
- Save diff output for audit trails
- Integrate diff into CI/CD pipelines
- Monitor diff metrics for drift detection

### DON'T ❌

- Don't rely solely on diff for security validation
- Don't ignore "removed" resources without investigation
- Don't run diff excessively (rate limit considerations)
- Don't use diff as a substitute for proper testing

## Related Commands

```bash
# Compare with kubectl diff (for standard resources)
kubectl diff -f stellar-node.yaml

# Get current state
kubectl get stellarnode my-node -n stellar -o yaml

# Describe resource with events
kubectl describe stellarnode my-node -n stellar

# Watch reconciliation
kubectl get stellarnode my-node -n stellar -w
```

## Related Documentation

- [Stellar-K8s API Reference](api-reference.md)
- [Stellar-K8s Resource Limits](resource-limits.md)
- [Stellar-K8s Health Checks](health-checks.md)
- [kubectl diff Documentation](https://kubernetes.io/docs/reference/generated/kubectl/kubectl-commands#diff)

## Support

For issues or questions about the diff utility:
- Open an issue on [GitHub](https://github.com/stellar/stellar-k8s/issues)
- Check existing documentation in the `docs/` directory
- Review operator logs: `kubectl logs -l app=stellar-operator -n stellar-system`
