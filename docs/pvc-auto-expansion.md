# Dynamic PVC Auto-Expansion

Stellar history archives grow indefinitely. The PVC auto-expansion controller monitors disk usage via Prometheus and automatically resizes PersistentVolumeClaims before they fill up — no manual intervention required.

---

## How It Works

```
┌─────────────────────────────────────────────────────────────────┐
│                    PVC Autoscaler Loop (5 min)                  │
│                                                                 │
│  For each StellarNode:                                          │
│    1. Query Prometheus for kubelet_volume_stats_used_bytes      │
│       and kubelet_volume_stats_capacity_bytes                   │
│    2. Compute usage % = used / capacity × 100                   │
│    3. If usage % ≥ threshold (default 80%):                     │
│       a. Verify StorageClass has allowVolumeExpansion: true     │
│       b. Compute new size = current × (1 + increment/100)      │
│       c. Patch PVC spec.resources.requests.storage             │
│       d. Emit Kubernetes Event + Prometheus metric              │
└─────────────────────────────────────────────────────────────────┘
```

The controller is implemented across two files:

- `src/controller/volume_resizer.rs` — core expansion logic, Prometheus queries, StorageClass checks
- `src/controller/pvc_autoscaler.rs` — background loop that drives the resizer for all StellarNodes

---

## Prerequisites

### StorageClass Must Support Expansion

The StorageClass backing your PVCs must have `allowVolumeExpansion: true`:

```yaml
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: stellar-gp3
provisioner: ebs.csi.aws.com
parameters:
  type: gp3
allowVolumeExpansion: true   # ← Required
volumeBindingMode: WaitForFirstConsumer
```

Verify your StorageClass:

```bash
kubectl get storageclass -o custom-columns=\
NAME:.metadata.name,PROVISIONER:.provisioner,EXPANSION:.allowVolumeExpansion
```

### Prometheus Must Expose kubelet Metrics

The controller queries:

```promql
kubelet_volume_stats_used_bytes{namespace="...", persistentvolumeclaim="..."}
kubelet_volume_stats_capacity_bytes{namespace="...", persistentvolumeclaim="..."}
```

These metrics are exposed by the kubelet and scraped by kube-prometheus-stack by default. Verify they are available:

```bash
curl -s http://prometheus:9090/api/v1/query \
  --data-urlencode 'query=kubelet_volume_stats_used_bytes' | jq '.data.result | length'
```

---

## Configuration

Configure the auto-expansion controller in the operator ConfigMap:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: stellar-operator-config
  namespace: stellar-system
data:
  # Enable/disable the PVC autoscaler
  pvc_autoscaler_enabled: "true"

  # Disk usage % that triggers expansion (0–100)
  pvc_expansion_threshold_pct: "80"

  # Percentage to grow the PVC by on each expansion
  pvc_expansion_increment_pct: "50"

  # Minimum seconds between expansions of the same PVC
  pvc_min_expansion_interval_secs: "3600"

  # Hard cap on auto-expansions per PVC (prevents runaway cost)
  pvc_max_expansions: "20"

  # Prometheus endpoint to query disk usage from
  prometheus_endpoint: "http://prometheus-operated.monitoring:9090"
```

### Configuration Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `pvc_autoscaler_enabled` | `true` | Enable/disable the autoscaler |
| `pvc_expansion_threshold_pct` | `80` | Usage % that triggers expansion |
| `pvc_expansion_increment_pct` | `50` | How much to grow the PVC (% of current size) |
| `pvc_min_expansion_interval_secs` | `3600` | Minimum time between expansions (1 hour) |
| `pvc_max_expansions` | `20` | Maximum auto-expansions per PVC |
| `prometheus_endpoint` | `http://prometheus:9090` | Prometheus query endpoint |

---

## Expansion Lifecycle

### Annotations

The controller tracks expansion state via PVC annotations:

| Annotation | Description |
|------------|-------------|
| `stellar.org/auto-expansion-count` | Number of times this PVC has been auto-expanded |
| `stellar.org/last-auto-expansion` | Unix timestamp of the last expansion |
| `stellar.org/expansion-in-flight` | Set to `"true"` while an expansion is pending |

```bash
# Check expansion state for a PVC
kubectl get pvc -n stellar validator-data \
  -o jsonpath='{.metadata.annotations}' | jq .
```

### Expansion States

| State | Description |
|-------|-------------|
| `BelowThreshold` | Usage is below the threshold; no action |
| `Expanded` | PVC was successfully patched with a larger storage request |
| `InFlight` | An expansion is already pending; waiting for storage provider |
| `StorageClassUnsupported` | StorageClass does not support expansion |
| `MaxExpansionsReached` | PVC has hit the expansion cap |
| `TooSoon` | Not enough time has elapsed since the last expansion |
| `QuotaExceeded` | Expansion would exceed a ResourceQuota |

---

## Edge Case Handling

### StorageClass Does Not Support Expansion

The controller checks `allowVolumeExpansion` on the StorageClass before attempting to patch the PVC. If expansion is not supported, it logs a warning and emits a `Warning` Kubernetes Event — it does not attempt the patch.

```bash
# Check for unsupported StorageClass events
kubectl get events -n stellar --field-selector reason=StorageClassUnsupported
```

**Resolution:** Update the StorageClass to set `allowVolumeExpansion: true`, or migrate to a StorageClass that supports expansion.

### Storage Quota Exceeded

If a `ResourceQuota` would be exceeded by the expansion, the controller skips the expansion and emits a `Warning` event with reason `StorageQuotaExceeded`.

```bash
# Check current quota usage
kubectl describe resourcequota -n stellar

# Check for quota events
kubectl get events -n stellar --field-selector reason=StorageQuotaExceeded
```

**Resolution:** Increase the `ResourceQuota` limit or delete unused PVCs.

### Expansion Already In-Flight

When a PVC is patched, the storage provider may take minutes to provision the additional capacity. The controller sets `stellar.org/expansion-in-flight: "true"` and skips re-queuing until the expansion completes.

The in-flight annotation is cleared automatically once the PVC's `status.capacity` reflects the new size.

### Maximum Expansions Reached

To prevent runaway storage costs, each PVC can be auto-expanded at most `pvc_max_expansions` times (default: 20). When this limit is reached, the controller emits a `Warning` event with reason `MaxExpansionsReached`.

```bash
# Check expansion count for a PVC
kubectl get pvc -n stellar validator-data \
  -o jsonpath='{.metadata.annotations.stellar\.org/auto-expansion-count}'
```

**Resolution:** Manually resize the PVC to a larger value and reset the annotation:

```bash
# Manually expand the PVC
kubectl patch pvc validator-data -n stellar \
  --type=merge \
  -p '{"spec":{"resources":{"requests":{"storage":"2Ti"}}}}'

# Reset the expansion counter (optional — allows auto-expansion to resume)
kubectl annotate pvc validator-data -n stellar \
  stellar.org/auto-expansion-count=0 --overwrite
```

---

## Monitoring

### Prometheus Metrics

The autoscaler emits the following metrics:

```
# Number of PVC expansions triggered (counter)
stellar_pvc_expansions_total{namespace, pvc_name, node_name}

# Current disk usage percentage (gauge)
stellar_pvc_disk_usage_pct{namespace, pvc_name, node_name}

# Number of expansion failures (counter)
stellar_pvc_expansion_failures_total{namespace, pvc_name, reason}
```

### Alerting Rules

```yaml
groups:
  - name: pvc_autoscaler
    rules:
      - alert: PvcDiskUsageHigh
        expr: stellar_pvc_disk_usage_pct > 75
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "PVC {{ $labels.pvc_name }} is {{ $value }}% full — expansion may trigger soon"

      - alert: PvcExpansionFailed
        expr: increase(stellar_pvc_expansion_failures_total[10m]) > 0
        labels:
          severity: critical
        annotations:
          summary: "PVC expansion failed for {{ $labels.pvc_name }}: {{ $labels.reason }}"

      - alert: PvcMaxExpansionsReached
        expr: |
          kube_persistentvolumeclaim_annotations{
            annotation_stellar_org_auto_expansion_count!=""
          } and on(persistentvolumeclaim)
          (kube_persistentvolumeclaim_annotations{
            annotation_stellar_org_auto_expansion_count="20"
          })
        labels:
          severity: warning
        annotations:
          summary: "PVC {{ $labels.persistentvolumeclaim }} has reached max auto-expansions"
```

### Kubernetes Events

```bash
# View all PVC expansion events
kubectl get events -n stellar \
  --field-selector reason=PvcAutoExpanded \
  --sort-by='.lastTimestamp'

# View all expansion warnings
kubectl get events -n stellar \
  --field-selector type=Warning \
  | grep -E "MaxExpansions|QuotaExceeded|StorageClass"
```

---

## Example: Full Setup

```yaml
# 1. StorageClass with expansion enabled
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: stellar-gp3
provisioner: ebs.csi.aws.com
parameters:
  type: gp3
  iops: "16000"
  throughput: "1000"
allowVolumeExpansion: true
volumeBindingMode: WaitForFirstConsumer
---
# 2. StellarNode using the expandable StorageClass
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator
  namespace: stellar
spec:
  nodeType: Validator
  network: Mainnet
  version: "v21.0.0"
  storage:
    storageClass: "stellar-gp3"
    size: "500Gi"          # Initial size; auto-expanded as needed
    retentionPolicy: Retain
  validatorConfig:
    seedSecretRef: "validator-seed"
    enableHistoryArchive: true
---
# 3. Operator config enabling the autoscaler
apiVersion: v1
kind: ConfigMap
metadata:
  name: stellar-operator-config
  namespace: stellar-system
data:
  pvc_autoscaler_enabled: "true"
  pvc_expansion_threshold_pct: "80"
  pvc_expansion_increment_pct: "50"
  pvc_max_expansions: "20"
  prometheus_endpoint: "http://prometheus-operated.monitoring:9090"
```

---

## Troubleshooting

### PVC Not Expanding Despite High Disk Usage

1. Verify the autoscaler is enabled:
   ```bash
   kubectl get configmap stellar-operator-config -n stellar-system \
     -o jsonpath='{.data.pvc_autoscaler_enabled}'
   ```

2. Verify Prometheus has the kubelet metrics:
   ```bash
   kubectl exec -n stellar-system deploy/stellar-operator -- \
     curl -s "http://prometheus-operated.monitoring:9090/api/v1/query?query=kubelet_volume_stats_used_bytes" \
     | jq '.data.result | length'
   ```

3. Check the operator logs for autoscaler activity:
   ```bash
   kubectl logs -n stellar-system deploy/stellar-operator | grep "pvc_autoscaler\|VolumeResizer"
   ```

4. Check if the StorageClass supports expansion:
   ```bash
   kubectl get storageclass stellar-gp3 -o jsonpath='{.allowVolumeExpansion}'
   ```

5. Check if the PVC has hit the expansion cap:
   ```bash
   kubectl get pvc -n stellar -o jsonpath='{range .items[*]}{.metadata.name}: {.metadata.annotations.stellar\.org/auto-expansion-count}{"\n"}{end}'
   ```

### PVC Expansion Stuck In-Flight

If `stellar.org/expansion-in-flight: "true"` has been set for more than 30 minutes:

1. Check the PVC status:
   ```bash
   kubectl describe pvc validator-data -n stellar
   ```
2. Look for `FileSystemResizePending` in the PVC conditions — this means the node needs to be restarted to complete the filesystem resize.
3. If the storage provider has already provisioned the new capacity, manually clear the annotation:
   ```bash
   kubectl annotate pvc validator-data -n stellar \
     stellar.org/expansion-in-flight- --overwrite
   ```

---

## See Also

- [Proactive Disk Scaling](proactive-disk-scaling.md)
- [Disk Scaling Quick Reference](disk-scaling-quick-reference.md)
- [Resource Limits](resource-limits.md)
- [Performance Tuning](performance-tuning.md)
