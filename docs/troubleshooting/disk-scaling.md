# Disk Scaling Troubleshooting

This guide provides procedures for diagnosing and resolving issues with the operator's **Proactive Disk Scaling** feature.

---

## 1. How Proactive Disk Scaling Works
The operator automatically monitors the storage utilization of persistent volumes attached to Stellar nodes. When the storage threshold is breached (typically >80%), the operator scales the PVC size in the resource spec to prevent "Disk Full" conditions.

```
┌─────────────────┐      Volume >80%      ┌────────────────────┐      Reconcile      ┌──────────────────┐
│  Stellar Node   ├──────────────────────►│  Operator Detects  ├────────────────────►│  PVC Expanded    │
│  Data Volume    │                       │  Usage Threshold   │                     │  in K8s Spec     │
└─────────────────┘                       └────────────────────┘                     └──────────────────┘
```

---

## 2. Common Issues & Troubleshooting

### 2.1 PVC Expansion is Stuck in "Resizing"
**Symptoms:**
```
kubectl get pvc -n stellar
NAME               STATUS      VOLUME                                     CAPACITY   ACCESS MODES   STORAGECLASS   AGE
data-validator-0   Resizing    pvc-12345678-abcd-ef01-2345-6789abcdef01   100Gi      RWO            standard       2d
```
The status stays as `Resizing` for a long time, and the volume size does not increase.

**Root Causes:**
1. **StorageClass does not support volume expansion**: The underlying storage driver does not allow dynamic resizing.
2. **File system expansion requires Pod restart**: Some storage classes (e.g., Azure Disk, AWS EBS in older Kubernetes versions) require the pod to be restarted/mounted again to expand the filesystem.

**Solution:**
1. Verify if the StorageClass allows volume expansion:
   ```bash
   kubectl get storageclass standard -o jsonpath='{.allowVolumeExpansion}'
   # If false or empty, you must recreate the PVC with a StorageClass that has allowVolumeExpansion: true
   ```
2. If the StorageClass allows expansion, restart the pod to trigger filesystem resizing:
   ```bash
   kubectl delete pod validator-0 -n stellar
   ```

### 2.2 Volume Cannot Be Expanded Further (Provider Limit)
**Symptoms:**
Operator logs show errors similar to:
```
Error expanding volume: Volume size exceeds cloud provider maximum limit.
```

**Solution:**
1. Check the cloud provider limits (e.g., AWS EBS gp3 limit is 16 TiB).
2. If limits are reached, deploy database pruning or switch to a high-density archive node setup. Refer to [Archive Pruning](../archive-pruning.md).

---

## 3. Manual Disk Expansion (Emergency Fallback)

If the operator cannot expand the disk automatically:
1. Temporarily pause the operator reconciliation loops for the resource:
   ```bash
   kubectl annotate stellarnode my-validator stellar.org/skip-reconciliation="true" -n stellar
   ```
2. Manually patch the PVC to request a larger size:
   ```bash
   kubectl patch pvc data-validator-0 -n stellar \
     -p '{"spec":{"resources":{"requests":{"storage":"200Gi"}}}}'
   ```
3. Monitor status until capacity increases:
   ```bash
   kubectl get pvc data-validator-0 -n stellar -w
   ```
4. Remove the annotation to resume operator reconciliation:
   ```bash
   kubectl annotate stellarnode my-validator stellar.org/skip-reconciliation- -n stellar
   ```

---

## 4. Alerting Rules and Metrics
Monitor storage health using these metrics:
- `kubelet_volume_stats_used_bytes`
- `kubelet_volume_stats_capacity_bytes`

Recommended Prometheus alerts:
- `StellarVolumeFillingUp`: Fires when volume is predicted to run out of space in less than 24 hours.
- `StellarVolumeResizingFailed`: Fires when a volume remains in the `Resizing` state for more than 30 minutes.
