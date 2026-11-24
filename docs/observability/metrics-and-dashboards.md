# Observability: Metrics and Dashboards

This document describes the metrics, Prometheus rules, and Grafana dashboards available for monitoring Stellar-K8s clusters.

## Table of Contents

- [Metrics Overview](#metrics-overview)
- [Core Metrics](#core-metrics)
- [Setup](#setup)
- [Dashboards](#dashboards)
- [Alerting](#alerting)
- [Metric Naming Conventions](#metric-naming-conventions)

## Metrics Overview

Stellar-K8s exposes three categories of metrics:

1. **Stellar Core Metrics** — Performance and health of Stellar Core nodes
2. **Operator Metrics** — Reconciliation performance and resource management
3. **Kubernetes Metrics** — Cluster-level infrastructure health (via kube-state-metrics)

All metrics are exposed in Prometheus text format on port 8080 at `/metrics`.

## Core Metrics

### Stellar Core TPS and Throughput

```promql
# Transactions per second (5-minute average)
rate(stellar_core_transactions_total[5m])

# Bytes processed per second
rate(stellar_core_bytes_processed_total[5m])

# Recording rules (pre-aggregated)
stellar:core:tps:5m    # TPS averaged over 5 minutes
stellar:core:tps:1h    # TPS averaged over 1 hour
```

**Use cases:**
- Monitor cluster throughput
- Detect performance degradation
- Capacity planning

### Stellar Core Ledger Metrics

```promql
# Current ledger sequence number
stellar_core_ledger_sequence

# Time since last ledger close (seconds)
time() - (stellar_core_ledger_close_time / 1000)

# Ledger close time percentiles
stellar:core:ledger:lag:p95
stellar:core:ledger:lag:p99
```

**Thresholds:**
- Ledger lag > 60s: Warning
- Ledger lag > 300s: Critical
- No ledger advancement for 5+ minutes: Critical

### Stellar Core Node Health

```promql
# Node synchronization status (0 or 1)
stellar_core_is_synced

# Validator status (0 or 1)
stellar_core_is_validator

# Quorum consensus status (0 or 1)
stellar_core_quorum_intact

# Memory usage (bytes)
process_resident_memory_bytes{job="stellar-core"}

# CPU usage (seconds)
rate(process_cpu_seconds_total{job="stellar-core"}[5m])

# Network connections
stellar_core_connection_count{direction="in"|"out"}

# Work queue depth (bytes)
stellar_core_work_queue_depth_bytes
```

**Recording rules:**
```yaml
stellar:core:node:is_synced
stellar:core:node:is_validator
stellar:core:node:quorum_intact
stellar:core:node:memory:usage
stellar:core:node:cpu:usage_percent
stellar:core:node:connections:incoming
stellar:core:node:connections:outgoing
stellar:core:node:queue_depth
```

### Operator Metrics

```promql
# Reconciliation duration (seconds) - histogram
stellar_operator_reconciliation_duration_seconds_bucket

# Reconciliation success counter
stellar_operator_reconciliation_success_total

# Reconciliation error counter
stellar_operator_reconciliation_errors_total

# Custom Resource Count
stellar_operator_crd_count_total

# Recording rules
stellar:operator:reconciliation:duration:p95
stellar:operator:reconciliation:duration:p99
stellar:operator:reconciliation:errors:5m
stellar:operator:reconciliation:success:rate
```

## Setup

### 1. Install Prometheus

Deploy Prometheus with the provided configuration:

```bash
kubectl apply -f monitoring/prometheus-config.yaml
kubectl apply -f monitoring/prometheus-rules.yaml
```

### 2. Install Grafana

Deploy Grafana dashboards:

```bash
kubectl apply -f monitoring/grafana-dashboards-cluster-overview.json
kubectl apply -f monitoring/grafana-dashboards-per-node-stats.json
kubectl apply -f monitoring/grafana-dashboards-alerting.json
```

### 3. Configure Alerting

AlertManager routes Prometheus alerts:

```yaml
alerting:
  alertmanagers:
    - static_configs:
        - targets: ["alertmanager:9093"]

rule_files:
  - "prometheus-rules.yaml"
```

## Dashboards

### Cluster Overview Dashboard

**File:** `monitoring/grafana-dashboards-cluster-overview.json`

High-level cluster health with 4 key metrics:

1. **Cluster Status** — Ready nodes count
2. **Operator Pods Running** — Operator replica count
3. **Stellar Core Nodes** — Active Stellar Core instances
4. **Synced Nodes** — Percentage of nodes in sync

### Per-Node Statistics Dashboard

**File:** `monitoring/grafana-dashboards-per-node-stats.json`

Detailed per-node view with:

1. **Node Sync Status** — Table of all nodes and sync state
2. **Ledger Sequence Per Node** — Sequence progression over time
3. **Memory Usage Per Node** — Memory consumption (MB)
4. **CPU Usage Per Node** — CPU utilization (%)
5. **Network Connections** — Incoming/outgoing connections
6. **Work Queue Depth** — Accumulated work in queue
7. **Quorum Status** — Table of quorum health per node
8. **Ledger Close Time Distribution** — Heatmap of close times

### Custom Dashboard Creation

To create a custom dashboard:

1. Open Grafana UI (e.g., `http://localhost:3000`)
2. Click **+** → **Dashboard**
3. Add panels with queries like:

```promql
# Example: TPS with threshold lines
query: stellar:core:tps:5m
threshold: 100  # warning
threshold: 5000 # critical
```

## Alerting

### Alert Rules

Prometheus evaluates alert rules in `prometheus-rules.yaml`:

| Alert | Severity | Condition | Duration |
|-------|----------|-----------|----------|
| `StellarCoreLowTPS` | Warning | TPS < 100 | 5m |
| `StellarCoreLedgerLag` | Warning | Lag > 60s | 2m |
| `StellarCoreLedgerStalled` | Critical | No advancement | 5m |
| `StellarCoreNotSynced` | Critical | is_synced == 0 | 5m |
| `StellarCoreQuorumNotIntact` | Warning | quorum_intact == 0 | 2m |
| `StellarCoreHighMemoryUsage` | Warning | Memory > 4GB | 5m |
| `StellarCoreHighCpuUsage` | Warning | CPU > 80% | 5m |
| `StellarOperatorReconciliationError` | Warning | Error rate > 0.1/s | 5m |

### Routing Alerts

AlertManager config example:

```yaml
global:
  resolve_timeout: 5m

route:
  receiver: 'default'
  routes:
    - match:
        severity: critical
      receiver: 'pagerduty'
    - match:
        component: stellar-core
      receiver: 'ops-team'

receivers:
  - name: 'default'
    slack_configs:
      - channel: '#alerts'
  - name: 'pagerduty'
    pagerduty_configs:
      - service_key: '${PAGERDUTY_KEY}'
  - name: 'ops-team'
    email_configs:
      - to: 'ops@example.com'
```

## Metric Naming Conventions

### Naming Format

```
stellar:[component]:[metric]:[dimension]
```

**Examples:**
- `stellar:core:tps:5m` — Stellar Core TPS (5-minute interval)
- `stellar:operator:reconciliation:duration:p99` — Operator reconciliation P99 latency
- `stellar:cluster:nodes:ready` — Ready Kubernetes nodes

### Components

- `core` — Stellar Core node metrics
- `operator` — Stellar-K8s operator metrics
- `cluster` — Kubernetes cluster metrics
- `node` — Individual node metrics

### Metric Types

- Recording rules: Pre-aggregated for performance
- Gauges: Current state (e.g., `is_synced`, `ledger_sequence`)
- Counters: Monotonically increasing (e.g., `transactions_total`)
- Histograms: Distribution data (e.g., `reconciliation_duration_seconds_bucket`)

### Dimension Examples

- `:5m`, `:1h` — Time intervals
- `:p95`, `:p99` — Percentiles
- `:incoming`, `:outgoing` — Direction
- `:bytes`, `:ms`, `:percent` — Units

## Troubleshooting

### Metrics not appearing

1. Verify Prometheus scrapes the operator:
   ```bash
   kubectl logs -n stellar-system -l app=prometheus
   ```

2. Check operator `/metrics` endpoint:
   ```bash
   kubectl port-forward -n stellar-system svc/stellar-operator 8080:8080
   curl http://localhost:8080/metrics
   ```

### Alerts not firing

1. Verify alert rules in Prometheus:
   ```bash
   kubectl port-forward -n stellar-system svc/prometheus 9090:9090
   # Visit http://localhost:9090/rules
   ```

2. Check AlertManager status:
   ```bash
   kubectl logs -n stellar-system -l app=alertmanager
   ```

### High cardinality issues

If using per-pod labels, consider:

```yaml
# Drop high-cardinality labels before scraping
metric_relabel_configs:
  - source_labels: [pod_id]
    action: drop  # Exclude pod ID labels
```

## References

- [Prometheus Documentation](https://prometheus.io/docs/)
- [Grafana Dashboard Best Practices](https://grafana.com/docs/grafana/latest/dashboards/)
- [Stellar Core Metrics](https://developers.stellar.org/docs/run-core-node/monitoring/)
