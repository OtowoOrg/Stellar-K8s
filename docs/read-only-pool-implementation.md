# Read-Only Pool Implementation

## Overview

This document describes the implementation of the Read-Only Pool feature for horizontally scalable read-only Stellar nodes.

## Features Implemented

### 1. Separate CRD for Read-Only Replica Pools

- **CRD**: `ReadOnlyPool` (shortname: `rop`)
- **Location**: `src/crd/read_only_pool.rs`
- **CRD YAML**: `config/crd/readonlypool-crd.yaml`
- **Helm Chart**: `charts/stellar-operator/templates/readonlypool-crd.yaml`

The ReadOnlyPool CRD is separate from StellarNode and designed specifically for horizontally scalable read-only nodes.

### 2. Weighted Load Balancing

- **Configuration**: `spec.loadBalancing`
- **Features**:
  - Weight assignment based on node freshness (fresh vs lagging)
  - Configurable weights: `freshNodeWeight` (default: 100) and `laggingNodeWeight` (default: 10)
  - Configurable lag threshold: `lagThreshold` (default: 1000 ledger sequences)
  - Automatic weight recalculation at configurable intervals

**Implementation Details**:
- Located in `src/controller/read_only_pool.rs`
- Function: `calculate_load_balancing_weights()`
- Updates service endpoints with weights via `update_service_weights()`
- Tracks replica weights in status: `status.replicaWeights`

### 3. Automated Shard Balancing

- **Configuration**: `spec.shardBalancing`
- **Features**:
  - Multiple shard assignment strategies:
    - `RoundRobin`: Distributes replicas evenly across shards
    - `HashBased`: Uses consistent hashing for stable assignments
    - `Manual`: Allows manual assignment via annotations
  - Configurable shard count: `shardCount` (default: 4)
  - Automatic rebalancing when nodes are added/removed
  - Shard assignments stored in pod annotations

**Implementation Details**:
- Located in `src/controller/read_only_pool.rs`
- Function: `calculate_shard_assignments()`
- Updates pod annotations with shard information via `update_pod_shard_assignments()`
- Tracks shard assignments in status: `status.shardAssignments`

### 4. Auto-Scaling Controller

- **Controller**: `ReadOnlyPoolController`
- **Location**: `src/controller/read_only_pool.rs`
- **Features**:
  - Automatic scaling based on replica health
  - Monitors fresh vs lagging replicas
  - Scales up when too many replicas are lagging
  - Respects min/max replica constraints

**Reconciliation Flow**:
1. Validate spec
2. Ensure ConfigMap, Deployment, and Service exist
3. Check health of all pods
4. Calculate load balancing weights (if enabled)
5. Calculate shard assignments (if enabled)
6. Update service with weights
7. Update pod annotations with shard info
8. Auto-scale based on metrics
9. Update status

## Resource Management

### Kubernetes Resources Created

1. **ConfigMap**: Contains Stellar Core configuration
   - Location: `src/controller/read_only_pool_resources.rs::build_config_map()`

2. **Deployment**: Manages the pool of read-only nodes
   - Location: `src/controller/read_only_pool_resources.rs::build_deployment()`
   - Supports horizontal scaling
   - Uses ConfigMap for configuration
   - Mounts persistent storage

3. **Service**: Exposes the pool
   - Location: `src/controller/read_only_pool_resources.rs::build_service()`
   - Ports: 11625 (peer), 11626 (HTTP)

## Status Tracking

The ReadOnlyPool status includes:
- `currentReplicas`: Current number of replicas
- `readyReplicas`: Number of ready replicas
- `freshReplicas`: Number of fresh (up-to-date) replicas
- `laggingReplicas`: Number of lagging replicas
- `replicaWeights`: Weight information per replica
- `shardAssignments`: Shard assignment per replica
- `averageLedgerSequence`: Average ledger sequence across replicas
- `networkLatestLedger`: Latest ledger from the network
- `averageLag`: Average lag across all replicas

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
    retentionPolicy: Delete
```

## Integration

The ReadOnlyPool controller runs alongside the StellarNode controller in `src/main.rs`:
- Both controllers run concurrently using `tokio::select!`
- Shared Kubernetes client
- Independent reconciliation loops

## RBAC Permissions

Updated in `charts/stellar-operator/templates/rbac.yaml`:
- `readonlypools` CRD permissions
- `readonlypools/status` permissions
- `readonlypools/finalizers` permissions

## Known Issues

1. **Compilation Error**: There's a `schemars::gen` error that affects both `StellarNode` and `ReadOnlyPool` CRDs. This appears to be a version compatibility issue between `kube-rs` and `schemars`. This needs to be resolved by:
   - Updating dependency versions
   - Or using manual schema generation (`schema = "manual"`)

2. **Metrics Endpoint**: The current implementation uses pod annotations for ledger sequence tracking. In production, this should query the Stellar Core metrics endpoint directly.

3. **Service Weight Updates**: The `update_service_weights()` function is a placeholder. In production, this should:
   - Use EndpointSlice annotations for weight-based routing
   - Or integrate with a service mesh (Istio, Linkerd) for weighted routing

## Future Enhancements

1. **Metrics Integration**: Direct integration with Prometheus metrics
2. **Service Mesh Integration**: Native support for Istio/Linkerd weighted routing
3. **Advanced Sharding**: Ledger range-based sharding for very large archives
4. **Health-Based Scaling**: Scale based on request latency and error rates
5. **Geographic Distribution**: Support for multi-region deployments
