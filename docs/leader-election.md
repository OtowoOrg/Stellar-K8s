# Leader Election Implementation Guide

## Overview

The Stellar-K8s Operator implements high availability (HA) through leader election, enabling multiple operator replicas to run simultaneously with only one designated leader processing reconciliation at any given time. This ensures:

- **High Availability**: If the leader becomes unavailable, a follower automatically takes over within ~20 seconds
- **No Split-Brain**: Kubernetes-native Lease-based coordination ensures only one leader at a time
- **Zero Downtime**: Leader transitions happen without interrupting cluster operations
- **Automatic Failover**: No manual intervention required for replica failures

## Architecture

### Leader Election Mechanism

The operator uses Kubernetes `coordination.k8s.io/v1` Lease resources for leader election:

1. **Lease Creation**: Each operator replica attempts to create/renew a Lease named `stellar-operator-leader`
2. **Lease Holder**: The replica that successfully holds the Lease is the leader
3. **Renewal**: The leader continuously renews its lease every 10 seconds
4. **Expiry**: If a leader crashes, its lease expires in 15 seconds
5. **Takeover**: Any follower can then acquire the lease and become the new leader

### Key Components

```rust
// In src/commands/operator.rs

const LEASE_NAME: &str = "stellar-operator-leader";
const LEASE_DURATION_SECS: i32 = 15;      // Lease validity period
const RENEW_INTERVAL: Duration = Duration::from_secs(10);  // Renewal frequency
const RETRY_INTERVAL: Duration = Duration::from_secs(5);   // Retry on failure

// Each replica maintains this atomic flag
let is_leader = Arc::new(AtomicBool::new(false));

// Background task manages leader election
tokio::spawn(async move {
    run_leader_election(lease_client, &lease_ns, &identity, is_leader).await;
});
```

### Replica Identity

Each operator replica is identified by:

- **POD_NAMESPACE**: Kubernetes namespace where the operator runs
- **HOSTNAME**: Pod name (automatically set from metadata.name in Kubernetes)

These are automatically injected by the deployment manifest.

## Deployment Configuration

### Single Replica (Default)

```yaml
replicaCount: 1
```

Only one operator instance runs. Leader election still works but is unused.

### High Availability Setup (Recommended)

```yaml
replicaCount: 3

podDisruptionBudget:
    enabled: true
    minAvailable: 1
```

Deploy 3 operator replicas with:

- Pod anti-affinity spreading replicas across different nodes
- Pod Disruption Budget ensuring ≥1 replica stays available during maintenance
- Automatic leader election coordinating which replica processes reconciliation

## Installation

### Install with Default (Single Replica)

```bash
helm install stellar-operator ./charts/stellar-operator
```

### Install with High Availability (3 Replicas)

```bash
helm install stellar-operator ./charts/stellar-operator \
  --set replicaCount=3 \
  --set podDisruptionBudget.enabled=true
```

Or use the provided HA values file:

```bash
helm install stellar-operator ./charts/stellar-operator \
  -f ./charts/stellar-operator/values-ha.yaml
```

### Upgrade Existing Single-Replica Deployment to HA

```bash
helm upgrade stellar-operator ./charts/stellar-operator \
  --set replicaCount=3 \
  --set podDisruptionBudget.enabled=true
```

## Monitoring Leader Election

### Check Current Leader

```bash
# View the Lease resource
kubectl get lease -n stellar-operator stellar-operator-leader -o yaml

# Shows:
# spec:
#   holderIdentity: stellar-operator-pod-1
#   acquireTime: 2024-05-31T10:15:23Z
#   renewTime: 2024-05-31T10:15:33Z
#   leaseDurationSeconds: 15
```

### View Operator Status

```bash
# Check if an operator pod is the leader
kubectl logs -n stellar-operator stellar-operator-pod-1 | grep "Acquired leadership"

# Non-leader output:
# kubectl logs -n stellar-operator stellar-operator-pod-2
# [2024-05-31T10:15:25Z] Lost leadership (will retry)
```

### Metrics

With metrics enabled, the operator exposes:

```
# HELP stellar_operator_leader_status Whether the operator is the current leader
# TYPE stellar_operator_leader_status gauge
stellar_operator_leader_status{instance="stellar-operator-pod-1"} 1
stellar_operator_leader_status{instance="stellar-operator-pod-2"} 0
stellar_operator_leader_status{instance="stellar-operator-pod-3"} 0

# HELP stellar_operator_uptime_seconds Operator uptime in seconds
# TYPE stellar_operator_uptime_seconds counter
stellar_operator_uptime_seconds{instance="stellar-operator-pod-1"} 3600
```

## Failover Scenarios

### Scenario 1: Leader Pod Crashes

Timeline:

- **T=0s**: Leader crashes, Pod enters CrashLoopBackOff
- **T=5s**: Followers notice no lease renewal
- **T=15s**: Lease expires
- **T=16s**: A follower acquires the lease and becomes leader
- **T=20s**: New leader starts processing reconciliation

### Scenario 2: Leader Pod Becomes Unresponsive (Network Partition)

Timeline:

- **T=0s**: Network partition isolates leader
- **T=10s**: Leader fails to renew lease due to API server unreachability
- **T=15s**: Lease expires
- **T=16s**: A responding follower acquires the lease
- **T=20s**: New leader resumes reconciliation

### Scenario 3: Planned Maintenance (Node Drain)

Timeline:

- **T=0s**: Cluster admin runs `kubectl drain node-1` (where leader runs)
- **T=10s**: Kubernetes eviction grace period begins
- **T=30s**: Pod is forcefully terminated
- **T=31s**: A follower on another node acquires the lease
- **T=35s**: New leader starts processing reconciliation

With Pod Disruption Budget enabled:

- Drain process respects `minAvailable: 1` constraint
- At least one operator pod stays available

## Behavior of Different Replica Types

### Leader Pod

**Responsibilities**:

- Processes reconciliation for StellarNode and other CRDs
- Updates status subresources
- Creates/updates Kubernetes resources (Deployments, Services, etc.)
- Logs reconciliation events

**Monitoring**:

- Serves metrics on `:9090/metrics`
- Serves health checks on `:9090/healthz` (always returns 200)
- Logs show `Acquired leadership: stellar-operator-leader`

### Follower Pod

**Responsibilities**:

- Remains ready to take over on leader failure
- Serves metrics and health checks (same as leader)
- Responds to liveness/readiness probes
- Does NOT process reconciliation (skips early if not leader)

**Benefits**:

- Still observable and monitorable
- Still counted in liveness/readiness checks
- Can be used for leader failover detection via metrics

## Environment Variables

The following environment variables control leader election:

| Variable        | Default                      | Purpose                       |
| --------------- | ---------------------------- | ----------------------------- |
| `POD_NAMESPACE` | `STELLAR_OPERATOR_NAMESPACE` | Lease namespace               |
| `HOSTNAME`      | System hostname              | Pod identity for lease holder |

These are automatically set by the Kubernetes downward API in the deployment manifest.

## RBAC Requirements

The operator ServiceAccount must have permissions for Leases:

```yaml
- apiGroups: ["coordination.k8s.io"]
  resources: ["leases"]
  verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
```

This is pre-configured in the Helm chart's RBAC template.

## Testing

### Unit Tests

Run the leader election test suite:

```bash
cargo test --test leader_election_test
```

Tests cover:

- Single leader election invariant
- Concurrent leadership transitions
- Lease renewal mechanics
- Failover scenarios
- Scaling with multiple replicas

### Integration Test

Deploy to a Kubernetes cluster and verify:

```bash
# Deploy 3 replicas
helm upgrade stellar-operator ./charts/stellar-operator \
  --set replicaCount=3

# Wait for deployment to settle
kubectl rollout status deployment/stellar-operator -n stellar-operator

# Find the leader
LEADER=$(kubectl get lease -n stellar-operator stellar-operator-leader -o jsonpath='{.spec.holderIdentity}')
echo "Current leader: $LEADER"

# Terminate the leader pod
kubectl delete pod $LEADER -n stellar-operator

# Watch leadership transfer (should complete within 20s)
watch kubectl get lease -n stellar-operator stellar-operator-leader -o yaml

# Verify reconciliation continues
kubectl logs -f deployment/stellar-operator -n stellar-operator | grep "reconciling"
```

## Troubleshooting

### Problem: Multiple Leaders Detected

**Symptom**: Lease shows multiple pods with leader status
**Cause**: Usually indicates a serious clock skew or API server issue
**Solution**:

1. Check NTP sync on all nodes: `timedatectl status`
2. Verify API server health: `kubectl get componentstatuses`
3. Check operator logs for API errors

### Problem: No Leader Election Activity

**Symptom**: Lease is never created or updated
**Cause**: Missing RBAC permissions
**Solution**: Verify operator ServiceAccount has Lease permissions

```bash
kubectl auth can-i patch leases --as=system:serviceaccount:stellar-operator:stellar-operator
```

### Problem: Leadership Flapping

**Symptom**: Leader changes every few seconds
**Cause**: Usually high latency or network issues
**Solution**:

1. Check network connectivity between API server and operator pods
2. Review operator logs for timeout messages
3. Increase `LEASE_DURATION_SECS` in code if needed (currently 15s)

### Problem: Long Failover Time

**Symptom**: Leader failure takes >30 seconds to trigger failover
**Cause**: Follower pods may be slow to renew attempts
**Solution**: Check Pod resource limits and node capacity

## Performance Considerations

### Overhead

- **Leader**: Minimal overhead from lease renewal (10s interval)
- **Followers**: Minimal overhead from lease acquisition attempts (5s interval)
- **Lease creation**: ~10-50ms per operation

### Scalability

- Tested with 3-10 replicas
- No significant performance degradation with more replicas
- Lease coordination is O(1) regardless of replica count

### Lease Timing

Current configuration:

- **Lease Duration**: 15 seconds
- **Renewal Interval**: 10 seconds
- **Retry Interval**: 5 seconds

This means:

- Maximum time from leader crash to follower takeover: ~20 seconds
- Time to detect leader failure: ~15 seconds

## Migration Guide

### From Single Replica to HA

1. **Phase 1: Deploy Additional Replicas** (no downtime)

    ```bash
    helm upgrade stellar-operator ./charts/stellar-operator \
      --set replicaCount=3
    ```

    - New replicas start as followers
    - Existing leader continues processing
    - Old leader automatically becomes a follower

2. **Phase 2: Verify** (5 minutes)
    - Check all pods are Running
    - Verify only one leader in Lease resource
    - Monitor logs for reconciliation activity

3. **Phase 3: Enable PDB** (optional)
    ```bash
    helm upgrade stellar-operator ./charts/stellar-operator \
      --set replicaCount=3 \
      --set podDisruptionBudget.enabled=true
    ```

### From HA Back to Single Replica

1. Scale down to 1 replica:

    ```bash
    helm upgrade stellar-operator ./charts/stellar-operator \
      --set replicaCount=1
    ```

    - Kubernetes will terminate follower pods
    - If leader is terminated, the remaining pod becomes leader
    - No data loss (state is in etcd)

## Advanced Configuration

### Custom Lease Duration

To adjust lease timing, edit `src/commands/operator.rs`:

```rust
const LEASE_DURATION_SECS: i32 = 30;    // Increase for higher latency networks
const RENEW_INTERVAL: Duration = Duration::from_secs(15);
const RETRY_INTERVAL: Duration = Duration::from_secs(5);
```

Then rebuild the operator image.

### Custom Leader Identity

The operator automatically uses the pod name (HOSTNAME) as the leader identity. To use a custom identity, set the HOSTNAME environment variable:

```yaml
env:
    - name: HOSTNAME
      value: "custom-operator-identity"
```

## Related Resources

- [Kubernetes Lease API Documentation](https://kubernetes.io/docs/reference/kubernetes-api/cluster-resources/lease-v1/)
- [Operator Pattern](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/)
- [Leader Election Implementations](https://github.com/kubernetes-sigs/controller-runtime/tree/main/pkg/leaderelection)
