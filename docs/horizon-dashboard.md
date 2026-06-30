# Horizon API Grafana Dashboard

A pre-built Grafana dashboard for monitoring Horizon API performance, request rates, error rates, and database connection pool health.

## Dashboard File

`monitoring/grafana-horizon.json`

## Panels

| Panel | Type | Description |
|---|---|---|
| HTTP Request Rate | Time series | Requests/s broken down by HTTP method and status code |
| HTTP Error Rate | Stat | Percentage of 4xx/5xx responses |
| Ingestion Ledger Lag | Stat | How many ledgers Horizon is behind the network |
| Request Latency (p50/p95/p99) | Time series | HTTP latency percentiles per route |
| Request Latency Heatmap | Heatmap | Full latency distribution over time |
| DB Connection Pool | Time series | Open, in-use, and idle DB connections |
| DB Query Duration (p50/p95/p99) | Time series | Database query latency percentiles per query type |
| DB Error Rate | Time series | Rate of database errors by type |
| Ingestion Throughput | Time series | Ledgers and transactions ingested per second |
| Ledger Ingestion Duration | Time series | Time taken to ingest each ledger (p50/p95/p99) |

## Prometheus Metrics Required

Horizon must expose the following metrics to Prometheus:

```
horizon_requests_total{method, status_code, route, namespace}
horizon_request_duration_seconds_bucket{route, namespace}
horizon_ingest_ledger_lag{namespace}
horizon_ingest_ledgers_total{namespace}
horizon_ingest_transactions_total{namespace}
horizon_ingest_ledger_duration_seconds_bucket{namespace}
horizon_db_pool_open_connections{namespace}
horizon_db_pool_in_use{namespace}
horizon_db_pool_idle{namespace}
horizon_db_query_duration_seconds_bucket{query_type, namespace}
horizon_db_errors_total{error_type, namespace}
```

Configure Horizon's `--metrics-port` (default `6060`) and add a Prometheus `scrape_config`:

```yaml
scrape_configs:
  - job_name: horizon
    kubernetes_sd_configs:
      - role: pod
    relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_app]
        regex: horizon
        action: keep
      - source_labels: [__meta_kubernetes_pod_annotation_prometheus_io_port]
        target_label: __address__
        replacement: "${1}:6060"
```

## Importing the Dashboard

### Via Grafana UI

1. Open your Grafana instance.
2. Go to **Dashboards → Import**.
3. Click **Upload JSON file** and select `monitoring/grafana-horizon.json`.
4. Select your **Prometheus** data source from the dropdown.
5. Click **Import**.

### Via Grafana API

```bash
curl -X POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $GRAFANA_API_KEY" \
  -d "{\"dashboard\": $(cat monitoring/grafana-horizon.json), \"overwrite\": true, \"folderId\": 0}" \
  http://<grafana-host>/api/dashboards/import
```

### Via Helm (kube-prometheus-stack)

Add the dashboard as a ConfigMap so the Grafana sidecar picks it up automatically:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: grafana-horizon-dashboard
  namespace: monitoring
  labels:
    grafana_dashboard: "1"
data:
  grafana-horizon.json: |
    # paste contents of monitoring/grafana-horizon.json here
```

Or reference the file in your Helm values:

```yaml
grafana:
  dashboardsConfigMaps:
    horizon: grafana-horizon-dashboard
```

## Template Variables

| Variable | Description |
|---|---|
| `datasource` | Prometheus data source to query |
| `namespace` | Kubernetes namespace(s) to filter (supports multi-select) |

## Alerting

Example Prometheus alerting rules for Horizon:

```yaml
groups:
  - name: horizon
    rules:
      - alert: HorizonHighErrorRate
        expr: |
          100 * sum(rate(horizon_requests_total{status_code=~"[45].."}[5m]))
              / sum(rate(horizon_requests_total[5m])) > 5
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Horizon error rate above 5%"

      - alert: HorizonHighLedgerLag
        expr: horizon_ingest_ledger_lag > 20
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Horizon ingestion lagging behind network by {{ $value }} ledgers"

      - alert: HorizonHighP99Latency
        expr: |
          histogram_quantile(0.99,
            sum(rate(horizon_request_duration_seconds_bucket[5m])) by (le)
          ) > 2
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Horizon p99 latency above 2s"

      - alert: HorizonDBPoolExhausted
        expr: horizon_db_pool_idle == 0
        for: 2m
        labels:
          severity: critical
        annotations:
          summary: "Horizon DB connection pool has no idle connections"
```
