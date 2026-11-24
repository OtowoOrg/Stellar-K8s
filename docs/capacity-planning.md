# Capacity Planning

Stellar-K8s includes a built-in capacity forecasting engine that analyses
historical resource utilisation, predicts exhaustion 90 days out, and publishes
ranked scaling recommendations as a `CapacityRecommendationReport` Custom
Resource.

> For static cost estimates (instance types, monthly spend) see
> [Cost Estimate](cost-estimate.md).
> For cost-optimisation strategies see
> [Cost Optimization Guide](cost-optimization-guide.md).

---

## How It Works

The engine runs on a configurable schedule (default: quarterly) and processes
one or more `UtilizationSeries` — time-series of `(used, capacity)` pairs
sampled from Prometheus.

### Forecasting Models

| Model | When Selected |
|---|---|
| `RobustLinear` | Default; Theil–Sen slope is resistant to outliers |
| `SeasonalLinear` | Selected automatically when a weekly pattern reduces rolling-origin MAPE by ≥ 10 % |

Model selection is validated by a rolling-origin backtest at the configured
horizon. The chosen model and its MAPE are recorded in every report.

### Capacity Dimensions

Each dimension is forecast independently:

| Dimension | Typical Source |
|---|---|
| `Cpu` | `container_cpu_usage_seconds_total` |
| `Memory` | `container_memory_working_set_bytes` |
| `Storage` | `kubelet_volume_stats_used_bytes` |
| `ObjectCount` | `kube_<resource>_count` |

### Confidence Intervals

Every forecast point carries a prediction interval that widens with distance
from the training data. Time-to-exhaustion is reported for three bands:

- `earliestDays` — upper band (pessimistic)
- `expectedDays` — point forecast
- `latestDays` — lower band (optimistic)

---

## CapacityRecommendationReport CRD

The engine publishes results as a namespaced
`CapacityRecommendationReport` (short name `caprec`):

```bash
kubectl get caprec -n stellar-system
```

Example output:

```
NAME                  CYCLE     MODEL                    MAPE   NEXTEXHAUSTIONDAYS   PHASE
capacity-2026-q3      2026-Q3   capacity-forecast/1.0.0  8.3    21                   Pending
```

### Spec Fields

| Field | Description |
|---|---|
| `cycleId` | Identifier of the recommendation cycle, e.g. `2026-Q3` |
| `generatedAt` | RFC 3339 timestamp of the cycle run |
| `modelVersion` | Version string of the forecasting model |
| `horizonDays` | Forecast horizon (default 90) |
| `confidenceLevel` | Two-sided confidence level of every interval (default 0.9) |
| `forecasts[]` | Per-series forecast summaries |
| `recommendations[]` | Scaling recommendations ranked by time-to-exhaustion |
| `backtest` | Rolling-origin accuracy report |

### Status Phases

| Phase | Meaning |
|---|---|
| `Pending` | Report published; no action taken yet |
| `Acknowledged` | Operator has reviewed the recommendations |
| `Applied` | Scaling changes have been applied |
| `Superseded` | A newer cycle has replaced this report |

---

## Default Configuration

```yaml
horizonDays: 90
confidenceLevel: 0.9
minHistoryDays: 30
lookbackDays: 365
backtestStepDays: 7
minLeadTimeDays: 14
seasonalPeriodDays: 7
seasonalMinImprovement: 0.1
exhaustionThreshold: 1.0
targetUtilization: 0.8
mapeTargetPct: 15.0
```

Key parameters:

- `minLeadTimeDays` — recommendations are issued at least this many days before
  predicted exhaustion (default 14). Recommendations that cannot meet this lead
  time are marked `Critical`.
- `targetUtilization` — recommended capacity is sized so the upper forecast band
  stays below this fraction of capacity for the whole horizon (default 0.8 = 80 %).
- `mapeTargetPct` — the backtest must achieve MAPE below this value for
  `mape_target_met` to be `true` (default 15 %).

---

## Configuration File

The forecast parameters, the forecast dimensions, the operator sizing table
and the ledger growth assumptions are also kept in a reviewable file,
`config/capacity-planning.yaml`:

```bash
make capacity-plan          # validate the configuration and print a summary
make capacity-plan-check    # validate the configuration only
```

`make capacity-plan` prints the parameters in use, the forecast dimensions,
the operator resource requests per managed `StellarNode` count, the ledger
growth assumptions, and the alert thresholds that follow from
`minLeadTimeDays`. The target exits non-zero when a value is out of range
(for example a lookback window shorter than the minimum history, or a
`minLeadTimeDays` larger than the horizon), so it can be used as a
configuration gate in CI.

The file mirrors the defaults documented above. Change a value there, then
document the change in this page so the two stay in sync.

---

## Recommendation Priorities

| Priority | Condition |
|---|---|
| `Critical` | Exhaustion predicted within `minLeadTimeDays` (≤ 14 days) |
| `High` | Exhaustion predicted within 30 days |
| `Medium` | Exhaustion predicted within 60 days |
| `Low` | Exhaustion predicted within the horizon (> 60 days) |

---

## Prometheus Metrics

The capacity planning module exports two Prometheus gauges:

```
# Projected resource exhaustion timestamp (Unix epoch)
stellar_capacity_exhaustion_prediction{resource, node_type}

# Confidence score of the prediction (0–100)
stellar_capacity_prediction_confidence{resource, node_type}
```

Example PromQL — days until storage exhaustion for validators:

```promql
(stellar_capacity_exhaustion_prediction{resource="storage", node_type="Validator"}
  - time()) / 86400
```

---

## Alerting Rules

Add these rules to your Prometheus configuration:

```yaml
groups:
  - name: capacity_planning
    rules:
      - alert: CapacityExhaustionCritical
        expr: |
          (stellar_capacity_exhaustion_prediction - time()) / 86400 < 14
        for: 1h
        labels:
          severity: critical
        annotations:
          summary: "{{ $labels.resource }} exhaustion predicted in < 14 days"

      - alert: CapacityExhaustionWarning
        expr: |
          (stellar_capacity_exhaustion_prediction - time()) / 86400 < 30
        for: 1h
        labels:
          severity: warning
        annotations:
          summary: "{{ $labels.resource }} exhaustion predicted in < 30 days"

      - alert: CapacityForecastMAPEHigh
        expr: |
          stellar_capacity_prediction_confidence < 70
        for: 6h
        labels:
          severity: warning
        annotations:
          summary: "Capacity forecast confidence below 70 for {{ $labels.resource }}"
```

---

## Sizing Reference

Use the recommendations in the published `CapacityRecommendationReport` as the
primary source of truth. The table below provides starting-point estimates for
new deployments before sufficient history has accumulated.

### Operator Resource Sizing by Node Count

| StellarNodes | CPU Request | Memory Request |
|---|---|---|
| 0 – 50 | 100 m | 128 Mi |
| 50 – 100 | 250 m | 256 Mi |
| 100 – 200 | 500 m | 512 Mi |
| 200 – 300 | 750 m | 768 Mi |
| 300 – 500 | 1 000 m | 1 Gi |

For clusters with more than 200 `StellarNode` resources, consider namespace
sharding:

```bash
# Namespace A
stellar-operator run --watch-namespace stellar-ns-a

# Namespace B
stellar-operator run --watch-namespace stellar-ns-b
```

### Validator Storage Growth

Stellar ledger data grows at roughly **1–2 GB per day** on Mainnet under normal
transaction volume. The proactive disk-scaling feature expands EBS/GCP volumes
automatically; see [Proactive Disk Scaling](proactive-disk-scaling.md).

| Network | Initial Size | 6-Month Estimate | 12-Month Estimate |
|---|---|---|---|
| Testnet | 100 Gi | ~280 Gi | ~460 Gi |
| Mainnet | 500 Gi | ~860 Gi | ~1.2 Ti |

---

## Running a Manual Cycle

```bash
stellar-operator capacity-forecast \
  --namespace stellar-system \
  --cycle-id 2026-Q4 \
  --horizon-days 90
```

The command publishes a `CapacityRecommendationReport` CR and exits. Use
`--dry-run` to print the report without writing to the cluster.

The forecast parameters used by the command come from
`config/capacity-planning.yaml`; run
`make capacity-plan` to review them without touching a cluster.

---

## References

- [Cost Estimate](cost-estimate.md) — static AWS pricing tables
- [Cost Optimization Guide](cost-optimization-guide.md) — right-sizing, spot, savings plans
- [Proactive Disk Scaling](proactive-disk-scaling.md) — automatic PVC expansion
- [Scalability](scalability.md) — operator benchmarks and sharding patterns
- [API Reference](api-reference.md) — full `CapacityRecommendationReport` CRD schema
