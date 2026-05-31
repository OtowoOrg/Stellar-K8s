# PodDisruptionBudget Guide for Stellar-K8s

This guide explains how Stellar-K8s uses Kubernetes PodDisruptionBudgets (PDBs) to protect the operator and validator nodes during voluntary disruptions such as node drains, cluster upgrades, and maintenance operations.

## What is a PodDisruptionBudget?

A **PodDisruptionBudget** is a Kubernetes API object that limits the number of pods of a replicated application that can be down simultaneously due to voluntary disruptions. Voluntary disruptions include:

- Node drains (e.g., `kubectl drain`)
- Cluster upgrades
- Node pool resizing
- Manual pod evictions

**Involuntary disruptions** (like node failures, network partitions, or pod crashes) are NOT affected by PDBs.

## Why PDBs Matter for Stellar-K8s

Stellar validator nodes and the operator itself are critical infrastructure components. During cluster maintenance:

- **Validator nodes** need to maintain quorum to continue processing transactions
- **The operator** needs to stay available to manage StellarNode resources
- **History archives** should remain accessible for node synchronization

Without PDBs, a cluster administrator could accidentally evict too many validators at once, potentially causing:
- **Quorum loss** - Network cannot reach consensus
- **Transaction delays** - Reduced capacity to process transactions
- **Synchronization issues** - Nodes cannot fetch historical data

## Default Configuration

By default, Stellar-K8s installs a PDB for the operator with:

```yaml
podDisruptionBudget:
  enabled: true
  minAvailable: 1
```

This ensures that at least one operator pod remains available during voluntary disruptions.

## Configuration Options

You can configure PDBs in your `values.yaml` or via Helm `--set` flags.

### Option 1: Using `minAvailable` (Recommended for Operators)

Ensures a minimum number of pods are always available:

```yaml
podDisruptionBudget:
  enabled: true
  minAvailable: 1  # At least 1 pod must be running
```

Or as a percentage:

```yaml
podDisruptionBudget:
  enabled: true
  minAvailable: "50%"  # At least 50% of pods must be running
```

### Option 2: Using `maxUnavailable` (Recommended for Validators)

Allows a maximum number of pods to be unavailable:

```yaml
podDisruptionBudget:
  enabled: true
  minAvailable: null   # Clear minAvailable
  maxUnavailable: 1    # At most 1 pod can be down
```

**For validator nodes, `maxUnavailable: 1` is the recommended default** because it:
- Allows maintenance to proceed one node at a time
- Prevents simultaneous eviction of multiple validators
- Maintains network quorum during upgrades

### Disabling PDB

To disable PDB creation (not recommended for production):

```yaml
podDisruptionBudget:
  enabled: false
```

## Installation Examples

### Default Installation (minAvailable: 1)

```bash
helm install stellar-operator charts/stellar-operator \
  --namespace stellar-system \
  --create-namespace
```

### Validator-Optimized (maxUnavailable: 1)

```bash
helm install stellar-operator charts/stellar-operator \
  --namespace stellar-system \
  --create-namespace \
  --set podDisruptionBudget.minAvailable=null \
  --set podDisruptionBudget.maxUnavailable=1
```

### High Availability (Percentage-Based)

```bash
helm install stellar-operator charts/stellar-operator \
  --namespace stellar-system \
  --create-namespace \
  --set podDisruptionBudget.minAvailable="50%"
```

### Custom Values File

Create `my-values.yaml`:

```yaml
podDisruptionBudget:
  enabled: true
  maxUnavailable: 1
```

Then install:

```bash
helm install stellar-operator charts/stellar-operator \
  --namespace stellar-system \
  --create-namespace \
  -f my-values.yaml
```

## Verifying PDB Status

### Check PDB Configuration

```bash
kubectl get pdb -n stellar-system
```

Example output:

```
NAME                                 MIN AVAILABLE   MAX UNAVAILABLE   ALLOWED DISRUPTIONS   AGE
stellar-operator                     1               N/A               0                     5m
```

### Detailed PDB Information

```bash
kubectl describe pdb stellar-operator -n stellar-system
```

This shows:
- Current pod count
- Desired minimum available
- How many disruptions are currently allowed
- Recent events

### Check if Disruptions are Blocked

If `ALLOWED DISRUPTIONS` is `0`, voluntary disruptions are blocked:

```bash
kubectl get pdb stellar-operator -n stellar-system -o jsonpath='{.status.disruptionsAllowed}'
```

## Handling PDBs During Emergency Cluster Maintenance

### Scenario 1: Planned Node Drain

When draining a node for maintenance:

1. **Check PDB status first:**
   ```bash
   kubectl get pdb -n stellar-system
   kubectl describe pdb stellar-operator -n stellar-system
   ```

2. **If disruptions are allowed (ALLOWED DISRUPTIONS > 0):**
   ```bash
   kubectl drain <node-name> --ignore-daemonsets --delete-emptydir-data
   ```

3. **If disruptions are blocked (ALLOWED DISRUPTIONS = 0):**
   - Wait for natural pod turnover, OR
   - Temporarily increase replica count, OR
   - Temporarily disable the PDB (see below)

### Scenario 2: Emergency Maintenance (PDB Override)

In emergency situations where you MUST drain a node despite PDB restrictions:

**Option A: Temporarily Delete the PDB**

```bash
# Save PDB configuration for later
kubectl get pdb stellar-operator -n stellar-system -o yaml > pdb-backup.yaml

# Delete the PDB
kubectl delete pdb stellar-operator -n stellar-system

# Perform emergency maintenance
kubectl drain <node-name> --ignore-daemonsets --delete-emptydir-data --force

# Restore the PDB
kubectl apply -f pdb-backup.yaml
```

**Option B: Temporarily Modify the PDB**

```bash
# Set maxUnavailable to a higher value temporarily
kubectl patch pdb stellar-operator -n stellar-system \
  --type='json' \
  -p='[{"op": "add", "path": "/spec/maxUnavailable", "value": 999}]'

# Perform maintenance
kubectl drain <node-name> --ignore-daemonsets --delete-emptydir-data

# Restore original configuration
kubectl patch pdb stellar-operator -n stellar-system \
  --type='json' \
  -p='[{"op": "remove", "path": "/spec/maxUnavailable"}]'
```

**Option C: Force Drain (Use with Extreme Caution)**

```bash
# This ignores PDBs and can cause service disruption
kubectl drain <node-name> --ignore-daemonsets --delete-emptydir-data --force --pod-selector=""
```

⚠️ **WARNING:** Force draining can cause:
- Validator quorum loss
- Transaction processing interruption
- Network instability

Only use in genuine emergencies.

### Scenario 3: Cluster Upgrade

For rolling cluster upgrades:

1. **Ensure operator has appropriate PDB:**
   ```bash
   kubectl get pdb -n stellar-system
   ```

2. **Upgrade node pool one node at a time:**
   ```bash
   # For each node in the pool:
   kubectl drain <node-name> --ignore-daemonsets --delete-emptydir-data
   # Wait for node to be ready again
   kubectl uncordon <node-name>
   ```

3. **Monitor PDB status between nodes:**
   ```bash
   kubectl get pdb -n stellar-system
   kubectl get pods -n stellar-system -o wide
   ```

## Best Practices

### DO ✅

- Keep PDBs enabled in production environments
- Use `maxUnavailable: 1` for validator workloads
- Monitor PDB status before planned maintenance
- Plan maintenance windows with PDB constraints in mind
- Use percentage-based PDBs for large deployments (e.g., `minAvailable: "75%"`)
- Test PDB behavior in non-production environments first

### DON'T ❌

- Don't disable PDBs in production without a compelling reason
- Don't force drains unless it's a genuine emergency
- Don't set `minAvailable` equal to total replicas (blocks all maintenance)
- Don't ignore PDB warnings during maintenance
- Don't set both `minAvailable` and `maxUnavailable` simultaneously (use one or the other)

## Troubleshooting

### PDB Blocking Drains

**Symptom:** `kubectl drain` hangs or reports "Cannot evict pod"

**Solution:**
1. Check PDB status: `kubectl describe pdb <pdb-name>`
2. Wait for pods to become ready naturally, OR
3. Temporarily increase replica count, OR
4. Follow emergency procedures above

### Pod Stuck in Terminating State

**Symptom:** Pod remains in `Terminating` state despite node drain

**Possible causes:**
- PDB blocking eviction
- Finalizers preventing deletion
- Volume detachment delays

**Solution:**
```bash
# Check what's blocking
kubectl describe pod <pod-name>

# Check PDB status
kubectl get pdb

# If PDB is the issue, follow emergency procedures above
```

### ALLOWED DISRUPTIONS Shows 0

**Meaning:** No voluntary disruptions are currently permitted

**Common causes:**
- Not enough healthy replicas
- Pod readiness issues
- PDB configuration too restrictive

**Solution:**
1. Check pod health: `kubectl get pods -n stellar-system`
2. Fix any unhealthy pods
3. Wait for pods to become ready
4. Consider adjusting PDB if configuration is too restrictive

## Monitoring and Alerting

### Prometheus Metrics

Monitor these PDB-related metrics:

```promql
# Current disruptions allowed
kube_poddisruptionbudget_status_disruptions_allowed{namespace="stellar-system"}

# Expected disruptions desired
kube_poddisruptionbudget_status_desired_healthy{namespace="stellar-system"}

# Current pod count
kube_poddisruptionbudget_status_current_healthy{namespace="stellar-system"}

# Alert if no disruptions allowed for extended period
kube_poddisruptionbudget_status_disruptions_allowed{namespace="stellar-system"} == 0
```

### Recommended Alerts

```yaml
# Alert if PDB is blocking all disruptions for more than 1 hour
- alert: PDBBlockingDisruptions
  expr: kube_poddisruptionbudget_status_disruptions_allowed{namespace="stellar-system"} == 0
  for: 1h
  labels:
    severity: warning
  annotations:
    summary: "PDB blocking voluntary disruptions"
    description: "PDB {{ $labels.poddisruptionbudget }} has 0 allowed disruptions for more than 1 hour"
```

## Related Documentation

- [Kubernetes PodDisruptionBudget Documentation](https://kubernetes.io/docs/tasks/run-application/configure-pdb/)
- [Stellar-K8s Quickstart](quickstart.md)
- [Stellar-K8s Resource Limits](resource-limits.md)
- [Stellar-K8s Health Checks](health-checks.md)

## Support

For issues or questions about PDB configuration:
- Open an issue on [GitHub](https://github.com/stellar/stellar-k8s/issues)
- Check existing documentation in the `docs/` directory
- Review the [FMEA](fmea-stellarnode.md) for failure mode analysis
