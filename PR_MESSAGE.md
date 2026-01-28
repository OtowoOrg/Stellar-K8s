# Add Read-Only Pool Auto-Scaling with Weighted Load Balancing and Shard Balancing

## Branch Name
```
feature/read-only-pool-autoscaling
```

## Summary

Implements a separate controller and CRD for managing horizontally scalable pools of read-only Stellar nodes with intelligent load balancing and shard distribution capabilities.

## Motivation

While validators are sensitive and must remain read-only with a single replica, read-only nodes can be scaled horizontally to handle increased load. This feature enables:

- **Horizontal Scaling**: Automatically scale read-only node pools based on demand
- **Intelligent Load Balancing**: Route more traffic to fresh (up-to-date) nodes and less to lagging nodes
- **Shard Balancing**: Distribute large history archives across multiple shards for parallel processing

## Changes

### New CRD: `ReadOnlyPool`

- Separate CRD from `StellarNode` specifically designed for read-only replica pools
- Supports min/max/target replica configuration
- Configurable load balancing and shard balancing strategies

### Weighted Load Balancing

- Automatically assigns weights to replicas based on ledger lag
- Fresh nodes (within threshold) receive higher weight (default: 100)
- Lagging nodes receive lower weight (default: 10)
- Configurable lag threshold and update intervals
- Weight information tracked in status and applied to service endpoints

### Automated Shard Balancing

- Three shard assignment strategies:
  - **RoundRobin**: Even distribution across shards
  - **HashBased**: Consistent hashing for stable assignments
  - **Manual**: Manual assignment via annotations
- Automatic rebalancing when nodes are added/removed
- Shard assignments stored in pod annotations for configuration

### Auto-Scaling Controller

- New `ReadOnlyPoolController` runs alongside `StellarNodeController`
- Monitors replica health and ledger synchronization status
- Scales up when too many replicas are lagging
- Respects min/max replica constraints

## Implementation Details

### Files Added

- `src/crd/read_only_pool.rs` - ReadOnlyPool CRD definition
- `src/controller/read_only_pool.rs` - Controller reconciliation logic
- `src/controller/read_only_pool_resources.rs` - Kubernetes resource builders
- `config/crd/readonlypool-crd.yaml` - CRD YAML definition
- `charts/stellar-operator/templates/readonlypool-crd.yaml` - Helm chart CRD
- `docs/read-only-pool-implementation.md` - Feature documentation

### Files Modified

- `src/crd/mod.rs` - Export ReadOnlyPool types
- `src/controller/mod.rs` - Add ReadOnlyPool controller module
- `src/main.rs` - Integrate ReadOnlyPool controller (runs concurrently)
- `charts/stellar-operator/templates/rbac.yaml` - Add RBAC permissions

## Example Usage

```yaml
apiVersion: stellar.org/v1alpha1
kind: ReadOnlyPool
metadata:
  name: mainnet-readonly-pool
  namespace: stellar-nodes
spec:
  network: Mainnet
  version: "v21.0.0"
  minReplicas: 3
  maxReplicas: 20
  targetReplicas: 5
  loadBalancing:
    enabled: true
    freshNodeWeight: 100
    laggingNodeWeight: 10
    lagThreshold: 1000
    updateIntervalSeconds: 30
  shardBalancing:
    enabled: true
    shardCount: 4
    strategy: RoundRobin
    autoRebalance: true
  historyArchiveUrls:
    - "https://archive1.example.com"
    - "https://archive2.example.com"
  resources:
    requests:
      cpu: "500m"
      memory: "1Gi"
    limits:
      cpu: "2"
      memory: "4Gi"
  storage:
    storageClass: "standard"
    size: "100Gi"
```

## Testing

- [ ] Unit tests for load balancing weight calculation
- [ ] Unit tests for shard assignment strategies
- [ ] Integration tests for controller reconciliation
- [ ] Manual testing in Kubernetes cluster
- [ ] Verify auto-scaling behavior under load
- [ ] Verify weighted load balancing routes traffic correctly
- [ ] Verify shard assignments are applied to pods

## Acceptance Criteria

✅ Separate Spec for Read-Only replica pools  
✅ Weighted load-balancing between fresh nodes and lagging nodes  
✅ Automated shard-balancing for very large history archives  

## Known Issues

1. **Compilation Error**: There's a `schemars::gen` error affecting both `StellarNode` and `ReadOnlyPool` CRDs. This appears to be a version compatibility issue between `kube-rs` (0.94) and `schemars`. This needs to be resolved before merging.

2. **Service Weight Updates**: The `update_service_weights()` function is currently a placeholder. In production, this should integrate with:
   - EndpointSlice annotations for weight-based routing, or
   - Service mesh (Istio/Linkerd) for weighted traffic distribution

3. **Metrics Integration**: Currently uses pod annotations for ledger sequence tracking. Should be enhanced to query Stellar Core metrics endpoints directly.

## Future Enhancements

- Direct Prometheus metrics integration
- Service mesh integration (Istio/Linkerd) for weighted routing
- Advanced sharding with ledger range-based distribution
- Health-based scaling using request latency and error rates
- Multi-region deployment support

## Checklist

- [x] Code follows project style guidelines
- [x] Self-review completed
- [x] Comments added for complex logic
- [x] Documentation updated
- [x] CRD YAML files added
- [x] Helm chart updated
- [x] RBAC permissions added
- [ ] Tests added/updated
- [ ] All tests pass
- [ ] No new warnings introduced (except known dependency issue)

## Related Issues

Closes #[issue-number]

## Screenshots/Demo

_Add screenshots or demo links if applicable_
