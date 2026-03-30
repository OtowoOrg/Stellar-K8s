# ADR-0003: Use of kube-rs Finalizers

## Status
Accepted

## Context
In Kubernetes, when a Custom Resource (CR) is deleted, the deletion can happen immediately without giving the operator a chance to clean up associated resources. This can lead to:

1. **Orphaned resources**: Deployments, Services, PVCs, etc. left behind after StellarNode deletion
2. **Resource leaks**: Persistent volumes not properly released
3. **Inconsistent state**: Partial cleanup leaving the cluster in an undefined state
4. **Security risks**: Credentials or configurations not properly cleaned up

The Kubernetes finalizer mechanism provides a way to prevent deletion until all cleanup is complete. When a finalizer is present on a resource:

- The resource cannot be permanently deleted from the API server
- The deletion timestamp is set, but the resource remains visible
- The controller can perform cleanup operations
- The controller removes the finalizer when cleanup is complete
- Kubernetes then permanently deletes the resource

## Decision
We chose to use the **kube-rs finalizer framework** for managing StellarNode lifecycle and cleanup.

### Implementation Approach

1. **Finalizer Registration**: Register `stellar.k8s.io/finalizer` on StellarNode resources
2. **Finalizer Pattern**: Use kube-rs's built-in finalizer handling with the `finalizer` function
3. **Cleanup Ordering**: Implement ordered cleanup to respect resource dependencies
4. **Error Handling**: Proper error handling and retry logic for cleanup failures

### Key Features

#### Automatic Cleanup Chain
```rust
finalizer(&api, STELLAR_NODE_FINALIZER, obj, |event| async {
    match event {
        FinalizerEvent::Apply(node) => apply_stellar_node(&client, &node, &ctx).await,
        FinalizerEvent::Cleanup(node) => cleanup_stellar_node(&client, &node, &ctx).await,
    }
}).await
```

#### Ordered Resource Deletion
1. **Managed Database Resources** (CNPG clusters, poolers)
2. **Monitoring Resources** (HPA, ServiceMonitor, Alerting)
3. **Network Resources** (Ingress, NetworkPolicy, Services)
4. **Workloads** (Deployments, StatefulSets)
5. **Configuration** (ConfigMaps, Secrets)
6. **Storage** (PVCs - based on retention policy)

#### Dry-Run Support
- Finalizer cleanup operations respect the `--dry-run` flag
- Cleanup operations use server-side dry-run when enabled
- Proper logging of what would be deleted in dry-run mode

## Consequences

### Positive Consequences
- **Complete cleanup**: No orphaned resources when StellarNodes are deleted
- **Enterprise reliability**: Proper resource management for production environments
- **Consistent state**: Cluster remains in a known, clean state after deletions
- **Debug visibility**: Clear logging of cleanup progress and failures
- **Dry-run safety**: Cleanup operations can be tested safely

### Negative Consequences
- **Deletion latency**: Resources remain visible during cleanup process
- **Complexity**: Additional code for managing finalizer lifecycle
- **Error scenarios**: Need to handle cleanup failures gracefully
- **Resource blocking**: Failed cleanup can prevent resource deletion

### Mitigations
- **Timeout handling**: Implement reasonable timeouts for cleanup operations
- **Retry logic**: Automatic retry for transient cleanup failures
- **Status updates**: Clear status indicators for cleanup progress
- **Manual intervention**: Procedures for handling stuck finalizers

## Implementation Details

### Finalizer String
```rust
const STELLAR_NODE_FINALIZER: &str = "stellar.k8s.io/finalizer";
```

### Cleanup Strategy
The cleanup function follows dependency order:
1. **External Dependencies**: Load balancers, external services
2. **Application Resources**: Ingress, Services, workloads
3. **Configuration**: ConfigMaps, Secrets
4. **Storage**: PVCs (respecting retention policies)

### Error Handling
- **Transient errors**: Automatic retry with exponential backoff
- **Permanent errors**: Log and continue with remaining cleanup
- **Timeout errors**: Force removal of finalizer after timeout
- **Status updates**: Update StellarNode status with cleanup progress

## Alternatives Considered

### Owner References
**Pros**: Native Kubernetes mechanism, automatic cleanup
**Cons**:
- Limited to same namespace
- Cannot handle cross-namespace resources
- Less granular control over cleanup order
- Cannot implement custom cleanup logic

### Custom Controller Logic
**Pros**: Full control over cleanup process
**Cons**:
- Reinventing finalizer pattern
- Race conditions with rapid deletions
- More complex state management
- No built-in retry mechanisms

### External Cleanup Jobs
**Pros**: Separation of concerns
**Cons**:
- Additional deployment complexity
- Synchronization challenges
- Delayed cleanup response
- Resource overhead

## Best Practices Implemented

1. **Idempotent Operations**: All cleanup operations can be safely retried
2. **Dependency Ordering**: Resources deleted in reverse dependency order
3. **Error Resilience**: Continue cleanup despite individual failures
4. **Observability**: Comprehensive logging and status updates
5. **Testing**: Dry-run mode for testing cleanup operations

## References

- [Kubernetes Finalizers Documentation](https://kubernetes.io/docs/concepts/overview/working-with-objects/finalizers/)
- [kube-rs Finalizer Pattern](https://docs.rs/kube/latest/kube/runtime/finalizer/index.html)
- [Operator Pattern Best Practices](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/)

## Future Considerations

- **Finalizer Timeouts**: Implement automatic finalizer removal after extended periods
- **Cleanup Metrics**: Track cleanup duration and success rates
- **Parallel Cleanup**: Optimize by cleaning independent resources in parallel
- **Cleanup Validation**: Verify resource deletion before proceeding
