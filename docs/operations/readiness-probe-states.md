# Readiness Probe State Machine Coverage

## Overview

The Stellar-K8s operator implements comprehensive readiness probe state handling for validator nodes to ensure only fully operational nodes receive production traffic. This document describes the state machine coverage, accepted states, and operational implications.

## Problem Statement

Prior to this implementation, the readiness probe only rejected `CATCHING_UP` and `SYNCING` states, but did not handle all possible stellar-core states comprehensively. This led to scenarios where nodes in transitional states (e.g., `JOINING_SCP`, `BOOTING_UP`, `DISCONNECTED`) could be marked ready and receive traffic before they were capable of participating in consensus.

## Stellar Core States

Stellar-core reports various states through its `/info` HTTP endpoint. The readiness probe now handles all these states explicitly:

### Ready States (Pod Accepts Traffic)

| State | Description | Impact |
|-------|-------------|--------|
| `Synced!` | Fully synced with the network | **Ready** - Node can participate in consensus |
| `Tracking!` | Actively tracking consensus | **Ready** - Node is operational (rare but valid) |

### Not-Ready States (Pod Removed from Service)

| State | Description | Impact |
|-------|-------------|--------|
| `Booting Up` | Initial startup phase | **Not Ready** - Not yet connected to peers |
| `Joining SCP` | Attempting to join consensus | **Not Ready** - Not yet synced |
| `Connected` | Connected to peers | **Not Ready** - Not yet synced |
| `Catching up` | Syncing historical ledgers | **Not Ready** - Compute-intensive, not consensus-ready |
| `Syncing` | Similar to catching up | **Not Ready** - Not fully synced |
| `Stopping` | Graceful shutdown | **Not Ready** - Node shutting down |
| `Disconnected` | Lost connectivity to quorum | **Not Ready** - Cannot participate in consensus |

## Implementation

### Readiness Probe Script

The validator readiness probe uses an exec probe that queries the stellar-core HTTP API:

```bash
RESP=$(wget -qO- http://localhost:11626/info 2>/dev/null) && \
STATE=$(echo "$RESP" | grep -o '"state"[[:space:]]*:[[:space:]]*"[^"]*"' | \
sed 's/.*"\([^"]*\)"/\1/') && \
case "$STATE" in \
  'Synced!'|'Tracking!') exit 0 ;; \
  *) exit 1 ;; \
esac
```

### Probe Configuration

- **Initial Delay**: 15 seconds
- **Period**: 10 seconds (checks every 10 seconds)
- **Timeout**: 5 seconds per check
- **Failure Threshold**: 3 consecutive failures mark pod not ready
- **Success Threshold**: 1 success marks pod ready

## Operational Impact

### Traffic Routing

- **Ready Pods**: Only pods in `Synced!` or `Tracking!` state receive traffic through Kubernetes Services
- **Not-Ready Pods**: Automatically removed from Service endpoints but remain running

### Separation of Concerns

The readiness probe is intentionally separate from the liveness probe:

- **Liveness Probe**: TCP socket check on port 11625 (peer port)
  - Ensures the stellar-core process is running
  - Restarts the pod if the process is dead
  
- **Readiness Probe**: State machine check via HTTP API
  - Ensures the node is fully operational
  - Removes from traffic without restarting

This separation ensures that a syncing node is never restarted — it's only temporarily removed from the ready set.

## Testing

Unit tests verify all state transitions:

```rust
#[test]
fn test_readiness_probe_accepts_synced_state() { ... }

#[test]
fn test_readiness_probe_accepts_tracking_state() { ... }

#[test]
fn test_readiness_probe_rejects_catching_up_state() { ... }
```

Run tests with:
```bash
cargo test -p stellar-k8s readiness_probe
```

## Monitoring

### Observing Readiness State

Check pod readiness status:
```bash
kubectl get pods -l app.kubernetes.io/component=validator
```

View readiness probe failures:
```bash
kubectl describe pod <pod-name> | grep -A 10 "Readiness"
```

Check stellar-core state directly:
```bash
kubectl exec <pod-name> -- wget -qO- http://localhost:11626/info | jq -r .info.state
```

### Metrics

The operator exposes metrics for readiness probe failures:
- `stellar_node_readiness_probe_failures_total`
- `stellar_node_state_transitions_total`

## Troubleshooting

### Node Stuck in CATCHING_UP

**Symptom**: Pod remains not ready for extended period

**Diagnosis**:
```bash
kubectl exec <pod-name> -- wget -qO- http://localhost:11626/info | jq .
```

**Common Causes**:
1. Slow history archive download
2. Insufficient resources (CPU/memory)
3. Network connectivity issues
4. Database performance bottleneck

**Resolution**:
- Increase resources via `spec.resources`
- Enable sync-state scaling: `spec.syncStateScaling.enabled: true`
- Bootstrap from snapshot: `spec.storage.snapshotRef`

### Node Flapping Between Ready/Not Ready

**Symptom**: Pod rapidly transitions between ready and not ready

**Diagnosis**:
```bash
kubectl logs <pod-name> | grep -i "state change"
```

**Common Causes**:
1. Network instability to quorum peers
2. Overloaded node (high CPU/memory usage)
3. Database connection issues

**Resolution**:
- Check network policies and connectivity
- Scale up resources
- Review quorum set configuration

### Node Stuck in JOINING_SCP

**Symptom**: Node cannot join consensus after startup

**Diagnosis**:
```bash
kubectl exec <pod-name> -- wget -qO- http://localhost:11626/info | jq '.info | {state, num_peers, quorum}'
```

**Common Causes**:
1. Invalid quorum set configuration
2. Cannot reach quorum peers
3. Incorrect network passphrase

**Resolution**:
- Verify `spec.validatorConfig.quorumSet`
- Check network policies allow peer connectivity
- Confirm `spec.network` matches peers

## Best Practices

1. **Gradual Rollouts**: Use `PodDisruptionBudget` with `minAvailable` to ensure sufficient ready replicas during updates

2. **Resource Allocation**: Ensure validators have sufficient resources to avoid getting stuck in CATCHING_UP
   ```yaml
   spec:
     resources:
       requests:
         cpu: "4"
         memory: "8Gi"
   ```

3. **Snapshot Bootstrapping**: Use volume snapshots to reduce initial sync time
   ```yaml
   spec:
     storage:
       snapshotRef:
         volumeSnapshotName: validator-snapshot-20240101
   ```

4. **Monitoring**: Set up alerts for prolonged not-ready states
   ```yaml
   expr: kube_pod_status_ready{condition="false"} > 300
   annotations:
     summary: "Pod {{ $labels.pod }} not ready for 5+ minutes"
   ```

## Related Documentation

- [Health Checks](../health-checks.md)
- [Sync State Scaling](../sync-state-scaling.md)
- [Volume Snapshots](../volume-snapshots.md)
- [Performance Tuning](../performance-tuning.md)

## References

- [Stellar Core State Machine](https://github.com/stellar/stellar-core/blob/master/src/main/ApplicationUtils.cpp)
- [Kubernetes Probe Configuration](https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/)
- [Issue #1559: Readiness Probe State Machine Coverage](https://github.com/OtowoOrg/Stellar-K8s/issues/1559)
