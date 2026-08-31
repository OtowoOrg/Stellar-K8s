# Comprehensive Metrics and Monitoring Dashboard Setup Guide

This guide covers end-to-end setup for metrics collection, Grafana dashboards, and alerting for Stellar-K8s clusters.

## Table of Contents

- [Overview](#overview)
- [Prerequisites](#prerequisites)
- [Local Development Setup](#local-development-setup)
- [Production Deployment](#production-deployment)
- [Dashboard Configuration](#dashboard-configuration)
- [Alerting](#alerting)
- [Troubleshooting](#troubleshooting)

## Overview

The Stellar-K8s monitoring stack consists of:

- **Prometheus** — Metrics scraping and time-series storage
- **Grafana** — Visualization and dashboarding
- **AlertManager** — Alert routing and notification
- **Stellar Operator** — Built-in metrics exporter at `/metrics`

All components are Kubernetes-native and integrate seamlessly with the operator deployment.

## Prerequisites

- Kubernetes 1.24+
- `kubectl` configured with cluster access
- Helm 3.0+ (for production deployments)
- 2GB+ available memory for monitoring stack

## Local Development Setup

### Option 1: Docker Compose (Fastest)

```bash
# Start Prometheus + Grafana with Stellar operator
docker-compose -f docker-compose.yml -f docker-compose.monitoring.yml up -d

# Wait for services to be ready
sleep 10

# Access dashboards
echo "Grafana:     http://localhost:3000 (admin/admin)"
echo "Prometheus:  http://localhost:9090"
echo "Operator:    http://localhost:8080"
```

### Option 2: Manual Kubernetes Setup

1. Install Prometheus stack (if not already installed):

```bash
# Add Prometheus community Helm repo
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts
helm repo update

# Install kube-prometheus-stack
helm install monitoring prometheus-community/kube-prometheus-stack \
  --namespace monitoring \
  --create-namespace \
  --set prometheus.prometheusSpec.retention=30d \
  --set grafana.adminPassword=admin
```

2. Deploy the operator with monitoring enabled:

```bash
helm install stellar-operator ./charts/stellar-operator \
  --namespace stellar-system \
  --create-namespace \
  --set monitoring.enabled=true \
  --set monitoring.serviceMonitor.enabled=true
```

3. Verify ServiceMonitor is created:

```bash
kubectl get servicemonitor -n stellar-system
# NAME                                          AGE
# stellar-operator                              2m
```

## Production Deployment

### Helm Chart Configuration

1. Create `values-monitoring.yaml`:

```yaml
# Enable monitoring components
monitoring:
  enabled: true
  
  # ServiceMonitor for Prometheus scraping
  serviceMonitor:
    enabled: true
    interval: 30s
    scrapeTimeout: 10s
    labels:
      release: monitoring  # Match your Prometheus release label selector
  
  # PrometheusRule for alerting
  prometheusRule:
    enabled: true
    labels:
      release: monitoring
  
  # Grafana dashboards
  grafanaDashboards:
    enabled: true

# Expose metrics securely
service:
  ports:
    metrics:
      port: 9090
      targetPort: 9090
      protocol: TCP
  annotations:
    prometheus.io/scrape: "true"
    prometheus.io/port: "9090"
    prometheus.io/path: "/metrics"
```

2. Deploy with monitoring:

```bash
helm install stellar-operator ./charts/stellar-operator \
  -f values-monitoring.yaml \
  --namespace stellar-system \
  --create-namespace
```

### Prometheus Configuration

Update your Prometheus `prometheus.yaml` to scrape Stellar nodes:

```yaml
global:
  scrape_interval: 30s
  evaluation_interval: 30s

scrape_configs:
  - job_name: 'stellar-operator'
    kubernetes_sd_configs:
      - role: pod
        namespaces:
          names:
            - stellar-system
    relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_app_kubernetes_io_name]
        regex: stellar-operator
        action: keep
      - source_labels: [__meta_kubernetes_pod_container_port_name]
        regex: metrics
        action: keep
```

## Dashboard Configuration

### Import Pre-built Dashboards

1. **Via Grafana UI**:

```bash
# Get Grafana admin password
kubectl get secret -n monitoring kube-prom-stack-grafana -o jsonpath='{.data.admin-password}' | base64 -d

# Port-forward to Grafana
kubectl port-forward -n monitoring svc/kube-prom-stack-grafana 3000:80

# Open http://localhost:3000
# Navigate: Dashboards → Import
# Upload files from: monitoring/grafana-*.json
```

2. **Via ConfigMap (GitOps)**:

```bash
# Create ConfigMap for each dashboard
for dashboard in monitoring/grafana-*.json; do
  kubectl create configmap "$(basename "$dashboard" .json)" \
    --from-file="$dashboard" \
    -n monitoring
  kubectl label configmap "$(basename "$dashboard" .json)" \
    grafana_dashboard=1 \
    -n monitoring
done
```

### Available Dashboards

| Dashboard | Purpose | Metrics |
|-----------|---------|---------|
| `grafana-validator-dashboard.json` | Validator node health | Ledger close time, TPS, peer connections |
| `grafana-horizon-dashboard.json` | Horizon API performance | Request latency, ingestion lag |
| `grafana-soroban-rpc-dashboard.json` | Soroban RPC metrics | Contract execution time, WASM cache |
| `grafana-operator-health-dashboard.json` | Operator reconciliation | Reconciliation duration, error rate |
| `grafana-cost-dashboard.json` | Cost optimization | Resource utilization, spending trends |

### Custom Dashboard Creation

1. Open Grafana and create new dashboard
2. Add panels using PromQL queries:

```promql
# Example: Ledger close time p99
histogram_quantile(0.99, rate(stellar_ledger_close_time_seconds_bucket[5m]))

# Example: Transaction throughput
rate(stellar_ledger_transactions_total[5m])

# Example: Peer connection health
stellar_peer_connection_count > 0
```

3. Save dashboard and export JSON:
   - Dashboard → Save as
   - Panel menu → More → Export JSON
   - Commit to `monitoring/` directory

## Alerting

### Enable AlertManager

1. Configure AlertManager in your Prometheus stack values:

```yaml
alertmanager:
  enabled: true
  config:
    global:
      resolve_timeout: 5m
    route:
      receiver: 'default'
      group_by: ['alertname', 'cluster', 'service']
      group_wait: 10s
      group_interval: 10s
      repeat_interval: 12h
    receivers:
      - name: 'default'
        # Configure your notification channels (Slack, PagerDuty, etc.)
        slack_configs:
          - api_url: YOUR_SLACK_WEBHOOK_URL
            channel: '#alerts'
            title: '{{ .GroupLabels.alertname }}'
```

2. Critical alerts configured (from `monitoring/prometheus-rules.yaml`):

```yaml
# Ledger close time alert
- alert: HighLedgerCloseTime
  expr: histogram_quantile(0.99, stellar_ledger_close_time_seconds_bucket) > 10
  for: 5m
  annotations:
    summary: "High ledger close time on {{ $labels.node_name }}"

# Peer connection alert
- alert: LowPeerConnections
  expr: stellar_peer_connection_count < 3
  for: 10m
  annotations:
    summary: "Node {{ $labels.node_name }} has low peer connections"

# Quorum integrity alert
- alert: QuorumIntersectionFailure
  expr: rate(stellar_scp_quorum_intersection_failures_total[5m]) > 0
  for: 1m
  annotations:
    summary: "Quorum intersection failure on {{ $labels.node_name }}"
```

## Troubleshooting

### Metrics Not Appearing in Prometheus

1. Verify operator is exporting metrics:

```bash
# Port-forward to operator metrics
kubectl port-forward -n stellar-system svc/stellar-operator 9090:9090

# Check metrics endpoint
curl http://localhost:9090/metrics | grep stellar_
```

2. Check ServiceMonitor:

```bash
# Verify ServiceMonitor exists
kubectl get servicemonitor -n stellar-system

# Check PrometheusRule:
kubectl get prometheusrule -n monitoring
```

3. Restart Prometheus:

```bash
kubectl rollout restart statefulset/kube-prom-stack-prometheus -n monitoring
```

### Dashboards Not Loading

1. Verify ConfigMaps are labeled correctly:

```bash
kubectl get configmap -n monitoring -l grafana_dashboard=1
```

2. Check Grafana pod logs:

```bash
kubectl logs -n monitoring -l app.kubernetes.io/name=grafana
```

3. Restart Grafana:

```bash
kubectl rollout restart deployment/kube-prom-stack-grafana -n monitoring
```

### High Memory Usage

Reduce Prometheus retention and increase resource limits:

```bash
helm upgrade monitoring prometheus-community/kube-prometheus-stack \
  --namespace monitoring \
  --set prometheus.prometheusSpec.retention=7d \
  --set prometheus.prometheusSpec.resources.requests.memory=1Gi
```

## Performance Tuning

### Scrape Interval Optimization

- **Development**: 60s (low overhead, slower alerting)
- **Production**: 30s (standard)
- **High-load**: 15s (requires more resources)

### Retention Policy

```bash
# Keep 30 days of metrics (default)
--storage.tsdb.retention.time=30d

# Keep 50GB of data
--storage.tsdb.retention.size=50GB
```

### Recording Rules

Pre-compute expensive queries with recording rules:

```yaml
groups:
  - name: stellar.rules
    interval: 30s
    rules:
      - record: stellar:ledger:close_time:p99
        expr: histogram_quantile(0.99, rate(stellar_ledger_close_time_seconds_bucket[5m]))
      
      - record: stellar:transaction:throughput:5m
        expr: rate(stellar_ledger_transactions_total[5m])
```

## Next Steps

- [Metrics Guide](docs/metrics/STELLAR_METRICS_GUIDE.md) — Detailed metric documentation
- [SCP Topology Monitoring](docs/scp-consensus-topology-and-monitoring.md) — Quorum visualization
- [Byzantine Monitoring](docs/byzantine-monitoring.md) — Network partition detection
- [Cost Optimization](docs/cost-optimization-guide.md) — Resource tracking

