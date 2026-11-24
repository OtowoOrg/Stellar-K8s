# Automated Load Testing & Performance Budgets

Issue #1336. Workflow:
[`.github/workflows/load-test.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/load-test.yml).

## Why a second performance gate

`benchmarks/scripts/compare_benchmarks.py` answers *"did this get slower than
last time?"* — a relative question. It cannot tell you whether the absolute
numbers were ever acceptable: two 8% regressions in a row pass a 10% threshold
while doubling latency over a month.

`check_performance_budgets.py` answers the absolute question: does this run
meet the SLO targets? Both gates run; they catch different failures.

The existing performance pipeline also only ran on pushes to `main`. This
workflow runs on a **schedule** (04:00 UTC daily), so a regression is caught
even in a quiet week.

## Performance budgets

All SLO targets live in one reviewable file,
[`benchmarks/performance-budgets.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/benchmarks/performance-budgets.yaml),
rather than scattered across five k6 scripts' `options.thresholds`.

The k6 in-script thresholds are deliberately kept: they abort a run early,
while these budgets produce the tracked, reported verdict afterwards.

```yaml
suites:
  operator:
    budgets:
      - metric: metrics.http_req_duration.p95   # dotted path into the k6 summary
        comparison: lt                          # lt | lte | gt | gte
        budget: 500
        unit: ms
        blocking: true                          # false → advisory, reported only
        description: Why this target exists.
```

### Current targets

| Suite | Metric | Budget | Gate |
|---|---|---|---|
| operator | `http_req_duration.p95` | < 500 ms | blocking |
| operator | `http_req_duration.p99` | < 1000 ms | blocking |
| operator | `reconciliation_duration.p95` | < 3000 ms | blocking |
| operator | `reconciliation_duration.p99` | < 5000 ms | advisory |
| operator | `api_latency.p95` | < 200 ms | blocking |
| operator | `error_rate` | < 0.01 | blocking |
| operator | `tps.avg` | ≥ 100 req/s | blocking |
| webhook | `validation_p99` / `mutation_p99` | < 50 ms | blocking |
| webhook | `validation_p95` / `mutation_p95` | < 30 ms | blocking |
| webhook | `throughput` | ≥ 100 req/s | blocking |
| webhook | `error_rate` | < 0.001 | blocking |

The webhook budgets are ten times stricter on error rate because the webhook
sits in the API server's synchronous admission path — a failing admission
webhook blocks writes to the whole cluster.

`reconciliation_duration.p99` is advisory rather than blocking: p99 reconcile
time is sensitive to CI runner noise and would make the gate flaky.

## Failure modes the gate handles

A **missing metric** fails a blocking budget. Silently passing because a
metric vanished from the run summary is exactly how a performance gate stops
protecting anything.

A **throughput floor** uses `gte`, so a change that trades throughput for
latency still fails rather than looking like an improvement.

## Running it

```bash
# Locally, against an existing k6 summary
python3 benchmarks/scripts/check_performance_budgets.py \
  --results results/benchmark-summary.json \
  --suite operator

# Markdown for a CI job summary, plus a tracked history line
python3 benchmarks/scripts/check_performance_budgets.py \
  --results results/benchmark-summary.json \
  --suite operator \
  --markdown "$GITHUB_STEP_SUMMARY" \
  --history benchmarks/history

# Machine-readable
python3 benchmarks/scripts/check_performance_budgets.py \
  --results results/webhook-benchmark.json --suite webhook --format json
```

Exit codes: `0` all blocking budgets met, `1` a blocking budget violated,
`2` bad invocation. `--warn-only` always exits 0.

### On demand in CI

Actions → **Load Test & Performance Budgets** → *Run workflow*, choosing the
suite and optionally `warn_only`.

## CI reporting and tracking over time

- The budget table is written to the **job summary**, so the verdict is visible
  on the run page without opening logs — measured value, budget, headroom
  percentage, and blocking/advisory for every metric.
- Blocking violations are emitted as `::error` annotations.
- `--history` appends one JSON line per run to `results/history/<suite>.jsonl`,
  giving an appendable series with no database.
- Results are uploaded as an artifact with **90-day retention**, covering a
  full quarter of trend analysis.

## Verification

```bash
python3 -m unittest benchmarks.scripts.test_check_performance_budgets
```

41 tests. The ones that speak to this issue's acceptance criteria:

| Test | Proves |
|---|---|
| `a_missing_blocking_metric_fails` | The gate cannot silently become a no-op |
| `an_advisory_violation_is_reported_but_passes` | Advisory budgets do not block |
| `gte_enforces_a_throughput_floor` | Comparison direction is respected |
| `webhook_latency_budget_is_tighter_than_the_operator_one` | The stricter admission budget cannot be loosened to match |
| `error_rate_budgets_are_ratios_not_percentages` | Catches a `1` vs `0.01` mix-up |
| `every_budget_declares_a_description` | Every target explains itself |
| `history_appends_one_line_per_run` | Results tracked over time |
| `a_violating_run_exits_one` | Regressions actually fail CI |

End to end, against the real budgets file:

```bash
cat > /tmp/run.json <<'JSON'
{"metrics":{"tps":{"avg":150},
 "http_req_duration":{"p95":300,"p99":800},
 "reconciliation_duration":{"p95":2000,"p99":4000},
 "api_latency":{"p95":120},"error_rate":0.002}}
JSON
python3 benchmarks/scripts/check_performance_budgets.py \
  --results /tmp/run.json --suite operator; echo "exit=$?"   # → 0

# Break one budget
sed -i 's/"error_rate":0.002/"error_rate":0.5/' /tmp/run.json
python3 benchmarks/scripts/check_performance_budgets.py \
  --results /tmp/run.json --suite operator; echo "exit=$?"   # → 1
```
