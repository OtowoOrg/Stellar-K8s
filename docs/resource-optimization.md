# ML-Based Resource Optimization

Dynamic resource optimization for Stellar-K8s using ML-based workload prediction,
SLA-aware autoscaling, and intelligent VPA recommendations.

## Architecture

```text
┌─────────────────────────────────────────────────────────────────────┐
│                    Resource Optimization Pipeline                    │
│                                                                      │
│  Prometheus ──► Time-Series Collector ──► Forecast Engine (Ensemble) │
│                         │                        │                   │
│                         ▼                        ▼                   │
│                  SLA Evaluator ◄────── Predictive Autoscaler         │
│                         │                        │                   │
│                         ▼                        ▼                   │
│              VPA Optimizer ──► HPA minReplicas Patch                  │
│                         │                                            │
│                         ▼                                            │
│              Optimization Dashboard (REST API)                       │
└─────────────────────────────────────────────────────────────────────┘
```

## Components

| Module | Purpose |
|--------|---------|
| `resource_optimization/forecasting` | Holt-Winters, linear, and ensemble forecasting |
| `resource_optimization/sla` | SLA constraint evaluation and replica adjustment |
| `resource_optimization/vpa_optimizer` | ML-driven vertical pod right-sizing |
| `resource_optimization/simulation` | What-if capacity planning scenarios |
| `resource_optimization/controller` | Predictive autoscaling orchestration |
| `resource_optimization/metrics` | Prometheus metrics for cost, SLA, accuracy |

## Configuration

```yaml
spec:
  resourceOptimization:
    enabled: true
    forecastHorizonMinutes: 60
    predictiveScaling:
      enabled: true
      prometheusUrl: "http://prometheus:9090"
      tpsPerReplica: 1000
    sla:
      maxP99LatencyMs: 500
      minAvailabilityPct: 99.9
    vpaOptimization:
      enabled: true
```

## Metrics

- `stellar_optimization_cost_savings_pct` — estimated cost savings
- `stellar_optimization_sla_compliance` — SLA compliance score (0-100)
- `stellar_optimization_prediction_mape` — forecast accuracy (lower is better)
- `stellar_optimization_cycles_total` — optimization cycles executed

## REST API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/v1/optimization/recommendations` | GET | Current optimization recommendations |
| `/api/v1/optimization/simulate` | POST | Run what-if capacity simulation |
| `/api/v1/optimization/forecast` | GET | Workload forecast preview |

## Operational Considerations

- Requires at least 2 minutes of Prometheus TPS data before generating recommendations.
- SLA violations trigger automatic scale-up with a 25% headroom multiplier.
- VPA recommendations are advisory; apply via `spec.vpaConfig` for automatic enforcement.
