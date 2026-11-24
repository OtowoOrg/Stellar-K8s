# [EPIC] Label-Based Index Sharding for CRD Informer Caches

**Labels:** `epic`, `200-points`, `scalability`, `performance`, `phase-3`

## Epic Overview

Define and manage label-based index sharding for CRD informer caches so large collections scale without unbounded memory growth. This epic enables Stellar to operate efficiently at scale with 500k+ CRD objects while maintaining sub-second lookup latency and predictable memory consumption.

## Business Value

- **Unbounded scalability**: Support 10x+ growth without memory concerns
- **Predictable costs**: Memory usage stays bounded regardless of collection size
- **Performance**: Maintain flat lookup latency as collections grow
- **Reliability**: No watch disconnects during rebalancing operations

## Scope & Requirements

### Core Requirements

1. **Consistent Hashing Strategy**
   - Implement consistent hashing on designated shard key
   - Ensure O(1/N) key movement during rebalancing (not entire collection)
   - Minimize cascade effects when nodes added/removed

2. **Shard Management**
   - Dynamic shard allocation based on collection size
   - Automatic shard rebalancing without watch disconnects
   - Support for custom shard key selection per CRD type

3. **Informer Cache Sharding**
   - Partition `StellarNode`, `StellarCluster`, `StellarValidator` informers
   - Implement per-shard in-memory indexes
   - Query routing logic to find correct shard

4. **Status Visibility**
   - Publish shard strategy and allocation in CRD status
   - Metrics for shard distribution and utilization
   - Debugging tools for shard inspection

5. **Backward Compatibility**
   - Non-sharded mode for backward compatibility
   - Automatic migration from non-sharded to sharded
   - Graceful downgrade capability

### Non-Functional Requirements

- **Memory budget**: Cache stays under 500GB at 500k objects (1MB per object)
- **Rebalance impact**: Zero watch disconnects during rebalancing
- **Lookup latency**: Stays flat (< 5% variance) as collection grows 10x
- **Shard count**: Support 10-1000 shards dynamically
- **Rebalance time**: Complete within 30 seconds for 500k objects

## Technical Design

### Architecture Components

```
┌─────────────────────────────────────────────────────────────┐
│              Label-Based Index Sharding Layer                │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐            │
│  │  Shard 0   │  │  Shard 1   │  │  Shard N   │            │
│  │ (Labels:   │  │ (Labels:   │  │ (Labels:   │            │
│  │  region=us)│  │  region=eu)│  │  region=ap)│            │
│  └────────────┘  └────────────┘  └────────────┘            │
└─────────────────────────────────────────────────────────────┘
         │                    │                    │
    ┌────┴────┐          ┌────┴────┐          ┌────┴────┐
    │Informer │          │Informer │          │Informer │
    │ Cache 0 │          │ Cache 1 │          │ Cache N │
    │(500MB)  │          │(500MB)  │          │(500MB)  │
    └─────────┘          └─────────┘          └─────────┘
```

### Implementation Approach

1. **Shard Key Mapping**
   ```yaml
   apiVersion: stellar.org/v1alpha1
   kind: ShardingPolicy
   metadata:
     name: default-sharding
   spec:
     shardConfigs:
       - crds: ["StellarNode", "StellarCluster"]
         shardKey: "metadata.labels.region"
         shardCount: 16
         hashAlgorithm: "consistent-hash"
         rebalanceThreshold: 0.2  # 20% imbalance triggers rebalance
   ```

2. **Consistent Hash Implementation**
   - Use jump hash or ring hash for O(1) shard lookup
   - Virtual nodes (100-1000 per physical shard) for even distribution
   - Track shard membership changes separately from data

3. **Shard Controller**
   - Watch for collection size changes
   - Adjust shard count automatically (every 5-10k object threshold)
   - Coordinate rebalancing without informer disconnects
   - Publish shard allocation to status

4. **Query Router**
   - Determine shard from object labels
   - Execute queries on appropriate shard(s)
   - Combine results for cross-shard queries
   - Cache query routing decisions

5. **Rebalancing Algorithm**
   - Compute new shard distribution
   - Generate migration plan (move only affected keys)
   - Execute migration in batches to avoid spike
   - Verify consistency after rebalance

6. **Status & Observability**
   ```yaml
   status:
     sharding:
       strategy: label-based
       shardKey: "metadata.labels.region"
       shards: 16
       distribution:
         - shardID: 0
           objectCount: 31250
           memoryUsage: 512Mi
           healthStatus: healthy
         - shardID: 1
           objectCount: 31250
           memoryUsage: 512Mi
           healthStatus: healthy
       lastRebalance: 2024-11-20T10:30:00Z
       nextRebalanceEstimate: 2024-11-27T10:30:00Z
   ```

## Acceptance Criteria

- [ ] Consistent hashing implementation passes correctness tests
- [ ] Cache memory stays under 500GB budget at 500k objects
- [ ] Rebalancing completes without any watch disconnects
- [ ] Lookup latency variance < 5% as collection grows 10x (100k → 1M objects)
- [ ] Shard strategy visible in CRD status (ShardingPolicy status)
- [ ] Performance benchmarks comparing sharded vs non-sharded modes
- [ ] Automatic shard count adjustment based on collection size
- [ ] Zero data loss during rebalancing operations
- [ ] Graceful degradation if shard fails (queries route to replicas)
- [ ] kubectl commands to inspect shard distribution and utilization
- [ ] E2E tests simulating collection growth and rebalancing
- [ ] Documentation on shard key selection and tuning

## Dependencies & Blockers

- Requires informer infrastructure to support shard-aware queries
- Depends on label-based object classification
- May require etcd performance tuning for high shard counts
- Needs persistent shard state (ConfigMap or CRD)

## Testing Strategy

### Unit Tests
- Consistent hash algorithm correctness
- Shard key extraction and validation
- Query routing logic

### Integration Tests
- Multi-shard cache coherence
- Rebalancing during active queries
- Shard failure recovery

### E2E Tests
- Grow collection from 1k to 500k objects
- Monitor memory and latency throughout
- Trigger rebalances at various collection sizes
- Chaos: kill shards and verify recovery
- Cross-shard queries and aggregations

### Performance Benchmarks
- Baseline latency per shard size (1k, 10k, 100k, 1M)
- Latency comparison: sharded vs non-sharded
- Memory overhead of sharding infrastructure
- Rebalance throughput (objects/sec migrated)

## Estimated Effort

**200 Story Points** (~4-6 weeks for 2 engineers)

## Related Issues

- #TBD: Memory profiling and optimization
- #TBD: Query performance optimization
- #TBD: Etcd performance at scale

## References

- [Consistent Hashing](https://en.wikipedia.org/wiki/Consistent_hashing)
- [Jump Hash Algorithm](https://arxiv.org/pdf/1406.2294.pdf)
- [Kubernetes Informer Pattern](https://pkg.go.dev/k8s.io/client-go/tools/cache)
- [Cache-Oblivious B-Trees](https://en.wikipedia.org/wiki/Cache-oblivious_algorithm)

