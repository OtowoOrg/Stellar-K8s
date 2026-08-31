# Benchmark Methodology and Results

This document describes the benchmarking approach, methodology, and how to interpret results.

## Table of Contents

- [Benchmark Suite Overview](#benchmark-suite-overview)
- [CRD Validation Benchmarks](#crd-validation-benchmarks)
- [Helm Rendering Benchmarks](#helm-rendering-benchmarks)
- [Operator API Benchmarks](#operator-api-benchmarks)
- [Reconciliation Benchmarks](#reconciliation-benchmarks)
- [Running Benchmarks](#running-benchmarks)
- [Regression Detection](#regression-detection)
- [Interpreting Results](#interpreting-results)

## Benchmark Suite Overview

Stellar-K8s includes four benchmark suites to detect performance regressions:

| Suite | Focus | Threshold |
|-------|-------|-----------|
| CRD Validation | Manifest processing speed | 15% regression |
| Helm Rendering | Template compilation | 20% regression |
| Operator API | REST endpoint throughput | 25% latency, 5% throughput |
| Reconciliation | Resource sync latency | 30% regression |

### Baseline Storage

Baselines are stored in `benchmarks/baselines/`:

```
benchmarks/baselines/
├── crd-performance-v0.1.0.json
├── helm-rendering-v0.1.0.json
├── operator-api-v0.1.0.json
└── reconciliation-v0.1.0.json
```

Each baseline captures:
- Version and timestamp
- Test environment (CPU, memory, OS)
- Metric values (throughput, latencies, percentiles)
- Regression thresholds

## CRD Validation Benchmarks

### What We Measure

- **Throughput:** Manifests processed per second
- **Latency:** Average, P95, P99 per-manifest validation time

### Benchmark Definition

```rust
fn bench_crd_validation(manifests: Vec<String>) {
    let start = Instant::now();
    let mut timings = vec![];
    
    for manifest in manifests {
        let t0 = Instant::now();
        validate_manifest(&manifest)?;
        timings.push(t0.elapsed());
    }
    
    CrdValidationResult::new(
        manifests.len(),
        start.elapsed().as_secs_f64(),
        &timings,
    )
}
```

### Test Cases

| Case | Count | Size | Scenario |
|------|-------|------|----------|
| Simple CRDs | 100 | ~2KB | Single resource |
| Complex CRDs | 100 | ~10KB | Full spec with defaults |
| Large Batch | 500 | ~5KB | Typical production load |
| Mixed | 300 | varies | Realistic mix |

### Baseline (v0.1.0)

```json
{
  "total_manifests": 500,
  "duration_secs": 2.45,
  "throughput_per_sec": 204.1,
  "average_validation_ms": 4.9,
  "p95_validation_ms": 7.2,
  "p99_validation_ms": 9.1
}
```

**Thresholds:**
- Average validation time: 4.9ms ± 15% = 4.17-5.64ms (warning)
- P99 latency: 9.1ms ± 15% = 7.74-10.47ms (warning)

## Helm Rendering Benchmarks

### What We Measure

- **Throughput:** Templates rendered per second
- **Latency:** Time to render each template
- **Output size:** Total rendered manifest bytes

### Benchmark Definition

```rust
fn bench_helm_rendering(values_set: Vec<HelmValues>) {
    let start = Instant::now();
    let mut timings = vec![];
    let mut rendered_bytes = 0;
    
    for values in values_set {
        let t0 = Instant::now();
        let rendered = helm_template("stellar-operator", &values)?;
        timings.push(t0.elapsed());
        rendered_bytes += rendered.len();
    }
    
    HelmRenderingResult::new(
        "stellar-operator",
        values_set.len(),
        templates.len(),
        start.elapsed().as_secs_f64(),
        &timings,
        rendered_bytes,
    )
}
```

### Test Cases

| Case | Templates | Values | Scenario |
|------|-----------|--------|----------|
| Default | 15 | standard | Out-of-box configuration |
| HA Mode | 15 | ha-config | High availability setup |
| Multi-Region | 15 | multi-region | Distributed deployment |
| Full Spec | 15 | all-features | Maximum feature set |

### Baseline (v0.1.0)

```json
{
  "chart_name": "stellar-operator",
  "values_count": 50,
  "total_templates": 15,
  "total_duration_secs": 1.12,
  "average_per_template_ms": 74.7,
  "p95_per_template_ms": 89.3,
  "rendered_bytes": 45230
}
```

**Thresholds:**
- Average per-template time: 74.7ms ± 20% = 59.76-89.64ms (warning)

## Operator API Benchmarks

### What We Measure

- **Throughput:** Requests per second
- **Latency:** Response time percentiles (P50, P95, P99)
- **Error rate:** Failed requests percentage

### Benchmark Definition

```rust
fn bench_operator_api(endpoint: &str, num_requests: usize) {
    let start = Instant::now();
    let mut latencies = vec![];
    let mut successful = 0;
    let mut failed = 0;
    
    for _ in 0..num_requests {
        let t0 = Instant::now();
        match http_get(endpoint) {
            Ok(_) => {
                latencies.push(t0.elapsed());
                successful += 1;
            }
            Err(_) => failed += 1,
        }
    }
    
    ApiThroughputResult::new(
        endpoint,
        successful,
        failed,
        start.elapsed().as_secs_f64(),
        &latencies,
    )
}
```

### Endpoints

| Endpoint | Method | Load | Scenario |
|----------|--------|------|----------|
| `/api/v1/stellarnodes` | GET | 50 RPS | List all nodes |
| `/api/v1/stellarnodes/{id}` | GET | 100 RPS | Single node lookup |
| `/api/v1/stellarnodes` | POST | 10 RPS | Create node |
| `/metrics` | GET | 50 RPS | Metrics export |

### Baseline (v0.1.0)

```json
{
  "endpoint": "/api/v1/stellarnodes",
  "total_requests": 1000,
  "successful_requests": 995,
  "failed_requests": 5,
  "duration_secs": 10.0,
  "rps": 99.5,
  "error_rate_percent": 0.5,
  "avg_latency_ms": 8.2,
  "p50_latency_ms": 7.1,
  "p95_latency_ms": 12.5,
  "p99_latency_ms": 18.3
}
```

**Thresholds:**
- P99 latency: 18.3ms ± 25% = 13.73-22.88ms (warning)
- Throughput: 99.5 RPS minimum (5% drop = 94.5 RPS)
- Error rate: < 1.5% (baseline 0.5% + 1%)

## Reconciliation Benchmarks

### What We Measure

- **Latency:** Time per reconciliation cycle
- **Success rate:** Percentage of successful reconciliations
- **Error count:** Failed reconciliation attempts

### Benchmark Definition

```rust
fn bench_reconciliation(resource_count: usize) {
    let start = Instant::now();
    let mut durations = vec![];
    let mut successful = 0;
    let mut failed = 0;
    
    for i in 0..resource_count {
        let resource = create_test_resource(i);
        let t0 = Instant::now();
        
        match reconcile(&resource) {
            Ok(_) => {
                durations.push(t0.elapsed());
                successful += 1;
            }
            Err(_) => {
                failed += 1;
            }
        }
    }
    
    ReconciliationResult::new(
        "StellarNode",
        resource_count,
        successful,
        failed,
        &durations,
    )
}
```

### Scenarios

| Scenario | Count | Resource Type | Condition |
|----------|-------|---------------|-----------|
| Steady State | 50 | StellarNode | All healthy |
| With Failures | 50 | StellarNode | 5 nodes degraded |
| Large Batch | 200 | StellarNode | Initial sync |
| Mixed Resources | 100 | mixed | Multiple resource types |

### Baseline (v0.1.0)

```json
{
  "resource_type": "StellarNode",
  "resource_count": 50,
  "total_reconciliations": 245,
  "successful": 240,
  "failed": 5,
  "avg_duration_ms": 342.5,
  "p50_duration_ms": 320.0,
  "p95_duration_ms": 520.0,
  "p99_duration_ms": 680.0,
  "total_duration_secs": 83.8
}
```

**Thresholds:**
- P99 duration: 680ms ± 30% = 476-884ms (warning)

## Running Benchmarks

### Run All Benchmarks

```bash
# Run all benchmarks and compare against baselines
make benchmark-all

# Output:
# → Running all benchmarks...
# [PASS] CRD validation: 204.5 RPS (baseline: 204.1) ✓
# [PASS] Helm rendering: 74.2ms avg (baseline: 74.7ms) ✓
# [PASS] Operator API: 99.1 RPS (baseline: 99.5) ✓
# [PASS] Reconciliation: 345ms p99 (baseline: 680ms) ✓
```

### Run Specific Benchmark

```bash
# CRD validation only
cargo bench --bench crd_operations

# Helm rendering only
make benchmark-helm

# Operator API load test
make benchmark-api

# Reconciliation latency
make benchmark-reconciliation
```

### Custom Run with Parameters

```bash
# Override number of requests
BENCH_REQUESTS=5000 make benchmark-api

# Specify threshold
BENCH_THRESHOLD=20 make benchmark-crd

# Save results to specific file
BENCH_OUTPUT=results/custom-run.json make benchmark-all
```

## Regression Detection

### Automatic Detection in CI

The CI pipeline automatically:

1. Runs benchmarks on every push to main
2. Compares against baseline (`benchmarks/baselines/`)
3. Fails if regression exceeds threshold
4. Reports detailed differences

**CI Job Output Example:**

```
[BENCHMARK] Comparing against v0.1.0 baseline...

❌ CRD validation REGRESSION:
   Baseline:  4.9ms avg
   Current:   6.2ms avg
   Increase:  26.5% (threshold: 15%)

✓ Helm rendering: OK (1% variance)
✓ Operator API: OK (3% variance)
✓ Reconciliation: OK (5% variance)

RESULT: FAILED (1 regression detected)
```

### Manual Regression Check

```bash
# Compare against baseline
python3 scripts/check-crd-performance.py \
  --current results/current-run.json \
  --baseline benchmarks/baselines/crd-performance-v0.1.0.json \
  --threshold 15
```

### Baseline Updates

To update baseline after intentional improvement:

```bash
# Run benchmarks
make benchmark-all

# Review results
cat results/benchmark-summary.json

# Update baseline (with review approval)
cp results/benchmark-summary.json benchmarks/baselines/v$(cat VERSION).json
git add benchmarks/baselines/
git commit -m "perf: update benchmark baseline after optimization"
```

## Interpreting Results

### Normal Variance

Benchmarks have natural variance ±2-5% due to:
- OS scheduling
- CPU frequency scaling
- Background processes
- Network conditions (API tests)

### Meaningful Regressions

- **5-10% regression:** Investigate, may be acceptable
- **10-20% regression:** Address before merge
- **>20% regression:** Requires explanation and fix

### Performance Optimization Opportunities

Look for:
- Consistent slow-down across all tests (system degradation)
- Regression in one specific test (code change impact)
- High percentile (P99) increases (outliers/contention)

### Healthy Improvements

- P99 latency decreases (better tail behavior)
- Throughput increases
- Error rate decreases

## References

- [Microbenchmarking Guidelines](https://github.com/bheisler/criterion.rs)
- [Rust Benchmarking](https://doc.rust-lang.org/unstable-book/library-features/test.html)
- [Kubernetes Performance Testing](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/performance-testing/)
