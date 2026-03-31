# Resource Limits for Stellar Node Types

This document provides recommended CPU and memory resource limits for different Stellar node types managed by the Stellar-K8s operator.

## Overview

The operator supports three node types, each with different resource requirements:

- **Validator**: Full consensus node running Stellar Core
- **Horizon**: REST API server for ledger queries
- **Soroban RPC**: Smart contract interaction node

Resource requirements vary based on network (Mainnet vs Testnet), workload, and operational requirements.

## Recommended Resource Limits

### Validator Nodes

Validator nodes participate in consensus and require consistent, predictable resources.

#### Production (Mainnet)

```yaml
spec:
  nodeType: Validator
  resources:
    requests:
      cpu: "2"
      memory: "4Gi"
    limits:
      cpu: "4"
      memory: "8Gi"
```

**Rationale:**

- CPU: Validators need consistent CPU for consensus participation and transaction validation
- Memory: Ledger state and in-memory operations require 4-8Gi
- Disk I/O: High IOPS required for ledger writes (use SSD-backed storage)

#### Development/Testnet

```yaml
spec:
  nodeType: Validator
  resources:
    requests:
      cpu: "500m"
      memory: "1Gi"
    limits:
      cpu: "2"
      memory: "4Gi"
```

**Rationale:**

- Lower transaction volume allows reduced resource allocation
- Suitable for testing and development environments

### Horizon Nodes

Horizon nodes serve API requests and can be horizontally scaled.

#### Production (Mainnet)

```yaml
spec:
  nodeType: Horizon
  resources:
    requests:
      cpu: "1"
      memory: "2Gi"
    limits:
      cpu: "4"
      memory: "8Gi"
```

**Rationale:**

- CPU: API request handling and database queries
- Memory: Query result caching and connection pooling
- Scalability: Use HPA (Horizontal Pod Autoscaler) for traffic spikes

#### Development/Testnet

```yaml
spec:
  nodeType: Horizon
  resources:
    requests:
      cpu: "250m"
      memory: "512Mi"
    limits:
      cpu: "2"
      memory: "4Gi"
```

**Rationale:**

- Lower request volume in test environments
- Minimal resource footprint for development

### Soroban RPC Nodes

Soroban RPC nodes handle smart contract simulation and submission.

#### Production (Mainnet)

```yaml
spec:
  nodeType: SorobanRpc
  resources:
    requests:
      cpu: "2"
      memory: "4Gi"
    limits:
      cpu: "8"
      memory: "16Gi"
```

**Rationale:**

- CPU: Contract execution and Wasm runtime require significant compute
- Memory: Captive Core instance + contract state + VM memory
- Higher limits accommodate complex contract simulations

#### Development/Testnet

```yaml
spec:
  nodeType: SorobanRpc
  resources:
    requests:
      cpu: "500m"
      memory: "2Gi"
    limits:
      cpu: "4"
      memory: "8Gi"
```

**Rationale:**

- Reduced contract complexity in test environments
- Suitable for development and testing

## Resource Precedence

The operator applies resources in the following order of precedence:

1. **StellarNode spec.resources** (highest priority)
2. **Helm chart defaultResources** (from values.yaml)
3. **Hardcoded operator defaults** (fallback)

### Example: Custom Resources

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: my-validator
spec:
  nodeType: Validator
  network: Mainnet
  resources:
    requests:
      cpu: "3"
      memory: "6Gi"
    limits:
      cpu: "6"
      memory: "12Gi"
```

## Storage Requirements

### Validator

- **Mainnet**: 500Gi - 1Ti (growing ~50Gi/year)
- **Testnet**: 100Gi - 200Gi
- **Storage Class**: SSD-backed (high IOPS required)

### Horizon

- **Mainnet**: 1Ti - 2Ti (full history)
- **Testnet**: 200Gi - 500Gi
- **Storage Class**: SSD-backed (database performance)

### Soroban RPC

- **Mainnet**: 500Gi - 1Ti (Captive Core + contract state)
- **Testnet**: 100Gi - 200Gi
- **Storage Class**: SSD-backed (Wasm execution performance)

## Autoscaling Recommendations

### Horizon (Recommended)

```yaml
spec:
  nodeType: Horizon
  autoscaling:
    minReplicas: 2
    maxReplicas: 10
    targetCPUUtilizationPercentage: 70
```

### Soroban RPC (Optional)

```yaml
spec:
  nodeType: SorobanRpc
  autoscaling:
    minReplicas: 2
    maxReplicas: 6
    targetCPUUtilizationPercentage: 75
```

### Validator (Not Recommended)

Validators should NOT use autoscaling as they:

- Require stable identity for consensus
- Use StatefulSets (not Deployments)
- Need consistent peer connections

## Monitoring and Tuning

### Key Metrics

Monitor these metrics to tune resource limits:

1. **CPU Utilization**
   - Target: 60-80% average utilization
   - Alert: >90% sustained for 5+ minutes

2. **Memory Usage**
   - Target: 70-85% of limit
   - Alert: >95% or OOMKilled events

3. **Disk I/O**
   - Target: <80% IOPS capacity
   - Alert: I/O wait >20%

### Prometheus Queries

```promql
# CPU utilization by node
rate(container_cpu_usage_seconds_total{pod=~"stellar-.*"}[5m])

# Memory usage by node
container_memory_working_set_bytes{pod=~"stellar-.*"}

# Disk I/O wait
rate(node_disk_io_time_seconds_total[5m])
```

## Performance Tuning

### CPU Optimization

1. **Validator**: Set CPU requests = limits (guaranteed QoS)
2. **Horizon**: Allow burstable CPU for traffic spikes
3. **Soroban**: Higher limits for complex contract execution

### Memory Optimization

1. **Validator**: Set memory requests = limits (avoid OOM)
2. **Horizon**: Configure connection pool sizes based on memory
3. **Soroban**: Allocate extra memory for Wasm VM overhead

### Example: Guaranteed QoS for Validator

```yaml
spec:
  nodeType: Validator
  resources:
    requests:
      cpu: "4"
      memory: "8Gi"
    limits:
      cpu: "4" # Same as requests = Guaranteed QoS
      memory: "8Gi" # Same as requests = Guaranteed QoS
```

## Cost Optimization

### Development Environments

Use minimal resources and spot instances:

```yaml
spec:
  resources:
    requests:
      cpu: "250m"
      memory: "512Mi"
    limits:
      cpu: "1"
      memory: "2Gi"
```

### Production Environments

Balance cost and reliability:

1. **Validators**: Use reserved instances (predictable cost)
2. **Horizon**: Use autoscaling with spot instances for burst capacity
3. **Soroban**: Mix on-demand and spot instances

## Troubleshooting

### OOMKilled Pods

**Symptoms**: Pods restart with exit code 137

**Solutions**:

1. Increase memory limits
2. Check for memory leaks in application logs
3. Review database query efficiency (Horizon)

### CPU Throttling

**Symptoms**: High latency, slow response times

**Solutions**:

1. Increase CPU limits
2. Check for inefficient queries or operations
3. Consider horizontal scaling (Horizon/Soroban)

### Disk I/O Bottlenecks

**Symptoms**: High I/O wait, slow ledger writes

**Solutions**:

1. Upgrade to faster storage class (SSD/NVMe)
2. Increase IOPS provisioning
3. Consider local SSD for validators

## References

- [Kubernetes Resource Management](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/)
- [Stellar Core Configuration](https://developers.stellar.org/docs/run-core-node)
- [Horizon Configuration](https://developers.stellar.org/docs/run-api-server)
- [Soroban RPC Configuration](https://developers.stellar.org/docs/data/rpc)

## Hardcoded Defaults

The operator provides these fallback defaults when no configuration is specified:

| Node Type | CPU Request | Memory Request | CPU Limit | Memory Limit |
| --------- | ----------- | -------------- | --------- | ------------ |
| Validator | 500m        | 1Gi            | 2         | 4Gi          |
| Horizon   | 250m        | 512Mi          | 2         | 4Gi          |
| Soroban   | 500m        | 2Gi            | 4         | 8Gi          |

These defaults are defined in `src/controller/operator_config.rs::hardcoded_defaults()`.
