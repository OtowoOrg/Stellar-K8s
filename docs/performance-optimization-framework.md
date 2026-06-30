# Performance Optimization Framework (#868)

Implements the foundation of the Performance Optimization Framework epic: the
`StellarPerformance` CRD plus the operator logic that evaluates workloads
against **performance budgets (SLOs)** and detects **regressions** against a
rolling baseline.

## StellarPerformance CRD

`StellarPerformance` (`stellar.org/v1alpha1`, shortname `sperf`) governs the
performance of a target workload.

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarPerformance
metadata:
  name: horizon-prod-perf
  namespace: stellar
spec:
  targetRef: horizon-prod          # StellarNode (or service) under governance
  evaluationIntervalSeconds: 60
  budgets:
    maxP95LatencyMs: 200           # upper bound on p95 API latency
    minThroughputTps: 100          # lower bound on sustained throughput
    maxErrorRatePct: 1.0           # upper bound on error rate
  regression:
    maxLatencyIncreasePct: 10      # flag if p95 rises >10% over baseline
    maxThroughputDecreasePct: 10   # flag if throughput drops >10% under baseline
```

### Status

```yaml
status:
  phase: WithinBudget              # Pending | WithinBudget | BudgetExceeded | Regressed
  regressionDetected: false
  current:   { p95LatencyMs: 120, throughputTps: 150, errorRatePct: 0.2 }
  baseline:  { p95LatencyMs: 118, throughputTps: 152, errorRatePct: 0.2 }
  budgetCompliance:
    - { metric: p95LatencyMs,  withinBudget: true, observed: 120, budget: 200 }
    - { metric: throughputTps, withinBudget: true, observed: 150, budget: 100 }
    - { metric: errorRatePct,  withinBudget: true, observed: 0.2, budget: 1.0 }
```

## Controller logic

`src/controller/performance/` holds pure, unit-tested functions:

- `evaluate_budgets` — one `BudgetResult` per SLO (latency/error-rate are
  upper-bounded, throughput is lower-bounded; thresholds are inclusive).
- `detect_regression` — compares the current sample to the rolling baseline
  under the `RegressionPolicy`.
- `update_baseline` — exponentially-weighted moving average (alpha = 0.2) so a
  short spike does not poison the baseline; the first sample seeds it.
- `derive_phase` — a detected regression dominates a budget breach, because it
  signals a trend even while absolute values remain nominally within budget.

Keeping these side-effect-free lets the decision logic be tested without a
live cluster; a reconciler wires them to the metrics source and writes status.

## Scope and follow-up

This slice delivers the CRD, budget evaluation, regression detection, rolling
baseline, and phase derivation. The remaining epic capabilities build on this
foundation and are tracked as follow-up work:

- Continuous benchmarking on every deployment (reuses `StellarBenchmark`).
- Continuous CPU/memory/I-O profiling (e.g. Pyroscope).
- Slow-query detection and index recommendations.
- Multi-tier caching (L1/L2/CDN) with hit-rate targets.
- Automated resource tuning and load testing (e.g. k6).
