# Soroban RPC Monitoring Dashboard Guide

This guide explains how to use the Soroban-specific Grafana dashboard for monitoring smart contract operations on Stellar.

## Overview

The Soroban RPC monitoring dashboard (`grafana-soroban.json`) provides comprehensive visibility into:

- **Smart Contract Performance**: Wasm execution times, host function calls
- **Resource Consumption**: CPU, memory usage per contract invocation
- **Transaction Metrics**: Success/failure rates, ingestion rates
- **Storage Economics**: Contract storage fee distribution
- **System Health**: Database performance, goroutine count, GC metrics

## Dashboard Panels

### 1. Health & Status (Row 1)

#### Soroban RPC Health
- **Type**: Stat
- **Metric**: `up{job="soroban-rpc"}`
- **Description**: Shows if the Soroban RPC node is up and reachable
- **Colors**: Green (Healthy), Red (Down)

#### Latest Ledger Ingested
- **Type**: Stat
- **Metric**: `soroban_rpc_ingest_local_latest_ledger`
- **Description**: Current ledger sequence ingested by the RPC node
- **Use Case**: Verify the node is keeping up with the network

#### Transaction Ingestion Rate
- **Type**: Stat
- **Metric**: `rate(soroban_rpc_transactions_count[5m])`
- **Description**: Rate of Soroban transactions being processed
- **Unit**: Operations per second

#### Events Ingestion Rate
- **Type**: Stat
- **Metric**: `rate(soroban_rpc_events_count[5m])`
- **Description**: Rate of contract events being ingested
- **Unit**: Operations per second

### 2. Smart Contract Performance (Row 2)

#### Wasm Execution Time (Histogram)
- **Type**: Time series
- **Metrics**:
  - p50: `histogram_quantile(0.50, sum(rate(soroban_rpc_wasm_execution_duration_microseconds_bucket[5m])) by (le, instance))`
  - p95: `histogram_quantile(0.95, ...)`
  - p99: `histogram_quantile(0.99, ...)`
- **Description**: Distribution of Wasm host function execution times
- **Unit**: Microseconds
- **Interpretation**:
  - p50 < 1000µs: Excellent
  - p95 < 5000µs: Good
  - p99 < 10000µs: Acceptable
  - p99 > 50000µs: Investigate performance issues

#### Contract Storage Fee Distribution
- **Type**: Time series
- **Metrics**: Similar histogram quantiles for storage fees
- **Description**: Distribution of storage fees charged for contract operations
- **Unit**: Stroops (1 stroop = 0.0000001 XLM)
- **Use Case**: Monitor storage costs and identify expensive operations

### 3. Resource Consumption (Row 3)

#### Resource Consumption - CPU per Invocation
- **Type**: Time series
- **Metrics**:
  - `rate(process_cpu_seconds_total{job="soroban-rpc"}[5m]) * 100`
  - `avg(rate(soroban_rpc_contract_invocation_cpu_instructions[5m])) by (instance)`
- **Description**: CPU usage and instructions per contract invocation
- **Thresholds**:
  - Green: < 70%
  - Yellow: 70-90%
  - Red: > 90%

#### Resource Consumption - Memory per Invocation
- **Type**: Time series
- **Metrics**:
  - Process Memory: `process_resident_memory_bytes{job="soroban-rpc"}`
  - Wasm VM Memory: `avg(soroban_rpc_wasm_vm_memory_bytes) by (instance)`
  - Per Invocation: `avg(soroban_rpc_contract_invocation_memory_bytes) by (instance)`
- **Description**: Memory consumption at different levels
- **Thresholds**:
  - Green: < 1GB
  - Yellow: 1-2GB
  - Red: > 2GB

### 4. Transaction & Contract Metrics (Row 4)

#### Soroban Transaction Success/Failure Rate
- **Type**: Time series (stacked percentage)
- **Metrics**:
  - Success: `sum(rate(soroban_rpc_transaction_result_total{result="success"}[5m])) by (instance) / sum(rate(soroban_rpc_transaction_result_total[5m])) by (instance)`
  - Failure: Similar for `result="failed"`
- **Description**: Real-time success and failure rates
- **Alert**: If failure rate > 10% for 5 minutes

#### Contract Invocation Rate by Type
- **Type**: Time series (stacked)
- **Metric**: `sum(rate(soroban_rpc_contract_invocations_total[5m])) by (contract_type, instance)`
- **Description**: Breakdown of contract invocations by type
- **Use Case**: Identify most active contract types

### 5. Performance & Database (Row 5)

#### Database Round Trip Time
- **Type**: Time series
- **Metric**: `soroban_rpc_db_round_trip_time_seconds`
- **Description**: Time to execute `SELECT 1` query
- **Thresholds**:
  - Green: < 0.1s
  - Yellow: 0.1-0.5s
  - Red: > 0.5s
- **Alert**: If > 0.5s for 5 minutes

#### Host Function Call Distribution
- **Type**: Pie chart
- **Metric**: `sum(increase(soroban_rpc_host_function_calls_total[5m])) by (function_name)`
- **Description**: Which host functions are being called most
- **Use Case**: Understand contract behavior patterns

### 6. RPC Request Latency (Row 6)

#### RPC Request Latency by Method
- **Type**: Time series
- **Metrics**: p50, p95, p99 for `soroban_rpc_request_duration_seconds_bucket` by method
- **Description**: Latency of JSON RPC requests grouped by method
- **Methods**: `getHealth`, `getLatestLedger`, `getTransaction`, `simulateTransaction`, etc.
- **Thresholds**:
  - Green: < 0.1s
  - Yellow: 0.1-1s
  - Red: > 1s

### 7. System Health (Row 7)

#### Ledger Ingestion Lag
- **Type**: Stat
- **Metric**: `soroban_rpc_ingest_ledger_lag`
- **Description**: Number of ledgers behind the network
- **Thresholds**:
  - Green: < 5 ledgers
  - Yellow: 5-10 ledgers
  - Red: > 10 ledgers

#### Active Goroutines
- **Type**: Stat
- **Metric**: `go_goroutines{job="soroban-rpc"}`
- **Description**: Number of concurrent goroutines
- **Thresholds**:
  - Green: < 1000
  - Yellow: 1000-5000
  - Red: > 5000

#### Memory Allocation Rate
- **Type**: Stat
- **Metric**: `rate(go_memstats_alloc_bytes_total{job="soroban-rpc"}[5m])`
- **Description**: Rate of memory allocations
- **Unit**: Bytes per second

#### GC Pause Time (avg)
- **Type**: Stat
- **Metric**: `rate(go_gc_duration_seconds_sum{job="soroban-rpc"}[5m]) / rate(go_gc_duration_seconds_count{job="soroban-rpc"}[5m])`
- **Description**: Average garbage collection pause duration
- **Thresholds**:
  - Green: < 0.01s
  - Yellow: 0.01-0.1s
  - Red: > 0.1s

## Installation

### Prerequisites

- Grafana 10.0+
- Prometheus datasource configured
- Soroban RPC node with metrics enabled

### Import Steps

1. **Enable Metrics on Soroban RPC**:
   ```bash
   stellar-rpc --admin-endpoint 0.0.0.0:8001
   ```

2. **Configure Prometheus** to scrape Soroban RPC:
   ```yaml
   scrape_configs:
     - job_name: 'soroban-rpc'
       static_configs:
         - targets: ['soroban-rpc:8001']
   ```

3. **Import Dashboard**:
   - Open Grafana
   - Navigate to Dashboards → Import
   - Upload `monitoring/grafana-soroban.json`
   - Select Prometheus datasource
   - Click Import

## Alerting Rules

### Critical Alerts

```yaml
groups:
  - name: soroban_critical
    rules:
      - alert: SorobanRPCDown
        expr: up{job="soroban-rpc"} == 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Soroban RPC is down"
          
      - alert: HighTransactionFailureRate
        expr: |
          sum(rate(soroban_rpc_transaction_result_total{result="failed"}[5m])) /
          sum(rate(soroban_rpc_transaction_result_total[5m])) > 0.1
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Transaction failure rate > 10%"
```

### Warning Alerts

```yaml
      - alert: HighWasmExecutionLatency
        expr: histogram_quantile(0.99, rate(soroban_rpc_wasm_execution_duration_microseconds_bucket[5m])) > 100000
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Wasm execution p99 > 100ms"
          
      - alert: HighLedgerIngestionLag
        expr: soroban_rpc_ingest_ledger_lag > 10
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Ledger ingestion lagging > 10 ledgers"
          
      - alert: HighDatabaseLatency
        expr: soroban_rpc_db_round_trip_time_seconds > 0.5
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Database round trip time > 500ms"
```

## Troubleshooting

### High Wasm Execution Latency

**Symptoms**: p99 > 100ms

**Possible Causes**:
1. Complex contract logic
2. Insufficient CPU resources
3. High concurrent load

**Solutions**:
- Scale horizontally (add more RPC nodes)
- Increase CPU limits
- Optimize contract code
- Enable caching

### High Transaction Failure Rate

**Symptoms**: Failure rate > 10%

**Possible Causes**:
1. Invalid transactions
2. Insufficient fees
3. Contract errors
4. Network issues

**Solutions**:
- Check transaction logs
- Verify fee settings
- Review contract code
- Check network connectivity

### High Ledger Ingestion Lag

**Symptoms**: Lag > 10 ledgers

**Possible Causes**:
1. Slow database
2. Network latency
3. Insufficient resources
4. Stellar Core issues

**Solutions**:
- Optimize database (indexes, vacuum)
- Check network connectivity
- Increase resources
- Verify Stellar Core health

### High Memory Usage

**Symptoms**: Memory > 2GB

**Possible Causes**:
1. Memory leaks
2. Large contract state
3. High concurrent requests
4. Insufficient GC

**Solutions**:
- Restart RPC node
- Increase memory limits
- Tune GC parameters
- Investigate memory leaks

## Best Practices

1. **Set Up Alerts**: Configure Prometheus alerts for critical metrics
2. **Monitor Trends**: Watch for gradual degradation over time
3. **Capacity Planning**: Use metrics to plan resource scaling
4. **Regular Reviews**: Review dashboard weekly for anomalies
5. **Baseline Metrics**: Establish normal operating ranges
6. **Correlate Events**: Cross-reference with application logs
7. **Test Scenarios**: Use dashboard during load testing

## References

- [Stellar Soroban RPC Documentation](https://developers.stellar.org/docs/data/apis/rpc)
- [Soroban Metrics Guide](https://developers.stellar.org/docs/data/apis/rpc/admin-guide/monitoring)
- [Prometheus Query Examples](https://prometheus.io/docs/prometheus/latest/querying/examples/)
- [Grafana Dashboard Best Practices](https://grafana.com/docs/grafana/latest/dashboards/build-dashboards/best-practices/)

## Support

For issues or questions:
- GitHub Issues: [stellar-k8s/issues](https://github.com/stellar/stellar-k8s/issues)
- Stellar Discord: #soroban-rpc channel
- Documentation: [docs/](../docs/)
