# Monitoring and Dashboards Operational Runbook

This runbook provides procedures for operators to verify, maintain, and troubleshoot the Stellar-K8s monitoring stack.

## Table of Contents

- [Pre-flight Checks](#pre-flight-checks)
- [Health Verification](#health-verification)
- [Common Issues](#common-issues)
- [Maintenance Procedures](#maintenance-procedures)
- [Escalation Guide](#escalation-guide)

## Pre-flight Checks

### 1. Verify Operator Metrics Export

```bash
# Check operator pod is running
kubectl get pods -n stellar-system -l app.kubernetes.io/name=stellar-operator
# Expected: 1/1 Running

# Port-forward to operator metrics
kubectl port-forward -n stellar-system svc/stellar-operator 9090:9090 &

# Verify metrics endpoint
curl http://localhost:9090/metrics | head -20
# Expected: HELP, TYPE lines for stellar_* metrics

# Kill port-forward
pkill -f "port-forward"
```

### 2. Verify Prometheus Scraping

```bash
# Check ServiceMonitor exists
kubectl get servicemonitor -n stellar-system
# Expected: stellar-operator ServiceMonitor present

# Check PrometheusRule
kubectl get prometheusrule -n monitoring
# Expected: Rules configured

# Check Prometheus targets
kubectl port-forward -n monitoring svc/kube-prom-stack-prometheus 9090:9090 &
curl http://localhost:9090/api/v1/targets | jq '.data.activeTargets[] | select(.labels.job=="stellar-operator")'
# Expected: stellar-operator in active targets
pkill -f "port-forward"
```

### 3. Verify Grafana Dashboards

```bash
# Check Grafana is running
kubectl get pods -n monitoring -l app.kubernetes.io/name=grafana
# Expected: 1/1 Running

# Check dashboard ConfigMaps
kubectl get configmap -n monitoring -l grafana_dashboard=1
# Expected: Several dashboard ConfigMaps

# Verify Grafana can access Prometheus
kubectl port-forward -n monitoring svc/kube-prom-stack-grafana 3000:80 &
# Navigate to http://localhost:3000/api/datasources
# Expected: Prometheus datasource in Active state
pkill -f "port-forward"
```

## Health Verification

### Dashboard Health Check Endpoint

```bash
# Check monitoring status
kubectl port-forward -n stellar-system svc/stellar-operator 8080:8080 &
curl http://localhost:8080/api/v1/dashboard/monitoring-status | jq .
# Expected output:
# {
#   "healthy": true,
#   "metricsEndpointReachable": true,
#   "operatorMetricsAvailable": true,
#   "lastMetricsScrape": "2026-08-30T...",
#   "totalMetricsCollected": 64,
#   "dashboardStatus": {
#     "grafanaAvailable": true,
#     "prometheusAvailable": true,
#     "dashboardsLoaded": 5
#   }
# }
pkill -f "port-forward"
```

### Metrics Collection Verification

```bash
# Verify metrics are being scraped
kubectl port-forward -n monitoring svc/kube-prom-stack-prometheus 9090:9090 &

# Check metric count (sample 1000 random metrics)
curl -s 'http://localhost:9090/api/v1/query' \
  --data-urlencode 'query=count({__name__=~"stellar.*"})' | jq '.data.result[0].value[1]'
# Expected: Non-zero metric count

pkill -f "port-forward"
```

### Alert Rules Verification

```bash
# Verify alert rules are loaded
kubectl port-forward -n monitoring svc/kube-prom-stack-prometheus 9090:9090 &
curl -s http://localhost:9090/api/v1/rules | jq '.data.groups[] | select(.file=="/etc/prometheus/rules.yaml") | .rules | length'
# Expected: > 0 (number of rules loaded)

# Check if any alerts are firing
curl -s http://localhost:9090/api/v1/alerts | jq '.data.alerts[] | select(.state=="firing")'
pkill -f "port-forward"
```

## Common Issues

### Issue: No Metrics Appearing in Prometheus

**Symptoms**: Prometheus targets show `Down` or missing stellar-operator

**Resolution**:

```bash
# 1. Check operator is running and healthy
kubectl get pods -n stellar-system -l app.kubernetes.io/name=stellar-operator
kubectl logs -n stellar-system -l app.kubernetes.io/name=stellar-operator | grep -i "metrics"

# 2. Verify metrics endpoint is accessible
kubectl port-forward -n stellar-system svc/stellar-operator 9090:9090 &
curl -v http://localhost:9090/metrics
# Look for: 200 OK response

# 3. Restart Prometheus to force re-scraping
kubectl rollout restart statefulset/kube-prom-stack-prometheus -n monitoring

# 4. Check ServiceMonitor labels match Prometheus selector
kubectl get servicemonitor -n stellar-system -o yaml | grep -A 5 "labels:"
kubectl get prometheus -n monitoring -o yaml | grep -A 5 "serviceMonitorSelector:"
```

### Issue: Grafana Dashboards Not Loading

**Symptoms**: Empty dashboards or dashboard list shows no Stellar dashboards

**Resolution**:

```bash
# 1. Verify dashboards are in ConfigMaps
kubectl get configmap -n monitoring -l grafana_dashboard=1 --show-labels

# 2. Check Grafana logs for errors
kubectl logs -n monitoring -l app.kubernetes.io/name=grafana | grep -i error

# 3. Verify dashboard provisioning path
kubectl exec -n monitoring -it $(kubectl get pods -n monitoring -l app.kubernetes.io/name=grafana -o name | head -1) -- \
  ls -la /etc/grafana/dashboards/stellar/

# 4. Restart Grafana
kubectl rollout restart deployment/kube-prom-stack-grafana -n monitoring
```

### Issue: High CPU/Memory Usage

**Symptoms**: Prometheus or Grafana using excessive resources

**Resolution**:

```bash
# Check current resource usage
kubectl top pods -n monitoring

# Reduce Prometheus retention
helm upgrade monitoring prometheus-community/kube-prometheus-stack \
  -n monitoring \
  --set prometheus.prometheusSpec.retention=7d

# Increase resource requests
helm upgrade monitoring prometheus-community/kube-prometheus-stack \
  -n monitoring \
  --set prometheus.prometheusSpec.resources.requests.memory=2Gi \
  --set prometheus.prometheusSpec.resources.limits.memory=4Gi

# Reduce scrape frequency
kubectl get servicemonitor -n stellar-system -o yaml | sed 's/interval: 30s/interval: 60s/' | kubectl apply -f -
```

### Issue: Alerts Not Firing

**Symptoms**: No alerts despite issues in cluster

**Resolution**:

```bash
# 1. Verify AlertManager is running
kubectl get pods -n monitoring -l app.kubernetes.io/name=alertmanager

# 2. Check alert rule evaluation
kubectl port-forward -n monitoring svc/kube-prom-stack-prometheus 9090:9090 &
curl -s http://localhost:9090/api/v1/rules | jq '.data.groups[].rules[] | select(.state=="firing")'

# 3. Verify AlertManager config
kubectl get configmap -n monitoring alertmanager-kube-prom-alertmanager -o jsonpath='{.data.alertmanager\.yml}'

# 4. Test alert manually
kubectl exec -n monitoring kube-prom-stack-prometheus-0 -- \
  amtool alert query severity=critical
pkill -f "port-forward"
```

## Maintenance Procedures

### Daily Checks

```bash
#!/bin/bash
# Save as check-monitoring.sh and run daily via cron

namespace=${1:-monitoring}
operator_ns=${2:-stellar-system}

echo "=== Monitoring Status Check ==="
kubectl get pods -n "$namespace" | grep -E "prometheus|grafana|alertmanager"
kubectl get pods -n "$operator_ns" -l app.kubernetes.io/name=stellar-operator

echo "=== Metrics Endpoint ==="
kubectl port-forward -n "$operator_ns" svc/stellar-operator 9090:9090 &
PF_PID=$!
sleep 2
curl -s http://localhost:9090/metrics | grep "stellar_ledger" | head -3
kill $PF_PID 2>/dev/null

echo "=== Prometheus Health ==="
kubectl port-forward -n "$namespace" svc/kube-prom-stack-prometheus 9090:9090 &
PF_PID=$!
sleep 2
curl -s http://localhost:9090/-/healthy || echo "UNHEALTHY"
kill $PF_PID 2>/dev/null
```

### Weekly Maintenance

```bash
# Backup current dashboards
for cm in $(kubectl get configmap -n monitoring -l grafana_dashboard=1 -o name); do
  kubectl get "$cm" -n monitoring -o yaml > "backup-${cm##*/}.yaml"
done

# Verify data retention
kubectl get prometheus -n monitoring -o jsonpath='{.items[0].spec.retention}'

# Check disk usage
kubectl exec -n monitoring kube-prom-stack-prometheus-0 -- \
  du -sh /prometheus
```

### Monthly Maintenance

```bash
# Update dashboard definitions
helm repo update prometheus-community
helm upgrade monitoring prometheus-community/kube-prometheus-stack \
  -n monitoring \
  -f values-monitoring.yaml

# Verify all metrics are still being collected
kubectl port-forward -n monitoring svc/kube-prom-stack-prometheus 9090:9090 &
PF_PID=$!
sleep 2

# Sample 10 random stellar metrics
curl -s 'http://localhost:9090/api/v1/query' \
  --data-urlencode 'query={__name__=~"stellar.*"}' | \
  jq '.data.result | length'

kill $PF_PID 2>/dev/null
```

## Escalation Guide

### When to Page On-Call

**CRITICAL - Page Immediately**:
- Prometheus down > 5 minutes (no metrics collection)
- Quorum integrity alert firing
- Ledger close time > 30 seconds
- Network partition detected

**HIGH - Page within 30 min**:
- Grafana unavailable
- AlertManager not routing alerts
- Peer connections < 3 on validator
- Database slow queries > 10% of total

**MEDIUM - Create ticket**:
- Metrics lag > 5 minutes
- Single dashboard failing to load
- Storage usage > 80% of limit
- Scrape errors on non-critical endpoints

### Escalation Process

```
1. Check monitoring-status endpoint
   curl http://<operator>/api/v1/dashboard/monitoring-status

2. Verify Prometheus is scraping
   kubectl port-forward -n monitoring svc/prometheus 9090:9090
   Check http://localhost:9090/targets

3. Check AlertManager
   kubectl logs -n monitoring alertmanager-* | tail -50

4. If unresolved in 15 min, page SRE on-call
   - Include monitoring status JSON
   - Include relevant log excerpts
   - Include last 10 Prometheus alerts
```

## Verification Checklist

Use this before declaring monitoring "healthy":

```
☐ Operator metrics endpoint responding (HTTP 200)
☐ Prometheus ingesting metrics (> 100 samples)
☐ All target endpoints in UP state
☐ PrometheusRule loaded (grep "stellar:* " rules)
☐ Grafana accessible (HTTP 200 at /api/health)
☐ All dashboards visible in Grafana UI
☐ AlertManager routing alerts correctly
☐ No critical alerts firing (except intentional tests)
☐ Retention policy set appropriately
☐ Storage usage < 80% of allocation
☐ Disk space available on monitoring nodes
```

## References

- [Monitoring Setup Guide](docs/MONITORING_SETUP_GUIDE.md)
- [Stellar Metrics Guide](docs/metrics/STELLAR_METRICS_GUIDE.md)
- [Prometheus Operator Docs](https://prometheus-operator.dev/)
- [Grafana Docs](https://grafana.com/docs/grafana/latest/)

