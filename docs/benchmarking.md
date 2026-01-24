# Performance Benchmarking Guide

This document describes the automated performance benchmarking suite for the Stellar-K8s operator. The suite measures TPS, latency, and resource consumption for every release and blocks CI/CD if performance regressions exceed 10%.

## Overview

The benchmarking suite consists of:

- **k6 Load Tests**: Comprehensive load testing scripts measuring API endpoints, CRD operations, and reconciliation loops
- **Baseline Management**: Version-tagged performance baselines for regression comparison
- **Regression Detection**: Automated comparison tool that fails builds when performance degrades
- **CI/CD Integration**: GitHub Actions workflow for automated benchmarking on every PR and release

## Quick Start

### Prerequisites

```bash
# Install k6
brew install k6  # macOS
# or
sudo apt-get install k6  # Ubuntu/Debian

# Verify installation
k6 version
```

### Running Locally

```bash
# Start the operator (must be running)
cargo run

# In another terminal, start kubectl proxy
kubectl proxy --port=8001

# Run benchmarks
./benchmarks/scripts/run-benchmarks.sh
```

### Running with Custom Options

```bash
# Compare against specific baseline version
./benchmarks/scripts/run-benchmarks.sh --baseline v1.0.0

# Use custom regression threshold (15% instead of 10%)
./benchmarks/scripts/run-benchmarks.sh --threshold 15

# Update baseline after successful run
./benchmarks/scripts/run-benchmarks.sh --update-baseline

# Verbose output
./benchmarks/scripts/run-benchmarks.sh --verbose
```

## Benchmark Scenarios

The k6 test suite includes four scenarios:

### 1. Steady State (2 minutes)
- **VUs**: 10 constant
- **Purpose**: Establish baseline performance under normal load
- **Metrics**: Average TPS, latency percentiles

### 2. Stress Test (3.5 minutes)
- **VUs**: Ramp from 0 → 20 → 50 → 100 → 0
- **Purpose**: Test performance degradation under increasing load
- **Metrics**: Breaking point identification, error rates

### 3. Spike Test (50 seconds)
- **VUs**: Sudden spike to 200 VUs
- **Purpose**: Test system resilience to sudden traffic bursts
- **Metrics**: Recovery time, error handling

### 4. Reconciliation Load (2 minutes)
- **Rate**: 50 reconciliations/second
- **Purpose**: Test controller throughput
- **Metrics**: Reconciliation latency, queue depth

## Metrics Collected

### Core Metrics

| Metric | Description | Threshold |
|--------|-------------|-----------|
| `tps` | Transactions per second | > 100 req/s |
| `http_req_duration (p95)` | 95th percentile latency | < 500ms |
| `http_req_duration (p99)` | 99th percentile latency | < 1000ms |
| `http_req_failed` | Error rate | < 1% |

### Reconciliation Metrics

| Metric | Description | Threshold |
|--------|-------------|-----------|
| `reconciliation_duration (p95)` | Reconciliation latency | < 3000ms |
| `reconciliation_duration (p99)` | Reconciliation latency | < 5000ms |
| `active_reconciliations` | Concurrent reconciliations | gauge |
| `queue_depth` | Pending items in queue | gauge |

### API Metrics

| Metric | Description | Threshold |
|--------|-------------|-----------|
| `api_latency (p95)` | REST API latency | < 200ms |
| `health_check_latency (p95)` | Health endpoint | < 100ms |
| `crd_operation_latency (p95)` | CRD CRUD operations | < 500ms |

## Regression Detection

### How It Works

1. **Baseline Creation**: Performance metrics are captured for each release tag
2. **Comparison**: Current run metrics are compared against the baseline
3. **Threshold Check**: Any metric exceeding the baseline by more than 10% is flagged
4. **CI/CD Gate**: If regressions are detected, the pipeline fails

### Regression Threshold

The default regression threshold is **10%**. This means:

- For latency metrics (lower is better): Current value must be ≤ baseline × 1.10
- For throughput metrics (higher is better): Current value must be ≥ baseline × 0.90

### Example Regression Report

```
=======================================================================
  PERFORMANCE REGRESSION ANALYSIS
=======================================================================

  Baseline Version:  v1.0.0
  Current Version:   sha-abc1234
  Threshold:         10%

❌ FAILED: 2 regressions detected (threshold: 10%)

❌ REGRESSIONS (2)
------------------------------------------------------------
  reconciliation_duration.p95:
    Baseline: 1200.00
    Current:  1450.00 (↑ 20.8%)
    Status:   REGRESSION (exceeds 10% threshold)

  api_latency.p95:
    Baseline: 80.00
    Current:  95.00 (↑ 18.8%)
    Status:   REGRESSION (exceeds 10% threshold)

✅ IMPROVEMENTS (1)
------------------------------------------------------------
  tps.avg: 150.00 → 175.00 (↑ 16.7% higher throughput)

=======================================================================
  RESULT: FAILED - Performance regressions exceed 10% threshold
=======================================================================
```

## CI/CD Integration

### GitHub Actions Workflow

The `benchmark.yml` workflow runs automatically on:
- Pull requests to `main`
- Pushes to `main` and `develop`
- Release tags (`v*`)

### Workflow Jobs

1. **Build**: Compile operator and build Docker image
2. **Benchmark**: Run k6 tests in Kind cluster
3. **Report**: Post results as PR comment
4. **Update Baseline**: Create new baseline on release tags

### Manual Trigger

```bash
# Trigger benchmark with custom baseline
gh workflow run benchmark.yml \
  -f baseline_version=v1.0.0 \
  -f regression_threshold=15
```

## Creating Baselines

### Automatic (Recommended)

Baselines are automatically created when a release tag is pushed:

```bash
git tag v1.1.0
git push origin v1.1.0
# Baseline will be created after successful benchmark
```

### Manual

```bash
python benchmarks/scripts/compare_benchmarks.py baseline \
  --input results/benchmark-summary.json \
  --output benchmarks/baselines/v1.1.0.json \
  --version v1.1.0
```

## Baseline File Format

```json
{
  "version": "v1.0.0",
  "baseline_created": "2024-01-01T00:00:00Z",
  "metrics": {
    "tps": { "avg": 150.0 },
    "http_req_duration": {
      "avg": 45.0,
      "p95": 120.0,
      "p99": 250.0
    },
    "reconciliation_duration": {
      "avg": 450.0,
      "p95": 1200.0
    },
    "error_rate": 0.001
  }
}
```

## Interpreting Results

### Healthy Results

```
✅ PASSED: All 12 metrics within threshold
   TPS: 165.3 req/s
   Latency (p95): 98ms
   Error Rate: 0.05%
```

### Warning Signs

- **High p99 latency**: May indicate GC pauses or resource contention
- **Increasing queue depth**: Controller cannot keep up with load
- **Error spikes during stress**: May need backpressure mechanisms

### Troubleshooting Poor Performance

1. **Check resource limits**: Ensure operator has sufficient CPU/memory
2. **Review reconciliation logic**: Look for N+1 queries or unnecessary API calls
3. **Analyze logs**: Check for rate limiting or API errors
4. **Profile the application**: Use `cargo flamegraph` for CPU profiling

## Customizing Tests

### Adding Custom Metrics

Edit `benchmarks/k6/operator-load-test.js`:

```javascript
// Add new custom metric
const myMetric = new Trend('my_custom_metric');

// Record values
myMetric.add(someValue);
```

### Modifying Thresholds

Edit the `options.thresholds` object:

```javascript
export const options = {
    thresholds: {
        'my_custom_metric': ['p(95)<100'],
    },
};
```

### Adding New Scenarios

```javascript
export const options = {
    scenarios: {
        my_scenario: {
            executor: 'constant-vus',
            vus: 25,
            duration: '5m',
            tags: { scenario: 'my_scenario' },
        },
    },
};
```

## Best Practices

1. **Run benchmarks on dedicated infrastructure**: Avoid noisy neighbor effects
2. **Use consistent hardware**: Compare apples to apples
3. **Warm up the system**: First runs may be slower due to caching
4. **Monitor during benchmarks**: Check Prometheus/Grafana for insights
5. **Document baseline conditions**: Record cluster size, K8s version, etc.

## Troubleshooting

### "No baseline file found"

```bash
# Create initial baseline
./benchmarks/scripts/run-benchmarks.sh --update-baseline
```

### "Connection refused to operator"

```bash
# Ensure operator is running
cargo run &

# Check health endpoint
curl http://localhost:8080/healthz
```

### "k6 not found"

```bash
# Install k6
brew install k6  # macOS
# or see https://k6.io/docs/get-started/installation/
```

## References

- [k6 Documentation](https://k6.io/docs/)
- [k6 for Kubernetes](https://k6.io/docs/testing-guides/load-testing-kubernetes/)
- [Prometheus Metrics Best Practices](https://prometheus.io/docs/practices/naming/)
