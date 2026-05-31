# Proactive Disk Scaling

Automatically increase the size of EBS/GCP volumes as the Stellar ledger database grows, preventing 'Disk Full' outages without manual intervention.

## Overview

Stellar ledger growth is unpredictable. Running out of disk space is a fatal error for a validator. The proactive disk scaling feature monitors disk usage on managed PersistentVolumeClaims (PVCs) and automatically triggers volume expansion when usage exceeds a configurable threshold.

## Features

- **Automatic Monitoring**: Continuously monitors disk usage percentage on PVCs attached to Stellar nodes
- **Threshold-Based Expansion**: Automatically triggers PVC expansion when usage exceeds threshold (default: 80%)
- **Storage Provider Coordination**: Respects storage provider expansion limits and capabilities
- **Cost Auditing**: Logs every expansion event for cost tracking and auditing
- **Rate Limiting**: Prevents rapid successive expansions with configurable minimum intervals
- **Safety Limits**: Maximum expansion count per PVC to prevent runaway costs
- **Prometheus Metrics**: Exports disk usage and expansion metrics for monitoring

## Supported Storage Providers

| Provider | Online Expansion | Notes |
|----------|------------------|-------|
| AWS EBS | ✅ Yes | No pod restart required |
| GCP Persistent Disks | ✅ Yes | No pod restart required |
| Azure Disks | ✅ Yes | No pod restart required |
| Local Storage | ❌ No | Requires manual intervention |

## Configuration

Disk scaling is configured in the operator ConfigMap (`config/operator-config.yaml`):

```yaml
diskScaling:
  # Enable automatic disk scaling
  enabled: true

  # Disk usage percentage that triggers expansion (0-100)
  expansionThreshold: 80

  # Percentage to increase disk size by
  expansionIncrement: 50

  # Minimum time between expansions (seconds)
  minExpansionIntervalSecs: 3600

  # Maximum number of expansions allowed per PVC
  maxExpansions: 10
```

### Configuration Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `enabled` | boolean | `true` | Enable/disable automatic disk scaling |
| `expansionThreshold` | integer | `80` | Disk usage percentage (0-100) that triggers expansion |
| `expansionIncrement` | integer | `50` | Percentage to increase disk size by (e.g., 50 = 50% increase) |
| `minExpansionIntervalSecs` | integer | `3600` | Minimum seconds between expansions (prevents rapid successive expansions) |
| `maxExpansions` | integer | `10` | Maximum number of expansions per PVC (safety limit) |

## How It Works

1. **Monitoring**: During each reconciliation loop, the operator checks disk usage for each Stellar node's PVC
2. **Threshold Check**: If usage exceeds `expansionThreshold`, expansion is triggered
3. **Rate Limiting**: Checks if minimum interval has passed since last expansion
4. **Safety Check**: Verifies expansion count hasn't exceeded `maxExpansions`
5. **Storage Class Validation**: Confirms the storage class supports volume expansion
6. **Expansion**: Patches the PVC with new size (current size + `expansionIncrement`%)
7. **Event Logging**: Emits Kubernetes event and updates Prometheus metrics
8. **Annotation Tracking**: Updates PVC annotations with expansion count and timestamp

## Example Expansion Flow

Initial state:
- PVC size: 100Gi
- Disk usage: 85Gi (85%)
- Threshold: 80%

Expansion triggered:
- New size: 150Gi (100Gi + 50%)
- Expansion count: 1
- Event: `DiskExpanded` with details

After expansion:
- PVC size: 150Gi
- Disk usage: 85Gi (57%)
- Next expansion allowed after: 1 hour

## Kubernetes Events

The operator emits the following events:

### DiskExpanded (Normal)
```
PVC automatically expanded from 100Gi to 150Gi (expansion #1) due to high disk usage
```

### MaxDiskExpansionsReached (Warning)
```
PVC has reached maximum expansion limit (10). Manual intervention required.
```

### DiskExpansionFailed (Warning)
```
Failed to expand PVC: <error reason>
```

## Prometheus Metrics

The following metrics are exported for monitoring:

### stellar_pvc_disk_usage_percent
- **Type**: Gauge
- **Description**: Current disk usage percentage (0-100)
- **Labels**: `namespace`, `name`, `node_type`, `network`, `hardware_generation`

### stellar_pvc_expansion_total
- **Type**: Counter
- **Description**: Total number of PVC expansion events
- **Labels**: `namespace`, `name`, `node_type`, `network`, `hardware_generation`

### stellar_pvc_size_bytes
- **Type**: Gauge
- **Description**: Current PVC size in bytes
- **Labels**: `namespace`, `name`, `node_type`, `network`, `hardware_generation`

### stellar_pvc_expansion_count
- **Type**: Gauge
- **Description**: Number of expansions performed on this PVC
- **Labels**: `namespace`, `name`, `node_type`, `network`, `hardware_generation`

## Alerting

Example Prometheus alerts:

```yaml
groups:
  - name: stellar-disk-scaling
    rules:
      - alert: HighDiskUsage
        expr: stellar_pvc_disk_usage_percent > 75
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "High disk usage on {{ $labels.namespace }}/{{ $labels.name }}"
          description: "Disk usage is {{ $value }}% (threshold: 80%)"

      - alert: MaxExpansionsReached
        expr: stellar_pvc_expansion_count >= 10
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Maximum disk expansions reached for {{ $labels.namespace }}/{{ $labels.name }}"
          description: "PVC has been expanded {{ $value }} times. Manual intervention required."

      - alert: DiskExpansionFailed
        expr: increase(stellar_pvc_expansion_total[5m]) == 0 and stellar_pvc_disk_usage_percent > 80
        for: 15m
        labels:
          severity: critical
        annotations:
          summary: "Disk expansion may have failed for {{ $labels.namespace }}/{{ $labels.name }}"
          description: "Disk usage is {{ $value }}% but no expansion occurred in the last 15 minutes"
```

## Cost Management

### Tracking Expansion Costs

Each expansion event is logged with:
- Timestamp
- Old size → New size
- Expansion count
- Node namespace/name

Query expansion history:
```bash
kubectl get events --field-selector reason=DiskExpanded -n stellar-system
```

### Cost Estimation

Example AWS EBS cost calculation:
- Initial: 100Gi @ $0.10/GB/month = $10/month
- After 1st expansion: 150Gi = $15/month (+$5)
- After 2nd expansion: 225Gi = $22.50/month (+$7.50)
- After 3rd expansion: 337Gi = $33.70/month (+$11.20)

Total cost after 3 expansions: $33.70/month (237% increase)

### Cost Controls

1. **Set appropriate threshold**: Higher threshold = fewer expansions
2. **Adjust increment**: Smaller increment = more frequent but smaller expansions
3. **Monitor expansion count**: Alert when approaching `maxExpansions`
4. **Review expansion history**: Identify nodes with excessive growth

## Troubleshooting

### Expansion Not Triggered

Check:
1. Is disk scaling enabled? (`diskScaling.enabled: true`)
2. Is usage above threshold? (Check `stellar_pvc_disk_usage_percent` metric)
3. Has minimum interval passed? (Check PVC annotation `stellar.org/last-disk-expansion`)
4. Has max expansions been reached? (Check PVC annotation `stellar.org/disk-expansion-count`)
5. Does storage class support expansion? (Check `allowVolumeExpansion: true`)

### Expansion Failed

Common causes:
1. **Storage class doesn't support expansion**: Add `allowVolumeExpansion: true` to StorageClass
2. **Cloud provider limits**: Check provider-specific volume size limits
3. **RBAC permissions**: Ensure operator has permission to patch PVCs
4. **Volume in use**: Some providers require pod restart (rare with modern CSI drivers)

### Manual Expansion

If automatic expansion fails or is disabled:

```bash
# Check current size
kubectl get pvc stellar-node-data -n stellar-system

# Manually expand
kubectl patch pvc stellar-node-data -n stellar-system -p '{"spec":{"resources":{"requests":{"storage":"200Gi"}}}}'

# Verify expansion
kubectl get pvc stellar-node-data -n stellar-system -w
```

## Best Practices

1. **Set Conservative Thresholds**: Use 80% threshold to allow time for expansion before disk fills
2. **Monitor Metrics**: Set up alerts for high disk usage and expansion failures
3. **Review Costs Regularly**: Track expansion events and associated costs
4. **Plan for Growth**: Estimate ledger growth rate and adjust initial PVC size accordingly
5. **Test Expansion**: Verify storage class supports expansion before production deployment
6. **Document Expansions**: Keep audit trail of all expansion events for cost analysis

## Storage Class Configuration

Ensure your StorageClass supports volume expansion:

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: fast-ssd
provisioner: kubernetes.io/aws-ebs
parameters:
  type: gp3
  iops: "3000"
  throughput: "125"
allowVolumeExpansion: true  # Required for automatic expansion
volumeBindingMode: WaitForFirstConsumer
```

## Limitations

1. **Local Storage**: Cannot be expanded automatically (requires manual intervention)
2. **Shrinking**: Volume shrinking is not supported (Kubernetes limitation)
3. **Filesystem Resize**: Some filesystems may require manual resize (rare with modern CSI drivers)
4. **Provider Limits**: Each cloud provider has maximum volume size limits
5. **Cost Impact**: Automatic expansion increases storage costs

## Related Documentation

- [Storage Configuration](./storage-configuration.md)
- [Monitoring and Metrics](./monitoring.md)
- [Cost Optimization](./cost-optimization.md)
- [Troubleshooting Guide](./troubleshooting.md)
