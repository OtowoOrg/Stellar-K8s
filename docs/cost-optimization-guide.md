# Cost Optimization Guide

This guide covers cloud resource cost optimization for Stellar-K8s, including usage tracking, right-sizing recommendations, spot instance support, cost dashboards, and anomaly detection.

## Overview

Stellar-K8s provides automated cost optimization through:

- **Resource usage tracking** with rolling-window utilization metrics
- **Right-sizing recommendations** based on observed vs. requested resources
- **Spot instance support** for non-critical workloads
- **Cost dashboards** with Grafana integration
- **Anomaly detection** for unexpected cost increases

## Configuration

### Helm Values

```yaml
costOptimization:
  resourceTracking:
    enabled: true
    windowSize: 336  # hours (14 days)
    headroomFactor: 1.20
  spot:
    enabled: false
    maxPricePerHourUsd: "0.50"
    fallbackToOnDemand: true
  monthlyBudgetUsd: 0  # Set to >0 to enable budget alerts
```

### Enable Cost Optimization

```bash
helm install stellar-operator ./charts/stellar-operator \
  --set costOptimization.resourceTracking.enabled=true \
  --set costOptimization.spot.enabled=true \
  --set costOptimization.monthlyBudgetUsd=5000
```

## Resource Usage Tracking

The operator continuously monitors CPU and memory utilization for all managed workloads:

- **CPU utilization:** P95 usage over the observation window
- **Memory utilization:** P95 usage over the observation window
- **Request vs. usage:** Comparison of requested vs. actual usage
- **Node utilization:** Overall node resource consumption

### Metrics Collected

| Metric | Description |
|--------|-------------|
| `stellar_resource_cpu_request_m` | CPU request in millicores |
| `stellar_resource_cpu_p95_m` | P95 CPU usage in millicores |
| `stellar_resource_memory_request_bytes` | Memory request in bytes |
| `stellar_resource_memory_p95_bytes` | P95 memory usage in bytes |
| `stellar_resource_utilization_percent` | Overall utilization percentage |

## Right-Sizing Recommendations

The recommendation engine analyzes resource usage patterns and suggests optimal resource allocations:

### How It Works

1. **Observation window:** 336 hours (14 days) of metrics collected
2. **P95 analysis:** 95th percentile usage identified
3. **Headroom factor:** 20% buffer applied to P95 usage
4. **Recommendation:** Suggested request/limit values

### Recommendation Output

```json
{
  "workload": "stellar-validator-0",
  "namespace": "stellar",
  "currentRequest": {
    "cpu": "500m",
    "memory": "1Gi"
  },
  "observedUsage": {
    "cpu": "250m",
    "memory": "512Mi"
  },
  "recommendedRequest": {
    "cpu": "300m",
    "memory": "614Mi"
  },
  "confidence": 0.95,
  "observationWindow": "14d"
}
```

### Apply Recommendations

```bash
# View recommendations
kubectl get stellarnode -A -o json | jq '.items[] | select(.status.recommendations != null)'

# Apply recommendation (manual review required)
kubectl patch stellarnode <name> -n <namespace> --type merge -p '{"spec":{"resources":{"requests":{"cpu":"300m","memory":"614Mi"}}}}'
```

### Safety Controls

- Recommendations are **advisory by default** — no automatic changes
- Critical workloads (validators, databases) are excluded from spot scheduling
- Changes require operator approval before application
- Rollback procedure documented for each recommendation

## Spot Instance Support

Spot instances provide significant cost savings (60-90%) for non-critical workloads.

### Eligible Workloads

| Workload Type | Spot Eligible | Reason |
|---------------|---------------|--------|
| Indexer | Yes | Stateless, can be interrupted |
| Analytics | Yes | Batch processing, resumable |
| Monitoring | Yes | Non-critical, redundant |
| Validator | **No** | Critical consensus component |
| Horizon | **No** | Public API availability |
| PostgreSQL | **No** | Stateful, data integrity critical |

### Configuration

```yaml
costOptimization:
  spot:
    enabled: true
    maxPricePerHourUsd: "0.50"
    fallbackToOnDemand: true
```

### Spot Instance Scheduling

The operator uses Kubernetes scheduling primitives:

```yaml
affinity:
  nodeAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      nodeSelectorTerms:
        - matchExpressions:
            - key: stellar.org/spot-eligible
              operator: In
              values: ["true"]
tolerations:
  - key: "spot"
    operator: "Equal"
    value: "true"
    effect: "NoSchedule"
```

### Interruption Handling

When a spot instance is reclaimed:

1. **Notification received** (2 minutes before reclaim)
2. **Graceful shutdown** initiated
3. **State saved** to persistent storage
4. **Workload rescheduled** to on-demand instance (if `fallbackToOnDemand=true`)
5. **Service recovery** verified

## Cost Dashboard

### Grafana Integration

The cost dashboard is available as a Grafana dashboard ConfigMap:

```bash
kubectl apply -f charts/stellar-operator/dashboards/cost-dashboard.json -n monitoring
```

### Dashboard Panels

| Panel | Metric | Description |
|-------|--------|-------------|
| Total Monthly Cost | `stellar_cost_total_monthly_usd` | Current month's estimated cost |
| Potential Savings | `stellar_cost_potential_savings_usd` | Right-sizing savings opportunity |
| Cost by Namespace | `stellar_cost_namespace_usd` | Cost breakdown by namespace |
| Cost Trend | `stellar_cost_daily_usd` | 30-day cost trend |
| Spot Savings | `stellar_spot_savings_usd` | Savings from spot instances |
| Resource Utilization | `stellar_resource_utilization_percent` | Overall resource efficiency |

## Cost Anomaly Detection

The anomaly detector identifies unexpected cost increases:

### Alert Rules

| Alert | Condition | Severity |
|-------|-----------|----------|
| `StellarCostSpikeDetected` | >20% day-on-day increase | Warning |
| `StellarCostSpikeCritical` | >50% day-on-day increase | Critical |
| `StellarResourceWasteHigh` | >60% over-provisioned | Warning |
| `StellarSpotInterruptionRateHigh` | >0.1 interruptions/sec | Warning |
| `StellarMonthlyBudgetExceeded` | Monthly budget exceeded | Critical |

### Enable Cost Alerts

```yaml
monitoring:
  enabled: true
  prometheusRule:
    enabled: true
    costAlerts:
      enabled: true
```

## Budget Management

### Set Monthly Budget

```yaml
costOptimization:
  monthlyBudgetUsd: 5000
```

### Budget Alert

When `stellar_cost_total_monthly_usd > stellar_cost_budget_monthly_usd`, the `StellarMonthlyBudgetExceeded` alert fires.

## Best Practices

1. **Review recommendations monthly** — apply safe right-sizing changes
2. **Enable spot for non-critical workloads** — maximize cost savings
3. **Set budget alerts** — prevent unexpected cost overruns
4. **Monitor utilization** — identify idle resources
5. **Document changes** — track cost optimization actions

## Safety Controls

- **No automatic production changes** — all recommendations require approval
- **Critical workload protection** — validators, databases excluded from spot
- **Rollback procedures** — documented for all optimization changes
- **Budget guards** — alerts prevent unauthorized spending
- **Audit trail** — all cost optimization actions logged

## References

- [OpenCost documentation](https://www.opencost.io/docs/)
- [Kubecost documentation](https://docs.kubecost.com/)
- [Kubernetes resource management](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/)
