# Metric Naming Conventions and Dashboard Access

**Issue:** #1389 — Implement comprehensive metrics and monitoring dashboards  
**Audience:** Platform engineers, SREs, developers

---

## Overview

Stellar-K8s exports Prometheus metrics from three sources:

| Source | Prefix | Description |
|--------|--------|-------------|
| Stellar Core binary | `stellar_core_*` | Raw counters/gauges from the node |
| Horizon binary | `stellar_horizon_*` | Horizon API and ingestion metrics |
| Stellar-K8s operator | `stellar_operator_*` | Reconciler, leader, and CRD metrics |
| Derived (recording rules) | `stellar:*` | Aggregated/computed metrics via PrometheusRule |

---

## Naming Conventions

### Raw Metrics (`stellar_core_*`)

Exported directly by Stellar Core's built-in Prometheus endpoint (`/metrics`).

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `stellar_core_transactions_total` | Counter | `namespace`, `pod`, `network` | Total transactions processed |
| `stellar_core_operations_total` | Counter | `namespace`, `pod`, `network` | Total operations processed |
| `stellar_core_ledger_sequence_total` | Counter | `namespace`, `pod` | Total ledgers closed |
| `stellar_core_ledger_close_time_seconds` | Histogram | `namespace`, `pod` | Ledger close duration |
| `stellar_core_ledger_close_time_seconds` | Gauge | `namespace`, `pod` | Wall-clock time of last ledger close |
| `stellar_core_is_synced` | Gauge | `namespace`, `pod` | 1 = synced, 0 = not synced |
| `stellar_core_is_validator` | Gauge | `namespace`, `pod` | 1 = validator mode, 0 = watcher |
| `stellar_core_quorum_intact` | Gauge | `namespace`, `pod` | 1 = quorum intact, 0 = broken |
| `stellar_core_connection_count` | Gauge | `namespace`, `pod`, `direction` | Peer connection count |

### Operator Metrics (`stellar_operator_*`)

Exported by the Rust operator via `prometheus-client`.

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `stellar_operator_info` | Gauge | `version`, `git_sha`, `rust_version` | Always 1; build info |
| `stellar_operator_leader_status` | Gauge | `instance` | 1 = leader, 0 = follower |
| `stellar_operator_uptime_seconds_total` | Counter | | Operator process uptime |
| `stellar_operator_reconciliation_duration_seconds` | Histogram | `resource`, `namespace` | Reconcile latency |
| `stellar_operator_reconciliation_errors_total` | Counter | `resource`, `namespace`, `error_kind` | Reconcile errors |
| `stellar_operator_reconciliation_success_total` | Counter | `resource`, `namespace` | Successful reconciles |
| `stellar_pvc_disk_usage_percent` | Gauge | `namespace`, `name`, `node_type`, `network` | PVC disk usage % |
| `stellar_ledger_sequence` | Gauge | `namespace`, `name`, `node_type`, `network` | Current ledger sequence |
| `stellar_ingestion_lag` | Gauge | `namespace`, `name`, `node_type`, `network` | Ledger lag vs network |
| `stellar_node_up` | Gauge | `namespace`, `name`, `node_type`, `network` | Node availability |

### Derived Recording Rules (`stellar:*:*`)

Computed by `PrometheusRule` in `monitoring/stellar-core-metrics.yaml`.

| Rule | Expression | Description |
|------|-----------|-------------|
| `stellar:core:tps:1m` | `rate(stellar_core_transactions_total[1m])` | Per-node 1m TPS |
| `stellar:core:tps:5m` | `rate(stellar_core_transactions_total[5m])` | Per-node 5m TPS |
| `stellar:core:tps:cluster:5m` | `sum(stellar:core:tps:5m)` | Cluster-wide TPS |
| `stellar:core:ledger:close_lag_seconds` | `time() - stellar_core_ledger_close_time_seconds` | Seconds behind wall-clock |
| `stellar:core:ledger:close_time:p95` | `histogram_quantile(0.95, ...)` | p95 ledger close time |
| `stellar:core:ledger:close_time:p99` | `histogram_quantile(0.99, ...)` | p99 ledger close time |
| `stellar:core:node:memory_mib` | `process_resident_memory_bytes / 1048576` | RSS in MiB |
| `stellar:core:node:cpu_percent` | `rate(process_cpu_seconds_total[5m]) * 100` | CPU % (5m) |
| `stellar:core:cluster:synced_nodes` | `count(stellar:core:node:synced == 1)` | Count of synced nodes |
| `stellar:operator:reconcile:error_rate:5m` | `rate(stellar_operator_reconciliation_errors_total[5m])` | Operator error rate |
| `stellar:operator:reconcile:duration:p99` | `histogram_quantile(0.99, ...)` | Operator p99 latency |

---

## Alert Conventions

All Stellar-K8s alerts follow this labelling schema:

```yaml
labels:
  severity: critical | warning | info
  component: stellar-core | horizon | operator | blue-green
  issue: "<GitHub issue number>"   # traceability
```

### Severity definitions

| Severity | Meaning | Response SLA |
|----------|---------|-------------|
| `critical` | Node outage, data loss risk, quorum lost | Immediate (< 15 min) |
| `warning` | Degraded performance, lag growing, memory high | < 1 hour |
| `info` | Unusual but non-impacting (e.g. TPS spike) | Next business day |

---

## Dashboard Access

### Importing dashboards

1. **Open Grafana** and navigate to **Dashboards → Import**
2. **Upload JSON** — choose one of the files below
3. **Select Prometheus datasource** when prompted
4. **Save and open**

### Available dashboards

| File | UID | Title |
|------|-----|-------|
| `monitoring/grafana-cluster-overview.json` | `stellar-cluster-overview-1389` | Cluster Overview (TPS, lag, health) |
| `monitoring/grafana-per-node-stats.json` | `stellar-per-node-stats-1389` | Per-Node drill-down |
| `monitoring/grafana-alerting.json` | `stellar-alerting-1389` | Active alerts + history |
| `monitoring/grafana-dashboard.json` | `stellar-operator-main` | Operator main (reconciler, leader) |
| `monitoring/grafana-validator-dashboard.json` | `stellar-validator` | Validator-specific |
| `monitoring/grafana-horizon-dashboard.json` | `stellar-horizon` | Horizon ingestion |

### Verifying scrapeability

```bash
# Check that the ServiceMonitor is picked up
kubectl get servicemonitor -n stellar-system

# Verify Prometheus is scraping Stellar Core
kubectl port-forward svc/prometheus-operated 9090 -n monitoring &
curl -s 'http://localhost:9090/api/v1/targets' | \
  python3 -c "import json,sys; [print(t['labels']['job']) for t in json.load(sys.stdin)['data']['activeTargets']]"

# Sample a metric
curl -s 'http://localhost:9090/api/v1/query?query=stellar_core_is_synced'
```

### Rendering with sample data (offline testing)

```bash
# Generate 5 minutes of fake metrics for dashboard smoke test
python3 scripts/generate-sample-metrics.py \
  --nodes 3 \
  --duration 300 \
  --output /tmp/stellar-metrics.txt

# Push via pushgateway
curl --data-binary @/tmp/stellar-metrics.txt \
  http://localhost:9091/metrics/job/stellar-core-sample
```

---

## Adding custom metrics from Rust

To add a new operator metric, follow the pattern in `src/controller/metrics.rs`:

```rust
// 1. Declare the metric
static MY_METRIC: once_cell::sync::Lazy<prometheus_client::metrics::gauge::Gauge> =
    once_cell::sync::Lazy::new(|| {
        let g = prometheus_client::metrics::gauge::Gauge::default();
        // register with the global registry (see metrics::register_metrics)
        g
    });

// 2. Record a value
MY_METRIC.set(42);
```

Metric names must:
- Use `snake_case`
- Start with `stellar_operator_` for operator internals
- Use `_total` suffix for counters
- Use `_seconds` suffix for durations
- Use `_bytes` suffix for byte quantities
- Include `_bucket`, `_count`, `_sum` for histograms (auto-generated)
