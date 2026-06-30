# Performance Tuning Guide

Optimize Stellar node performance in Kubernetes environments to maximize throughput, minimize ledger sync lag, and reduce resource costs.

---

## CPU Optimization

### Resource Requests and Limits

Set CPU requests to guarantee scheduling on nodes with sufficient capacity. Avoid setting CPU limits on validators — CPU throttling causes ledger close delays.

```yaml
spec:
  resources:
    requests:
      cpu: "4"        # Guaranteed allocation
      memory: "8Gi"
    limits:
      # Do NOT set cpu limits on validators — throttling causes ledger lag
      memory: "16Gi"  # Memory limit is safe; OOM is preferable to throttling
```

For Horizon and Soroban RPC (stateless), CPU limits are acceptable:

```yaml
# Horizon: CPU limits are fine
resources:
  requests:
    cpu: "2"
    memory: "4Gi"
  limits:
    cpu: "4"
    memory: "8Gi"
```

### CPU Pinning (NUMA-Aware Scheduling)

On bare-metal or dedicated nodes, use the Kubernetes CPU Manager to pin Stellar Core to exclusive CPU cores:

```yaml
# Node label for CPU-pinned nodes
# kubectl label node <node> stellar.org/cpu-pinned=true

spec:
  nodeSelector:
    stellar.org/cpu-pinned: "true"
  resources:
    requests:
      cpu: "4"      # Must be integer for exclusive allocation
      memory: "8Gi"
    limits:
      cpu: "4"      # Must equal requests for Guaranteed QoS class
      memory: "8Gi"
```

Enable the CPU Manager policy in kubelet:

```yaml
# /var/lib/kubelet/config.yaml
cpuManagerPolicy: static
```

### Topology Spread for Multi-Core Utilization

Stellar Core is single-threaded for consensus but uses multiple threads for I/O and history. Spread replicas across NUMA nodes:

```yaml
spec:
  topologySpreadConstraints:
    - maxSkew: 1
      topologyKey: topology.kubernetes.io/zone
      whenUnsatisfiable: DoNotSchedule
      labelSelector:
        matchLabels:
          app: stellar-validator
```

---

## Memory Optimization

### Stellar Core Memory Tuning

Stellar Core's in-memory ledger cache (`LEDGER_CACHE_SIZE`) is the primary memory consumer. Tune it based on available RAM:

```toml
# stellar-core.cfg (managed via operator ConfigMap)
# Default: 4096 entries. Increase for faster ledger access.
LEDGER_CACHE_SIZE=16384

# BucketList cache — set to ~25% of available RAM
BUCKET_DIR_PATH="/data/buckets"
```

Configure via the `StellarNode` spec:

```yaml
spec:
  stellarCoreConfig:
    ledgerCacheSize: 16384
    bucketDirPath: "/data/buckets"
```

### JVM Heap for Horizon

Horizon is a Go application; it does not use a JVM. However, its PostgreSQL connection pool size directly affects memory:

```yaml
# Horizon environment variables
env:
  - name: HORIZON_DB_MAX_OPEN_CONNS
    value: "20"
  - name: HORIZON_DB_MAX_IDLE_CONNS
    value: "5"
  - name: HORIZON_CONNECTION_TIMEOUT
    value: "30"
```

### Memory Limits and OOM Behavior

Set memory limits 2× the request to absorb spikes without OOM-killing the pod:

```yaml
resources:
  requests:
    memory: "8Gi"
  limits:
    memory: "16Gi"
```

Configure the kernel OOM score to protect the validator process:

```yaml
spec:
  containers:
    - name: stellar-core
      securityContext:
        # Lower OOM score = less likely to be killed
        # Requires SYS_RESOURCE capability or privileged mode
        runAsNonRoot: true
```

---

## Storage Performance Tuning

### Use Local NVMe for Validators

Cloud block storage (EBS, GCP PD) introduces 1–5 ms latency per I/O operation. Local NVMe drives reduce this to < 0.1 ms, cutting ledger sync lag from 5–15 s to < 1 s.

```yaml
spec:
  storage:
    mode: Local
    storageClass: "local-path"
    size: "1Ti"
    nodeAffinity:
      requiredDuringSchedulingIgnoredDuringExecution:
        nodeSelectorTerms:
          - matchExpressions:
              - key: stellar.org/nvme
                operator: In
                values: ["true"]
```

Label NVMe nodes:

```bash
kubectl label node <nvme-node> stellar.org/nvme=true
```

### EBS Optimization (AWS)

When local NVMe is not available, use `gp3` volumes with provisioned IOPS:

```yaml
# StorageClass for high-performance EBS
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: stellar-gp3
provisioner: ebs.csi.aws.com
parameters:
  type: gp3
  iops: "16000"       # Max for gp3: 16,000 IOPS
  throughput: "1000"  # MB/s
allowVolumeExpansion: true
volumeBindingMode: WaitForFirstConsumer
```

```yaml
spec:
  storage:
    storageClass: "stellar-gp3"
    size: "500Gi"
```

### Filesystem Tuning

Use `ext4` with `noatime` and `data=writeback` for the BucketList directory:

```bash
# Mount options (set via StorageClass or node configuration)
mkfs.ext4 -E lazy_itable_init=0,lazy_journal_init=0 /dev/nvme0n1
mount -o noatime,data=writeback /dev/nvme0n1 /data
```

### Separate Data and BucketList Volumes

Isolate the BucketList (write-heavy) from the ledger database (read-heavy) on separate volumes:

```yaml
spec:
  storage:
    dataVolume:
      storageClass: "stellar-gp3"
      size: "200Gi"
    bucketVolume:
      storageClass: "local-nvme"
      size: "800Gi"
```

---

## Network Optimization

### Peer Connection Tuning

Increase the number of peer connections for better network propagation:

```toml
# stellar-core.cfg
TARGET_PEER_CONNECTIONS=25
MAX_ADDITIONAL_PEER_CONNECTIONS=10
MAX_PENDING_CONNECTIONS=50
```

### TCP Buffer Sizes

Increase kernel TCP buffers on nodes running validators:

```bash
# /etc/sysctl.d/99-stellar.conf
net.core.rmem_max=134217728
net.core.wmem_max=134217728
net.ipv4.tcp_rmem=4096 87380 134217728
net.ipv4.tcp_wmem=4096 65536 134217728
net.ipv4.tcp_congestion_control=bbr
```

Apply via a DaemonSet or node configuration tool (e.g., `tuned`).

### Network Policy for Reduced Latency

Restrict unnecessary traffic to reduce network processing overhead:

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: stellar-validator-netpol
  namespace: stellar
spec:
  podSelector:
    matchLabels:
      app: stellar-validator
  policyTypes:
    - Ingress
    - Egress
  ingress:
    - ports:
        - port: 11625  # Stellar peer port
  egress:
    - ports:
        - port: 11625  # Outbound peers
        - port: 443    # History archives (HTTPS)
        - port: 5432   # PostgreSQL (Horizon)
```

### Service Mesh Overhead

If using Istio or Linkerd, disable the sidecar proxy for Stellar Core pods to eliminate the ~0.5 ms per-hop overhead:

```yaml
metadata:
  annotations:
    sidecar.istio.io/inject: "false"   # Istio
    linkerd.io/inject: disabled         # Linkerd
```

Use mTLS at the Stellar Core level instead (see [mTLS Guide](mtls-guide.md)).

---

## Database Tuning

### PostgreSQL for Horizon

Horizon's PostgreSQL database is the primary bottleneck for API response times. Key tuning parameters:

```sql
-- postgresql.conf
shared_buffers = 4GB              -- 25% of RAM
effective_cache_size = 12GB       -- 75% of RAM
work_mem = 64MB                   -- Per sort/hash operation
maintenance_work_mem = 1GB        -- For VACUUM, CREATE INDEX
max_connections = 100             -- Match Horizon's connection pool
wal_level = minimal               -- Reduce WAL overhead (no replication needed)
synchronous_commit = off          -- Async commits for ~3× write throughput
checkpoint_completion_target = 0.9
random_page_cost = 1.1            -- For SSD/NVMe storage
```

Deploy PostgreSQL with these settings via the CNPG operator:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: horizon-db
  namespace: stellar
spec:
  instances: 1
  postgresql:
    parameters:
      shared_buffers: "4GB"
      effective_cache_size: "12GB"
      work_mem: "64MB"
      synchronous_commit: "off"
      random_page_cost: "1.1"
  storage:
    size: 200Gi
    storageClass: stellar-gp3
```

### Horizon Ingestion Tuning

Increase Horizon's ingestion parallelism to keep up with high-throughput networks:

```yaml
env:
  - name: HORIZON_INGEST_DISABLE_STATE_VERIFICATION
    value: "false"
  - name: HORIZON_PARALLEL_JOB_SIZE
    value: "100000"
  - name: HORIZON_HISTORY_RETENTION_COUNT
    value: "1000000"  # Keep last 1M ledgers
```

### Soroban RPC Database

Soroban RPC uses an embedded RocksDB. Tune the block cache size:

```yaml
spec:
  sorobanConfig:
    # RocksDB block cache (set to ~50% of available RAM)
    dbCacheSizeMb: 4096
    # Prefetch ledger entries for hot contracts
    prefetchBatchSize: 1000
```

---

## Benchmark Results

The following benchmarks were measured on a 3-node Testnet cluster (AWS `m6i.4xlarge`, `gp3` EBS with 16,000 IOPS).

### Storage Comparison

| Storage Type         | Peak IOPS | Read Latency | Write Latency | Avg Sync Lag |
|----------------------|-----------|--------------|---------------|--------------|
| Cloud Standard (EBS gp2) | ~3,000 | 1.5–2.5 ms | 2.0–5.0 ms | 5–15 s |
| Cloud High-Perf (EBS gp3) | ~16,000 | 0.5–1.0 ms | 0.5–1.5 ms | 1–3 s |
| Local NVMe           | 100,000+  | < 0.1 ms    | < 0.1 ms    | < 1 s |

### Horizon API Latency (p99)

| Configuration | `/transactions` | `/accounts/{id}` | `/ledgers` |
|---------------|-----------------|------------------|------------|
| Default (2 CPU, 4 Gi) | 450 ms | 120 ms | 80 ms |
| Tuned (4 CPU, 8 Gi, gp3) | 95 ms | 28 ms | 18 ms |
| Tuned + NVMe | 42 ms | 15 ms | 9 ms |

### Validator Ledger Close Time

| Configuration | Avg Close Time | p99 Close Time |
|---------------|----------------|----------------|
| Default | 5.2 s | 8.1 s |
| CPU unpinned, gp3 | 5.0 s | 6.8 s |
| CPU pinned, NVMe | 4.8 s | 5.2 s |

---

## Monitoring Performance

### Key Prometheus Metrics

```promql
# Ledger close time (target: < 5s)
stellar_node_ledger_close_duration_seconds

# Horizon ingestion lag (target: < 10 ledgers)
stellar_horizon_ingest_ledger_lag

# Disk usage percentage (trigger expansion at 80%)
kubelet_volume_stats_used_bytes / kubelet_volume_stats_capacity_bytes * 100

# CPU throttling (should be 0 for validators)
container_cpu_cfs_throttled_seconds_total{container="stellar-core"}

# PostgreSQL query time (target: < 10ms for simple queries)
pg_stat_statements_mean_exec_time_ms
```

### Performance Alerts

```yaml
groups:
  - name: stellar_performance
    rules:
      - alert: HighLedgerCloseTime
        expr: stellar_node_ledger_close_duration_seconds > 6
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Ledger close time exceeds 6s on {{ $labels.name }}"

      - alert: CPUThrottling
        expr: |
          rate(container_cpu_cfs_throttled_seconds_total{container="stellar-core"}[5m]) > 0
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "Stellar Core is being CPU throttled — remove CPU limits"

      - alert: HighHorizonIngestionLag
        expr: stellar_horizon_ingest_ledger_lag > 50
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Horizon ingestion is {{ $value }} ledgers behind"
```

---

## Performance Testing Toolkit

### Load Test with k6

Run the included k6 load test against your Horizon API:

```bash
# Install k6
brew install k6  # macOS
# or: https://k6.io/docs/getting-started/installation/

# Run the comprehensive performance test
k6 run benchmarks/k6/comprehensive-perf.js \
  -e HORIZON_URL=https://horizon.example.com \
  --vus 50 \
  --duration 5m
```

### Operator Load Test

```bash
k6 run benchmarks/k6/operator-load-test.js \
  -e OPERATOR_URL=https://stellar-operator.example.com \
  --vus 20 \
  --duration 10m
```

### Regression Testing

Compare performance between versions:

```bash
# Run regression test suite
./benchmarks/run-regression-test.sh \
  --baseline benchmarks/baselines/v0.1.0.json \
  --threshold 10  # Fail if any metric regresses > 10%
```

### Disk I/O Benchmark

```bash
# Benchmark storage on a validator node
kubectl exec -n stellar <validator-pod> -- \
  fio --name=randwrite --ioengine=libaio --iodepth=32 \
      --rw=randwrite --bs=4k --direct=1 --size=1G \
      --numjobs=4 --runtime=60 --group_reporting \
      --filename=/data/fio-test
```

---

## Troubleshooting Slow Performance

### Validator Falling Behind

1. Check ledger close time:
   ```bash
   kubectl stellar status -n stellar
   ```
2. Check for CPU throttling:
   ```bash
   kubectl top pod -n stellar
   kubectl describe pod <validator-pod> -n stellar | grep -A5 Limits
   ```
3. Check disk I/O wait:
   ```bash
   kubectl exec -n stellar <validator-pod> -- iostat -x 1 5
   ```
4. Check peer connectivity (low peer count = slow propagation):
   ```bash
   kubectl stellar logs <validator> | grep "peers connected"
   ```

### Horizon API Slow

1. Check PostgreSQL query performance:
   ```bash
   kubectl exec -n stellar <horizon-db-pod> -- \
     psql -U horizon -c "SELECT query, mean_exec_time FROM pg_stat_statements ORDER BY mean_exec_time DESC LIMIT 10;"
   ```
2. Check connection pool saturation:
   ```bash
   kubectl exec -n stellar <horizon-db-pod> -- \
     psql -U horizon -c "SELECT count(*) FROM pg_stat_activity;"
   ```
3. Check Horizon ingestion lag — if it is high, the API serves stale data:
   ```bash
   kubectl logs -n stellar <horizon-pod> | grep "ingest"
   ```

### High Memory Usage

1. Identify the memory consumer:
   ```bash
   kubectl top pod -n stellar --sort-by=memory
   ```
2. For Stellar Core, check BucketList size:
   ```bash
   kubectl exec -n stellar <validator-pod> -- du -sh /data/buckets/
   ```
3. Consider enabling archive pruning to reclaim disk and reduce memory pressure:
   ```bash
   stellar-operator prune-archive --archive-url s3://my-bucket/stellar-history --retention-days 30
   ```

---

## See Also

- [Resource Limits Reference](resource-limits.md)
- [Proactive Disk Scaling](proactive-disk-scaling.md)
- [Benchmarking Guide](benchmarking.md)
- [Benchmark Compare Tool](benchmark-compare.md)
- [Scalability](scalability.md)
