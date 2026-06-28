# Stellar-Specific Prometheus Metrics Guide

This guide documents all Stellar-specific metrics exported by the stellar-k8s operator for comprehensive observability of Stellar nodes.

## Table of Contents

- [Ledger Metrics](#ledger-metrics)
- [Transaction Throughput Metrics](#transaction-throughput-metrics)
- [Peer Connection Quality Metrics](#peer-connection-quality-metrics)
- [History Archive Health Metrics](#history-archive-health-metrics)
- [Database Metrics](#database-metrics)
- [SCP Metrics](#scp-metrics)
- [Soroban Metrics](#soroban-metrics)
- [Horizon Metrics](#horizon-metrics)
- [Example Prometheus Queries](#example-prometheus-queries)

## Metric Labels

All metrics include the following standard labels for filtering:

- `node_name`: Name of the StellarNode resource
- `node_type`: Type of node (validator, horizon, soroban-rpc)
- `namespace`: Kubernetes namespace
- `network`: Stellar network (mainnet, testnet, futurenet)

## Ledger Metrics

### stellar_ledger_close_time_seconds
**Type**: Histogram  
**Description**: Time taken to close a ledger in seconds  
**Labels**: node_name, node_type, namespace, network

**Example Query**:
```promql
# Average ledger close time over 5 minutes
rate(stellar_ledger_close_time_seconds_sum[5m]) / rate(stellar_ledger_close_time_seconds_count[5m])
```

### stellar_ledger_close_time_p50
**Type**: Gauge  
**Description**: 50th percentile (median) ledger close time  
**Labels**: node_name, node_type, namespace, network

### stellar_ledger_close_time_p95
**Type**: Gauge  
**Description**: 95th percentile ledger close time  
**Labels**: node_name, node_type, namespace, network

### stellar_ledger_close_time_p99
**Type**: Gauge  
**Description**: 99th percentile ledger close time  
**Labels**: node_name, node_type, namespace, network

**Alert Example**:
```yaml
- alert: HighLedgerCloseTime
  expr: stellar_ledger_close_time_p99 > 10
  for: 5m
  annotations:
    summary: "Ledger close time is high on {{ $labels.node_name }}"
```

### stellar_ledger_operations_total
**Type**: Counter  
**Description**: Total number of operations processed in ledgers  
**Labels**: node_name, node_type, namespace, network

### stellar_ledger_transactions_total
**Type**: Counter  
**Description**: Total number of transactions processed in ledgers  
**Labels**: node_name, node_type, namespace, network

### stellar_ledger_failed_transactions_total
**Type**: Counter  
**Description**: Total number of failed transactions  
**Labels**: node_name, node_type, namespace, network

**Example Query**:
```promql
# Transaction failure rate
rate(stellar_ledger_failed_transactions_total[5m]) / rate(stellar_ledger_transactions_total[5m])
```

## Transaction Throughput Metrics

### stellar_transaction_throughput_tps
**Type**: Gauge  
**Description**: Current transaction throughput in transactions per second  
**Labels**: node_name, node_type, namespace, network

**Example Query**:
```promql
# Peak TPS in the last hour
max_over_time(stellar_transaction_throughput_tps[1h])
```

### stellar_transaction_throughput_ops
**Type**: Gauge  
**Description**: Current operation throughput in operations per second  
**Labels**: node_name, node_type, namespace, network

### stellar_transaction_apply_time_seconds
**Type**: Histogram  
**Description**: Time taken to apply a transaction  
**Labels**: node_name, node_type, namespace, network

**Example Query**:
```promql
# 99th percentile transaction apply time
histogram_quantile(0.99, rate(stellar_transaction_apply_time_seconds_bucket[5m]))
```

### stellar_transaction_queue_size
**Type**: Gauge  
**Description**: Number of transactions waiting in queue  
**Labels**: node_name, node_type, namespace, network

**Alert Example**:
```yaml
- alert: HighTransactionQueueSize
  expr: stellar_transaction_queue_size > 1000
  for: 5m
  annotations:
    summary: "Transaction queue is backing up on {{ $labels.node_name }}"
```

### stellar_transaction_success_rate
**Type**: Gauge  
**Description**: Ratio of successful transactions (0.0-1.0)  
**Labels**: node_name, node_type, namespace, network

## Peer Connection Quality Metrics

### stellar_peer_connection_count
**Type**: Gauge  
**Description**: Number of peer connections by state  
**Labels**: node_name, peer_id, state (authenticated, pending, failed)

**Example Query**:
```promql
# Total authenticated peers
sum(stellar_peer_connection_count{state="authenticated"}) by (node_name)
```

### stellar_peer_message_latency_ms
**Type**: Histogram  
**Description**: Peer message round-trip latency in milliseconds  
**Labels**: node_name, peer_id, state

**Example Query**:
```promql
# 95th percentile peer latency
histogram_quantile(0.95, rate(stellar_peer_message_latency_ms_bucket[5m]))
```

### stellar_peer_messages_sent_total
**Type**: Counter  
**Description**: Total messages sent to peers  
**Labels**: node_name, peer_id, state

### stellar_peer_messages_received_total
**Type**: Counter  
**Description**: Total messages received from peers  
**Labels**: node_name, peer_id, state

### stellar_peer_connection_errors_total
**Type**: Counter  
**Description**: Total peer connection errors  
**Labels**: node_name, peer_id, state

**Alert Example**:
```yaml
- alert: HighPeerConnectionErrors
  expr: rate(stellar_peer_connection_errors_total[5m]) > 0.1
  for: 5m
  annotations:
    summary: "High peer connection error rate on {{ $labels.node_name }}"
```

### stellar_peer_bandwidth_bytes_sent
**Type**: Counter  
**Description**: Total bytes sent to peers  
**Labels**: node_name, peer_id, state

### stellar_peer_bandwidth_bytes_received
**Type**: Counter  
**Description**: Total bytes received from peers  
**Labels**: node_name, peer_id, state

**Example Query**:
```promql
# Peer bandwidth usage (MB/s)
rate(stellar_peer_bandwidth_bytes_sent[5m]) / 1024 / 1024
```

### stellar_peer_connection_uptime_seconds
**Type**: Gauge  
**Description**: Peer connection uptime in seconds  
**Labels**: node_name, peer_id, state

## History Archive Health Metrics

### stellar_archive_health_status
**Type**: Gauge  
**Description**: History archive health status (1=healthy, 0=unhealthy)  
**Labels**: node_name, archive_name, archive_url

**Alert Example**:
```yaml
- alert: ArchiveUnhealthy
  expr: stellar_archive_health_status == 0
  for: 10m
  annotations:
    summary: "History archive {{ $labels.archive_name }} is unhealthy"
```

### stellar_archive_last_check_timestamp
**Type**: Gauge  
**Description**: Unix timestamp of last archive health check  
**Labels**: node_name, archive_name, archive_url

### stellar_archive_check_duration_seconds
**Type**: Histogram  
**Description**: Duration of archive health check  
**Labels**: node_name, archive_name, archive_url

### stellar_archive_missing_files_total
**Type**: Counter  
**Description**: Total number of missing files in archive  
**Labels**: node_name, archive_name, archive_url

### stellar_archive_download_errors_total
**Type**: Counter  
**Description**: Total archive download errors  
**Labels**: node_name, archive_name, archive_url

### stellar_archive_upload_errors_total
**Type**: Counter  
**Description**: Total archive upload errors  
**Labels**: node_name, archive_name, archive_url

### stellar_archive_size_bytes
**Type**: Gauge  
**Description**: Total size of history archive in bytes  
**Labels**: node_name, archive_name, archive_url

**Example Query**:
```promql
# Archive size in GB
stellar_archive_size_bytes / 1024 / 1024 / 1024
```

## Database Metrics

### stellar_database_size_bytes
**Type**: Gauge  
**Description**: Database size in bytes  
**Labels**: node_name, node_type, namespace, network

**Example Query**:
```promql
# Database size in GB
stellar_database_size_bytes / 1024 / 1024 / 1024
```

### stellar_database_growth_rate_bytes_per_hour
**Type**: Gauge  
**Description**: Database growth rate in bytes per hour  
**Labels**: node_name, node_type, namespace, network

**Example Query**:
```promql
# Projected database size in 7 days
stellar_database_size_bytes + (stellar_database_growth_rate_bytes_per_hour * 24 * 7)
```

### stellar_database_query_duration_seconds
**Type**: Histogram  
**Description**: Database query execution time  
**Labels**: node_name, node_type, namespace, network

### stellar_database_connection_pool_active
**Type**: Gauge  
**Description**: Number of active database connections  
**Labels**: node_name, node_type, namespace, network

### stellar_database_connection_pool_idle
**Type**: Gauge  
**Description**: Number of idle database connections  
**Labels**: node_name, node_type, namespace, network

### stellar_database_transaction_count
**Type**: Counter  
**Description**: Total database transactions  
**Labels**: node_name, node_type, namespace, network

### stellar_database_slow_queries_total
**Type**: Counter  
**Description**: Total number of slow queries (>1s)  
**Labels**: node_name, node_type, namespace, network

**Alert Example**:
```yaml
- alert: HighSlowQueryRate
  expr: rate(stellar_database_slow_queries_total[5m]) > 1
  for: 5m
  annotations:
    summary: "High rate of slow queries on {{ $labels.node_name }}"
```

## SCP Metrics

### stellar_scp_nomination_time_seconds
**Type**: Histogram  
**Description**: Time spent in SCP nomination phase  
**Labels**: node_name, node_type, namespace, network

### stellar_scp_ballot_prepare_time_seconds
**Type**: Histogram  
**Description**: Time spent in SCP ballot prepare phase  
**Labels**: node_name, node_type, namespace, network

### stellar_scp_ballot_commit_time_seconds
**Type**: Histogram  
**Description**: Time spent in SCP ballot commit phase  
**Labels**: node_name, node_type, namespace, network

### stellar_scp_externalize_time_seconds
**Type**: Histogram  
**Description**: Time spent in SCP externalize phase  
**Labels**: node_name, node_type, namespace, network

**Example Query**:
```promql
# Total SCP consensus time (p99)
histogram_quantile(0.99, 
  rate(stellar_scp_nomination_time_seconds_bucket[5m]) +
  rate(stellar_scp_ballot_prepare_time_seconds_bucket[5m]) +
  rate(stellar_scp_ballot_commit_time_seconds_bucket[5m]) +
  rate(stellar_scp_externalize_time_seconds_bucket[5m])
)
```

### stellar_scp_quorum_intersection_failures
**Type**: Counter  
**Description**: Number of quorum intersection failures  
**Labels**: node_name, node_type, namespace, network

**Alert Example**:
```yaml
- alert: QuorumIntersectionFailure
  expr: rate(stellar_scp_quorum_intersection_failures[5m]) > 0
  for: 1m
  annotations:
    summary: "Quorum intersection failure detected on {{ $labels.node_name }}"
    severity: critical
```

## Soroban Metrics

### stellar_soroban_contract_invocations_total
**Type**: Counter  
**Description**: Total Soroban contract invocations  
**Labels**: node_name, node_type, namespace, network

### stellar_soroban_contract_execution_time_ms
**Type**: Histogram  
**Description**: Soroban contract execution time in milliseconds  
**Labels**: node_name, node_type, namespace, network

**Example Query**:
```promql
# Average contract execution time
rate(stellar_soroban_contract_execution_time_ms_sum[5m]) / rate(stellar_soroban_contract_execution_time_ms_count[5m])
```

### stellar_soroban_wasm_cache_hits
**Type**: Counter  
**Description**: Soroban WASM cache hits  
**Labels**: node_name, node_type, namespace, network

### stellar_soroban_wasm_cache_misses
**Type**: Counter  
**Description**: Soroban WASM cache misses  
**Labels**: node_name, node_type, namespace, network

**Example Query**:
```promql
# WASM cache hit rate
rate(stellar_soroban_wasm_cache_hits[5m]) / (rate(stellar_soroban_wasm_cache_hits[5m]) + rate(stellar_soroban_wasm_cache_misses[5m]))
```

### stellar_soroban_host_function_calls
**Type**: Counter  
**Description**: Total Soroban host function calls  
**Labels**: node_name, node_type, namespace, network

## Horizon Metrics

### stellar_horizon_request_duration_seconds
**Type**: Histogram  
**Description**: Horizon API request duration  
**Labels**: node_name, node_type, namespace, network

### stellar_horizon_requests_total
**Type**: Counter  
**Description**: Total Horizon API requests  
**Labels**: node_name, node_type, namespace, network

### stellar_horizon_ingestion_lag_seconds
**Type**: Gauge  
**Description**: Horizon ingestion lag behind network  
**Labels**: node_name, node_type, namespace, network

**Alert Example**:
```yaml
- alert: HighHorizonIngestionLag
  expr: stellar_horizon_ingestion_lag_seconds > 60
  for: 5m
  annotations:
    summary: "Horizon ingestion lag is high on {{ $labels.node_name }}"
```

### stellar_horizon_db_replication_lag_seconds
**Type**: Gauge  
**Description**: Horizon database replication lag  
**Labels**: node_name, node_type, namespace, network

## Example Prometheus Queries

### Network Health Overview
```promql
# Nodes by sync status
count(stellar_transaction_throughput_tps > 0) by (network, node_type)
```

### Performance Monitoring
```promql
# Average TPS across all validators
avg(stellar_transaction_throughput_tps{node_type="validator"}) by (network)

# Peak TPS in last 24 hours
max_over_time(stellar_transaction_throughput_tps[24h])

# Transaction success rate
avg(stellar_transaction_success_rate) by (node_name)
```

### Capacity Planning
```promql
# Database growth projection (30 days)
stellar_database_size_bytes + (stellar_database_growth_rate_bytes_per_hour * 24 * 30)

# Estimated days until disk full (assuming 1TB disk)
(1099511627776 - stellar_database_size_bytes) / stellar_database_growth_rate_bytes_per_hour / 24
```

### Peer Network Health
```promql
# Authenticated peer count
sum(stellar_peer_connection_count{state="authenticated"}) by (node_name)

# Peer connection error rate
rate(stellar_peer_connection_errors_total[5m])

# Average peer latency
avg(rate(stellar_peer_message_latency_ms_sum[5m]) / rate(stellar_peer_message_latency_ms_count[5m])) by (node_name)
```

### Archive Health
```promql
# Unhealthy archives
count(stellar_archive_health_status == 0) by (node_name)

# Archive error rate
rate(stellar_archive_download_errors_total[5m]) + rate(stellar_archive_upload_errors_total[5m])
```

### SCP Performance
```promql
# Total consensus time (all phases)
histogram_quantile(0.99,
  sum(rate(stellar_scp_nomination_time_seconds_bucket[5m])) by (le) +
  sum(rate(stellar_scp_ballot_prepare_time_seconds_bucket[5m])) by (le) +
  sum(rate(stellar_scp_ballot_commit_time_seconds_bucket[5m])) by (le) +
  sum(rate(stellar_scp_externalize_time_seconds_bucket[5m])) by (le)
)
```

### Soroban Performance
```promql
# Contract invocation rate
rate(stellar_soroban_contract_invocations_total[5m])

# WASM cache efficiency
rate(stellar_soroban_wasm_cache_hits[5m]) / (rate(stellar_soroban_wasm_cache_hits[5m]) + rate(stellar_soroban_wasm_cache_misses[5m])) * 100

# Average contract execution time
rate(stellar_soroban_contract_execution_time_ms_sum[5m]) / rate(stellar_soroban_contract_execution_time_ms_count[5m])
```

### Horizon Performance
```promql
# API request rate
rate(stellar_horizon_requests_total[5m])

# API latency (p95)
histogram_quantile(0.95, rate(stellar_horizon_request_duration_seconds_bucket[5m]))

# Ingestion lag
stellar_horizon_ingestion_lag_seconds
```

## Grafana Dashboard Integration

These metrics are designed to work seamlessly with the provided Grafana dashboards:

- `monitoring/grafana-validator-dashboard.json` - Validator node metrics
- `monitoring/grafana-horizon-dashboard.json` - Horizon API metrics
- `monitoring/grafana-soroban-dashboard.json` - Soroban RPC metrics
- `monitoring/grafana-operator-dashboard.json` - Operator health metrics

Import these dashboards into Grafana and configure your Prometheus datasource to visualize all metrics.

## Best Practices

1. **Use Recording Rules**: For frequently queried complex expressions, create Prometheus recording rules
2. **Set Appropriate Retention**: Store high-cardinality metrics (per-peer) with shorter retention
3. **Use Label Filtering**: Always filter by `namespace` and `network` in multi-tenant environments
4. **Monitor Cardinality**: Watch for label explosion, especially with peer_id labels
5. **Create Alerts**: Set up alerts for critical metrics like quorum failures and high lag

## Related Documentation

- [Grafana Dashboard Guide](../monitoring/GRAFANA_DASHBOARD_GUIDE.md)
- Prometheus Alert Rules
- Metrics Architecture
