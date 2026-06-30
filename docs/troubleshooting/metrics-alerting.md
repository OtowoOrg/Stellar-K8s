# Metrics and Dashboard Requirements

> Core metrics that must be visible in dashboards and tied to actionable alerts.

---

## 1) Mandatory Metrics

| Metric | Meaning | Alert Condition |
|---|---|---|
| `stellar_k8s_operator_reconciliation_duration_seconds` | Controller reconcile latency distribution | p95 > 5s for 15m |
| `stellar_k8s_operator_reconciliation_errors_total` | Reconcile error volume | rate increase > baseline burn |
| `stellar_core_ledger_age` | Seconds since latest ledger close | > 20s sustained |
| `stellar_core_peer_count` | Active peer connections | below quorum expectation |
| `process_resident_memory_bytes` | Process memory pressure | rapid slope + OOM events |

## 2) PromQL Alert Examples

```promql
histogram_quantile(
  0.95,
  sum(rate(stellar_k8s_operator_reconciliation_duration_seconds_bucket[5m])) by (le)
) > 5
```

```promql
max(stellar_core_ledger_age{role="validator"}) > 20
```

```promql
sum(rate(stellar_k8s_operator_reconciliation_errors_total[5m])) > 0.2
```

## 3) Dashboard Panels

1. Operator Health: reconcile p50/p95/p99, errors, queue depth.
2. Validator Health: ledger age, peer count, SCP success/failure counters.
3. RPC Health: request rate, error ratio, p95 latency, saturation.
4. Infrastructure: node network drops, disk IO latency, CPU throttling.

## 4) Alert Triage Rules

- If reconcile latency and errors spike together, inspect CRD webhook latency and API server health.
- If ledger age rises without operator errors, pivot to network, disk, or peer reachability.
- If RPC latency rises with stable ledger age, isolate API or database bottlenecks.
