# Stellar Operator Prometheus Metric Reference

The `stellar-operator` binary exports custom metrics on `/metrics` when built with the `metrics` feature. All metrics below are registered in `src/controller/metrics.rs` in the global `REGISTRY`.

Validate completeness locally:

```bash
chmod +x scripts/validate-metrics.sh
./scripts/validate-metrics.sh
```

Scrape configuration: [`examples/monitoring/pod-monitor.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/monitoring/pod-monitor.yaml).

## Common label sets

| Label set | Labels |
| --- | --- |
| `NodeLabels` | `namespace`, `name`, `node_type`, `network`, `hardware_generation` |
| `ReconcileLabels` | `controller` |
| `ErrorLabels` | `controller`, `kind` |
| `ReactiveLabels` | `namespace`, `name` |
| `SorobanLabels` | `namespace`, `name`, `network`, `contract_id` |
| `ContractInvocationLabels` | `namespace`, `name`, `network`, `contract_type` |
| `TransactionResultLabels` | `namespace`, `name`, `network`, `result` |
| `DRDrillLabels` | `namespace`, `name`, `status` |
| `OperatorInfoLabels` | `version`, `git_sha`, `rust_version` |

## Operator reconcile metrics

| Metric | Type | Labels | Description |
| --- | --- | --- | --- |
| `reconcile_duration_seconds` | Histogram | `controller` | Duration of reconcile loops in seconds (legacy name). Buckets: 1 ms – ~32 s (16 exponential buckets). |
| `stellar_reconcile_duration_seconds` | Histogram | `controller` | Duration of reconcile loops in seconds. Same buckets as above. |
| `stellar_reconcile_errors_total` | Counter | `controller`, `kind` | Total reconcile errors by controller and error kind (`kube`, `validation`, `unknown`). |
| `stellar_operator_reconcile_errors_total` | Counter | `controller`, `kind` | Operator-level reconcile errors by controller and kind. |

## Stellar node metrics

| Metric | Type | Labels | Description |
| --- | --- | --- | --- |
| `stellar_node_ledger_sequence` | Gauge | `NodeLabels` | Current ledger sequence number of the Stellar node. |
| `stellar_node_ingestion_lag` | Gauge | `NodeLabels` | Lag between latest network ledger and node ledger. |
| `stellar_horizon_tps` | Gauge | `NodeLabels` | Transactions per second for Horizon API nodes. |
| `stellar_node_active_connections` | Gauge | `NodeLabels` | Number of active peer connections. |
| `stellar_archive_ledger_lag` | Gauge | `NodeLabels` | Ledgers the history archive is behind the validator (0 = in sync). |
| `stellar_node_sync_status` | Gauge | `NodeLabels` | Sync phase encoded as integer: 0=Pending, 1=Creating, 2=Running, 3=Syncing, 4=Ready, 5=Failed, 6=Degraded, 7=Suspended, 8=Remediating, 9=Terminating. |
| `stellar_node_up` | Gauge | `NodeLabels` | Pod readiness indicator (1=up, 0=down). |
| `stellar_archive_integrity_status` | Gauge | `NodeLabels` | History archive integrity (1=healthy, 0=corrupted). |
| `stellar_zk_archive_signature_valid` | Gauge | `NodeLabels` | ZK archive manifest signature validity (1=valid, 0=invalid/missing). |
| `stellar_zk_archive_chain_gaps_total` | Gauge | `NodeLabels` | Checkpoint gaps in ZK archive hash chain (0=complete). |

## Reactive status metrics

| Metric | Type | Labels | Description |
| --- | --- | --- | --- |
| `stellar_reactive_status_updates_total` | Counter | `namespace`, `name` | Reactive status updates from database triggers. |
| `stellar_api_polls_avoided_total` | Counter | `namespace`, `name` | API health polls avoided due to reactive updates. |

## Quorum analysis metrics

| Metric | Type | Labels | Description |
| --- | --- | --- | --- |
| `stellar_quorum_critical_nodes` | Gauge | `NodeLabels` | Critical nodes whose failure would break consensus. |
| `stellar_quorum_min_overlap` | Gauge | `NodeLabels` | Minimum overlap count between quorum slices. |
| `stellar_quorum_consensus_latency_ms` | Histogram | `NodeLabels` | Consensus latency per validator in milliseconds. Buckets: 1 ms – ~32 s. |
| `stellar_quorum_fragility_score` | Gauge | `NodeLabels` | Fragility score (0.0=resilient, 1.0=fragile). |

## Soroban RPC metrics

| Metric | Type | Labels | Description |
| --- | --- | --- | --- |
| `soroban_rpc_wasm_execution_duration_microseconds` | Histogram | `SorobanLabels` | Wasm host execution time in microseconds. |
| `soroban_rpc_contract_storage_fee_stroops` | Histogram | `SorobanLabels` | Contract storage fees in stroops. |
| `soroban_rpc_wasm_vm_memory_bytes` | Gauge | `SorobanLabels` | Wasm VM memory usage in bytes. |
| `soroban_rpc_contract_invocation_cpu_instructions` | Gauge | `SorobanLabels` | CPU instructions per contract invocation. |
| `soroban_rpc_contract_invocation_memory_bytes` | Gauge | `SorobanLabels` | Memory bytes per contract invocation. |
| `soroban_rpc_contract_invocations_total` | Counter | `ContractInvocationLabels` | Contract invocations by type (`token`, `defi`, etc.). |
| `soroban_rpc_transaction_result_total` | Counter | `TransactionResultLabels` | Transactions by result (`success`, `failed`). |
| `soroban_rpc_host_function_calls_total` | Counter | `SorobanLabels` | Total host function calls. |

## Disaster recovery drill metrics

| Metric | Type | Labels | Description |
| --- | --- | --- | --- |
| `stellar_dr_drill_execution_time_ms` | Histogram | `DRDrillLabels` | DR drill execution time in milliseconds. |
| `stellar_dr_drill_executions_total` | Counter | `DRDrillLabels` | Total DR drill executions by status (`success`, `failed`, `rolled_back`). |
| `stellar_dr_drill_time_to_recovery_ms` | Gauge | `DRDrillLabels` | Time to recovery (TTR) in milliseconds. |

## PVC autoscaling metrics

| Metric | Type | Labels | Description |
| --- | --- | --- | --- |
| `stellar_pvc_disk_usage_percent` | Gauge | `NodeLabels` | PVC disk usage percentage (0–100). |
| `stellar_pvc_expansion_total` | Counter | `NodeLabels` | Total PVC expansion events. |
| `stellar_pvc_size_bytes` | Gauge | `NodeLabels` | Current PVC size in bytes. |
| `stellar_pvc_expansion_count` | Gauge | `NodeLabels` | Number of expansions performed on this PVC. |

## Operator process metrics

| Metric | Type | Labels | Description |
| --- | --- | --- | --- |
| `stellar_operator_info` | Gauge | `version`, `git_sha`, `rust_version` | Build info gauge (always 1). |
| `stellar_operator_leader_status` | Gauge | — | Leader election status (1=leader, 0=follower). |
| `stellar_operator_uptime_seconds` | Counter | — | Operator process uptime in seconds. |
| `stellar_operator_ready` | Gauge | — | Readiness (1=watch healthy and first reconcile complete). |

## Recommended Grafana queries

### Node health overview

```promql
sum by (namespace, name, network) (stellar_node_up)
```

### Reconcile latency p99

```promql
histogram_quantile(0.99,
  sum by (le, controller) (rate(stellar_reconcile_duration_seconds_bucket[5m]))
)
```

### Ingestion lag alert preview

```promql
max by (namespace, name) (stellar_node_ingestion_lag) > 100
```

### Quorum fragility

```promql
max(stellar_quorum_fragility_score) by (namespace, name)
```

### Soroban invocation error rate

```promql
sum(rate(soroban_rpc_transaction_result_total{result="failed"}[5m]))
  / sum(rate(soroban_rpc_transaction_result_total[5m]))
```

## Recommended alerting rules

```yaml
groups:
  - name: stellar-operator
    rules:
      - alert: StellarNodeDown
        expr: stellar_node_up == 0
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Stellar node {{ $labels.name }} is down"

      - alert: StellarIngestionLagHigh
        expr: stellar_node_ingestion_lag > 500
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "Node {{ $labels.name }} ingestion lag exceeds 500 ledgers"

      - alert: StellarArchiveLagHigh
        expr: stellar_archive_ledger_lag > 64
        for: 15m
        labels:
          severity: warning
        annotations:
          summary: "Archive lag high for {{ $labels.name }}"

      - alert: StellarQuorumFragile
        expr: stellar_quorum_fragility_score > 0.7
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Quorum fragility score critical for {{ $labels.name }}"

      - alert: StellarOperatorNotReady
        expr: stellar_operator_ready == 0
        for: 2m
        labels:
          severity: critical
        annotations:
          summary: "stellar-operator is not ready"

      - alert: StellarReconcileErrors
        expr: increase(stellar_operator_reconcile_errors_total[15m]) > 10
        labels:
          severity: warning
        annotations:
          summary: "Elevated reconcile errors on {{ $labels.controller }}"

      - alert: StellarPvcDiskFull
        expr: stellar_pvc_disk_usage_percent > 85
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "PVC usage above 85% for {{ $labels.name }}"
```

## Sidecar metrics (separate endpoints)

These metrics are **not** in the main operator registry but may appear in multi-container deployments:

| Component | Default port | Key metrics |
| --- | --- | --- |
| Fork detector | 9102 | Fork detection counters and gauges |
| Byzantine watcher | 9101 | Cross-region SCP observation metrics |
| CVE scanner | (sidecar) | CVE scan result gauges when enabled |

Scrape sidecars with dedicated `PodMonitor` objects targeting their annotated `prometheus.io/port`.
