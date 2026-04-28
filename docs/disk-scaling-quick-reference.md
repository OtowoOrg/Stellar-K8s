# Disk Scaling Quick Reference

Quick reference guide for operators managing Stellar nodes with proactive disk scaling.

## Quick Commands

### Check Disk Usage
```bash
# Get current disk usage metric
kubectl get --raw /metrics | grep stellar_pvc_disk_usage_percent

# Check PVC size
kubectl get pvc -n stellar-system

# View expansion history
kubectl get events -n stellar-system --field-selector reason=DiskExpanded

# Check expansion count
kubectl get pvc stellar-node-data -n stellar-system -o jsonpath='{.metadata.annotations.stellar\.org/disk-expansion-count}'
```

### Monitor Expansions
```bash
# Watch PVC size changes
kubectl get pvc -n stellar-system -w

# View recent expansion events
kubectl get events -n stellar-system --sort-by='.lastTimestamp' | grep Disk

# Check operator logs for expansion activity
kubectl logs -n stellar-system -l app=stellar-operator | grep -i "disk\|expansion\|pvc"
```

### Manual Expansion
```bash
# If automatic expansion fails, manually expand:
kubectl patch pvc stellar-node-data -n stellar-system \
  -p '{"spec":{"resources":{"requests":{"storage":"200Gi"}}}}'
```

## Configuration Cheat Sheet

### Default Values
```yaml
diskScaling:
  enabled: true                    # Enable automatic scaling
  expansionThreshold: 80           # Trigger at 80% usage
  expansionIncrement: 50           # Increase by 50%
  minExpansionIntervalSecs: 3600   # 1 hour between expansions
  maxExpansions: 10                # Max 10 expansions per PVC
```

### Conservative Settings (Lower Costs)
```yaml
diskScaling:
  enabled: true
  expansionThreshold: 85           # Wait until 85% full
  expansionIncrement: 30           # Smaller increments
  minExpansionIntervalSecs: 7200   # 2 hours between expansions
  maxExpansions: 15                # Allow more smaller expansions
```

### Aggressive Settings (Maximum Uptime)
```yaml
diskScaling:
  enabled: true
  expansionThreshold: 70           # Expand early at 70%
  expansionIncrement: 100          # Double the size
  minExpansionIntervalSecs: 1800   # 30 minutes between expansions
  maxExpansions: 8                 # Fewer but larger expansions
```

## Troubleshooting Checklist

### Expansion Not Triggering

- [ ] Is disk scaling enabled? Check `diskScaling.enabled: true` in operator config
- [ ] Is usage above threshold? Check `stellar_pvc_disk_usage_percent` metric
- [ ] Has minimum interval passed? Check `stellar.org/last-disk-expansion` annotation
- [ ] Is max expansions reached? Check `stellar.org/disk-expansion-count` annotation
- [ ] Does StorageClass support expansion? Check `allowVolumeExpansion: true`
- [ ] Are there operator errors? Check operator logs

### Expansion Failed

- [ ] Check operator logs: `kubectl logs -n stellar-system -l app=stellar-operator`
- [ ] Check PVC events: `kubectl describe pvc stellar-node-data -n stellar-system`
- [ ] Verify StorageClass: `kubectl get storageclass <name> -o yaml`
- [ ] Check cloud provider limits (AWS: 16TiB for gp3, GCP: 64TiB)
- [ ] Verify RBAC permissions for operator to patch PVCs

## Metrics Reference

| Metric | Type | Description |
|--------|------|-------------|
| `stellar_pvc_disk_usage_percent` | Gauge | Current disk usage (0-100) |
| `stellar_pvc_expansion_total` | Counter | Total expansion events |
| `stellar_pvc_size_bytes` | Gauge | Current PVC size in bytes |
| `stellar_pvc_expansion_count` | Gauge | Number of expansions performed |

## Alert Thresholds

| Alert | Threshold | Action |
|-------|-----------|--------|
| HighDiskUsage | 75% | Monitor, expansion will trigger at 80% |
| MaxExpansionsReached | 10 expansions | Manual intervention required |
| DiskExpansionStalled | 80% for 15min | Check operator logs, may need manual expansion |
| RapidDiskGrowth | 3 expansions in 6h | Investigate abnormal growth |

## Cost Estimation

### AWS EBS gp3 Pricing (Example: us-east-1)
- Base: $0.08/GB/month
- IOPS: $0.005/provisioned IOPS/month (above 3,000)
- Throughput: $0.04/MB/s/month (above 125 MB/s)

### Expansion Cost Example
```
Initial:     100Gi × $0.08 = $8.00/month
Expansion 1: 150Gi × $0.08 = $12.00/month (+$4.00)
Expansion 2: 225Gi × $0.08 = $18.00/month (+$6.00)
Expansion 3: 337Gi × $0.08 = $26.96/month (+$8.96)
Total after 3 expansions: $26.96/month (237% increase)
```

## Storage Class Template

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: stellar-fast-ssd
provisioner: ebs.csi.aws.com  # or kubernetes.io/gce-pd for GCP
parameters:
  type: gp3                    # or pd-ssd for GCP
  iops: "3000"
  throughput: "125"
  encrypted: "true"
allowVolumeExpansion: true     # REQUIRED for automatic expansion
volumeBindingMode: WaitForFirstConsumer
reclaimPolicy: Retain
```

## Common Scenarios

### Scenario 1: First Expansion
```
Initial: 100Gi, 85Gi used (85%)
Action: Expand to 150Gi
Result: 85Gi used (57%)
Next expansion allowed: After 1 hour
```

### Scenario 2: Rapid Growth
```
T+0h:  100Gi → 150Gi (expansion #1)
T+1h:  150Gi → 225Gi (expansion #2)
T+2h:  225Gi → 337Gi (expansion #3)
Alert: RapidDiskGrowth triggered
Action: Investigate ledger growth rate
```

### Scenario 3: Max Expansions Reached
```
Current: 3.3Ti (after 10 expansions)
Usage: 85%
Status: MaxExpansionsReached alert
Action: Manual intervention required
Options:
  1. Increase maxExpansions limit
  2. Manually expand to larger size
  3. Enable archive pruning
  4. Switch to Recent history mode
```

## Best Practices

1. **Monitor Regularly**: Set up alerts for all disk scaling metrics
2. **Review Costs**: Check expansion history monthly
3. **Plan Capacity**: Estimate growth rate and initial PVC size
4. **Test Expansion**: Verify storage class supports expansion before production
5. **Document Changes**: Keep audit trail of manual interventions
6. **Set Budgets**: Configure cloud provider budget alerts
7. **Archive Pruning**: Consider enabling for full history nodes
8. **Backup Strategy**: Ensure snapshots before major expansions

## Emergency Procedures

### Disk Full Despite Expansion
```bash
# 1. Check if expansion is stuck
kubectl describe pvc stellar-node-data -n stellar-system

# 2. Check filesystem resize
kubectl exec -n stellar-system stellar-node-0 -- df -h /data

# 3. Manual filesystem resize (if needed)
kubectl exec -n stellar-system stellar-node-0 -- resize2fs /dev/xvda1

# 4. Emergency manual expansion
kubectl patch pvc stellar-node-data -n stellar-system \
  -p '{"spec":{"resources":{"requests":{"storage":"500Gi"}}}}'
```

### Disable Automatic Expansion
```bash
# Update operator config
kubectl edit configmap stellar-operator-config -n stellar-system

# Set diskScaling.enabled: false
# Restart operator to apply
kubectl rollout restart deployment stellar-operator -n stellar-system
```

## Support Resources

- [Full Documentation](./proactive-disk-scaling.md)
- [Example Configuration](../config/samples/disk-scaling-example.yaml)
- [Troubleshooting Guide](./troubleshooting.md)
- [Cost Optimization](./cost-optimization.md)
