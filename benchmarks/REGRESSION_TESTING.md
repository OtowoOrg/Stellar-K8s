# Automated Performance Regression Testing

This document describes the automated performance regression testing system for the Stellar-K8s operator.

## Overview

The performance regression testing system ensures that no PR degrades the performance of the Stellar nodes or the operator itself. It automatically:

1. Spins up a kind cluster on every PR
2. Deploys the operator with the PR changes
3. Runs standardized load tests using k6
4. Compares results (TPS/Latency) with baseline from main branch
5. Fails CI if performance drops below threshold
6. Posts a summary comment on the PR with performance comparison table

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    GitHub Actions Workflow                   │
├─────────────────────────────────────────────────────────────┤
│                                                               │
│  1. Build Operator (Release Mode)                            │
│     └─> Docker Image                                         │
│                                                               │
│  2. Setup Kind Cluster                                       │
│     ├─> 1 Control Plane + 2 Workers                          │
│     ├─> Install CRDs                                         │
│     └─> Deploy Operator                                      │
│                                                               │
│  3. Run Performance Tests (k6)                               │
│     ├─> Operator Load Test (TPS, Latency, Reconciliation)   │
│     ├─> Webhook Load Test (Validation, Mutation)            │
│     └─> Generate Metrics JSON                                │
│                                                               │
│  4. Regression Analysis (Python)                             │
│     ├─> Compare with Baseline                                │
│     ├─> Detect Regressions (>10% degradation)               │
│     └─> Generate Report                                      │
│                                                               │
│  5. Post PR Comment                                          │
│     └─> Performance Comparison Table                         │
│                                                               │
│  6. Fail CI if Regression Detected                           │
│                                                               │
└─────────────────────────────────────────────────────────────┘
```

## Workflows

### 1. Performance Regression Testing (`.github/workflows/performance-regression.yml`)

**Triggers:**
- Pull requests to main branch (when src/, Cargo.toml, or benchmarks/ change)
- Manual workflow dispatch

**Jobs:**

1. **build**: Builds operator in release mode and creates Docker image
2. **setup-cluster**: Creates kind cluster and deploys operator
3. **performance-test**: Runs k6 load tests against operator
4. **regression-analysis**: Compares results with baseline and posts PR comment
5. **cleanup**: Deletes kind cluster

**Key Features:**
- Automatic kind cluster provisioning
- Operator deployment with proper RBAC
- Port forwarding for local access
- Comprehensive logging and artifact collection
- PR comment with performance comparison table
- CI failure on regression detection

### 2. Webhook Benchmark (`.github/workflows/webhook-benchmark.yml`)

**Triggers:**
- Pull requests that modify webhook code
- Pushes to main branch

**Focus:**
- Webhook-specific performance (validation/mutation latency)
- Comparison with Go-based webhook baselines
- Rust performance advantage quantification

### 3. General Benchmark (`.github/workflows/benchmark.yml`)

**Triggers:**
- Pushes to main branch
- Manual workflow dispatch

**Focus:**
- Full operator performance testing
- Baseline updates on release tags

## Baseline Management

### Baseline Files

Baselines are stored in `benchmarks/baselines/` with version-specific naming:

- `v0.1.0.json` - Operator baseline
- `webhook-v0.1.0.json` - Webhook baseline

### Baseline Structure

```json
{
  "version": "v0.1.0",
  "baseline_created": "2024-01-01T00:00:00Z",
  "description": "Performance baseline",
  "metrics": {
    "tps": { "avg": 150.0 },
    "http_req_duration": {
      "avg": 45.0,
      "p50": 35.0,
      "p95": 120.0,
      "p99": 250.0
    },
    "reconciliation_duration": {
      "avg": 450.0,
      "p95": 1200.0,
      "p99": 2500.0
    },
    "error_rate": 0.001
  },
  "thresholds": {
    "http_req_duration_p95": { "limit": 500, "unit": "ms" },
    "error_rate": { "limit": 0.01, "unit": "percentage" }
  }
}
```

### Creating a New Baseline

After a release, create a new baseline:

```bash
python benchmarks/scripts/compare_benchmarks.py baseline \
  --input results/benchmark-summary.json \
  --output benchmarks/baselines/v1.0.0.json \
  --version v1.0.0
```

## Regression Detection

### Comparison Script

The `benchmarks/scripts/compare_benchmarks.py` script performs regression analysis:

```bash
python benchmarks/scripts/compare_benchmarks.py compare \
  --current results/benchmark-summary.json \
  --baseline benchmarks/baselines/v0.1.0.json \
  --threshold 10 \
  --output results/regression-report.json \
  --fail-on-regression \
  --verbose
```

### Regression Criteria

A regression is detected when:

**Latency Metrics** (lower is better):
- Increase > threshold % (default: 10%)
- Examples: http_req_duration, reconciliation_duration, api_latency

**Throughput Metrics** (higher is better):
- Decrease > threshold % (default: 10%)
- Examples: TPS, requests per second

**Error Rate**:
- Any increase in error rate

### Regression Report Format

```json
{
  "timestamp": "2024-01-01T12:00:00Z",
  "threshold_percent": 10.0,
  "baseline_version": "v0.1.0",
  "current_version": "pr-123-abc1234",
  "overall_passed": false,
  "summary": "❌ 2 regression(s) detected exceeding 10% threshold.",
  "regressions": [
    {
      "metric": "http_req_duration.p99",
      "baseline": 250.0,
      "current": 300.0,
      "change_percent": 20.0,
      "threshold_percent": 10.0,
      "direction": "increased"
    }
  ],
  "improvements": [],
  "stable": []
}
```

## Performance Thresholds

### Operator Thresholds

| Metric | Threshold | Description |
|--------|-----------|-------------|
| TPS | > 100 req/s | Minimum transactions per second |
| HTTP p95 | < 500 ms | 95th percentile HTTP latency |
| HTTP p99 | < 1000 ms | 99th percentile HTTP latency |
| Reconciliation p95 | < 3000 ms | 95th percentile reconciliation time |
| Reconciliation p99 | < 5000 ms | 99th percentile reconciliation time |
| API p95 | < 200 ms | 95th percentile API latency |
| Error Rate | < 1% | Maximum error rate |

### Webhook Thresholds

| Metric | Threshold | Description |
|--------|-----------|-------------|
| Validation p99 | < 50 ms | 99th percentile validation latency |
| Validation p95 | < 30 ms | 95th percentile validation latency |
| Mutation p99 | < 50 ms | 99th percentile mutation latency |
| Mutation p95 | < 30 ms | 95th percentile mutation latency |
| Throughput | > 100 req/s | Minimum webhook throughput |
| Error Rate | < 0.1% | Maximum webhook error rate |

## PR Comment Format

The workflow posts a comment on each PR with performance results:

```markdown
## 📊 Performance Regression Test Results

**PR:** #123
**Version:** pr-123-abc1234
**Commit:** abc1234567890
**Baseline:** v0.1.0
**Threshold:** 10%

### 🎯 Key Performance Metrics

| Metric | Value | Threshold |
|--------|-------|-----------|
| **TPS (avg)** | 145.2 req/s | > 100 req/s |
| **HTTP Latency (p95)** | 125.3 ms | < 500 ms |
| **HTTP Latency (p99)** | 280.5 ms | < 1000 ms |
| **Reconciliation (p95)** | 1150.0 ms | < 3000 ms |
| **Error Rate** | 0.002 | < 0.01 |

### ✅ Regression Check: PASSED

✅ No regressions detected. All metrics within 10% threshold.

- 🟢 Improvements: 2
- ⚪ Stable: 6
- 🔴 Regressions: 0
```

## Local Testing

### Run Performance Tests Locally

1. Start a kind cluster:

```bash
kind create cluster --name benchmark
```

2. Build and deploy operator:

```bash
cargo build --release
docker build -t stellar-operator:local .
kind load docker-image stellar-operator:local --name benchmark

kubectl apply -f config/crd/stellarnode-crd.yaml
kubectl create namespace stellar-system
# Deploy operator (see workflow for full manifest)
```

3. Run benchmarks:

```bash
# Port forward operator
kubectl port-forward -n stellar-system svc/stellar-operator 8080:8080 &
kubectl proxy --port=8001 &

# Run k6 tests
k6 run \
  --env BASE_URL=http://localhost:8080 \
  --env K8S_API_URL=http://localhost:8001 \
  --env NAMESPACE=stellar-benchmark \
  benchmarks/k6/operator-load-test.js
```

4. Compare with baseline:

```bash
python benchmarks/scripts/compare_benchmarks.py compare \
  --current results/benchmark-summary.json \
  --baseline benchmarks/baselines/v0.1.0.json \
  --threshold 10 \
  --output results/regression-report.json \
  --verbose
```

## Troubleshooting

### Kind Cluster Issues

```bash
# Check cluster status
kind get clusters
kubectl cluster-info --context kind-benchmark

# View cluster logs
kind export logs --name benchmark logs/

# Delete and recreate
kind delete cluster --name benchmark
```

### Operator Deployment Issues

```bash
# Check operator logs
kubectl logs -n stellar-system -l app=stellar-operator

# Check operator status
kubectl get deployment -n stellar-system stellar-operator
kubectl describe deployment -n stellar-system stellar-operator

# Check RBAC
kubectl auth can-i --list --as=system:serviceaccount:stellar-system:stellar-operator
```

### Benchmark Failures

```bash
# Verify connectivity
curl http://localhost:8080/healthz
curl http://localhost:8001/api/v1/namespaces

# Check port forwarding
ps aux | grep "port-forward"
netstat -tlnp | grep 8080

# Run with debug output
k6 run --http-debug benchmarks/k6/operator-load-test.js
```

### Python Script Issues

```bash
# Install dependencies
pip install requests

# Run with verbose output
python benchmarks/scripts/compare_benchmarks.py compare \
  --current results/benchmark-summary.json \
  --baseline benchmarks/baselines/v0.1.0.json \
  --threshold 10 \
  --output results/regression-report.json \
  --verbose
```

## Configuration

### Adjust Regression Threshold

Default threshold is 10%. Adjust via workflow input:

```yaml
workflow_dispatch:
  inputs:
    regression_threshold:
      description: Regression threshold percentage
      default: "10"
```

Or in the workflow file:

```yaml
env:
  REGRESSION_THRESHOLD: 15  # 15% threshold
```

### Customize Baseline Version

Specify which baseline to compare against:

```yaml
workflow_dispatch:
  inputs:
    baseline_version:
      description: Baseline version to compare against
      default: v0.1.0
```

### Modify Performance Thresholds

Edit thresholds in k6 scripts:

```javascript
// benchmarks/k6/operator-load-test.js
thresholds: {
    http_req_duration: ['p(95)<500', 'p(99)<1000'],
    reconciliation_duration: ['p(95)<3000', 'p(99)<5000'],
    tps: ['rate>100'],
}
```

## Best Practices

1. **Update baselines on releases**: Create new baseline after each major release
2. **Review regressions carefully**: Not all regressions are bugs (some may be acceptable tradeoffs)
3. **Run locally before pushing**: Catch performance issues early
4. **Monitor trends**: Track performance over time, not just single PRs
5. **Document intentional changes**: If a PR intentionally trades performance for features, document it

## Integration with CI/CD

The performance regression workflow integrates with the existing CI/CD pipeline:

```
PR Created
    ↓
CI Workflow (lint, test, build)
    ↓
Performance Regression Workflow
    ├─> Build operator
    ├─> Setup kind cluster
    ├─> Run benchmarks
    ├─> Compare with baseline
    └─> Post PR comment
    ↓
Manual Review
    ↓
Merge (if all checks pass)
    ↓
Update Baseline (on release tags)
```

## Metrics Collected

### Operator Metrics

- **TPS (Transactions Per Second)**: Overall throughput
- **HTTP Request Duration**: API endpoint latency (avg, p50, p95, p99)
- **Reconciliation Duration**: Time to reconcile CRD changes (avg, p95, p99)
- **API Latency**: REST API response time (avg, p95, p99)
- **Health Check Latency**: Health endpoint response time
- **CRD Operation Latency**: Time for create/update/delete operations
- **Error Rate**: Percentage of failed requests
- **Queue Depth**: Number of pending reconciliations

### Webhook Metrics

- **Validation Latency**: Validation webhook response time (avg, p50, p95, p99)
- **Mutation Latency**: Mutation webhook response time (avg, p50, p95, p99)
- **Throughput**: Webhook requests per second
- **Error Rate**: Percentage of failed webhook calls

## Example PR Comment

When a PR is created, the workflow automatically posts a comment:

```markdown
## 📊 Performance Regression Test Results

**PR:** #123
**Version:** pr-123-abc1234
**Commit:** abc1234567890
**Baseline:** v0.1.0
**Threshold:** 10%

### 🎯 Key Performance Metrics

| Metric | Value | Threshold |
|--------|-------|-----------|
| **TPS (avg)** | 145.2 req/s | > 100 req/s |
| **HTTP Latency (p95)** | 125.3 ms | < 500 ms |
| **HTTP Latency (p99)** | 280.5 ms | < 1000 ms |
| **Reconciliation (p95)** | 1150.0 ms | < 3000 ms |
| **Reconciliation (p99)** | 2300.0 ms | < 5000 ms |
| **API Latency (p95)** | 85.0 ms | < 200 ms |
| **Error Rate** | 0.002 | < 0.01 |
| **Total Requests** | 50000 | - |

### ✅ Regression Check: PASSED

✅ No regressions detected. All metrics within 10% threshold.

- 🟢 Improvements: 2
- ⚪ Stable: 6
- 🔴 Regressions: 0

<details>
<summary>📈 View Improvements</summary>

- API Latency p95: 100ms → 85ms (-15%)
- Reconciliation p99: 2500ms → 2300ms (-8%)

</details>
```

## Handling Regressions

### When Regression is Detected

1. **Review the PR comment**: Check which metrics regressed
2. **Analyze the changes**: Identify code changes that may have caused regression
3. **Profile the code**: Use profiling tools to find bottlenecks
4. **Optimize or justify**: Either fix the regression or document why it's acceptable

### Acceptable Regressions

Some regressions may be acceptable if:
- New features require additional processing
- Security improvements add overhead
- Better error handling adds latency
- Tradeoff is documented and justified

In these cases, update the baseline or adjust thresholds.

### Unacceptable Regressions

Regressions that should be fixed:
- Accidental inefficiencies (N+1 queries, unnecessary allocations)
- Missing optimizations (caching, batching)
- Blocking operations in async code
- Memory leaks or resource exhaustion

## Performance Optimization Tips

### Rust-Specific Optimizations

1. **Use release builds**: Always benchmark with `--release`
2. **Profile with flamegraph**: `cargo flamegraph --bin stellar-operator`
3. **Check allocations**: Use `cargo-instruments` or `heaptrack`
4. **Optimize hot paths**: Focus on reconciliation loop and webhook handlers
5. **Use async efficiently**: Avoid blocking operations, use tokio::spawn for parallelism

### Kubernetes-Specific Optimizations

1. **Batch API calls**: Use informers and caching
2. **Optimize watches**: Filter events at the API level
3. **Use field selectors**: Reduce data transfer
4. **Implement rate limiting**: Prevent API server overload
5. **Cache frequently accessed data**: Reduce API calls

## Monitoring Performance Trends

### View Historical Results

Benchmark results are stored as artifacts:

```bash
# Download from GitHub Actions
gh run download <run-id> -n performance-results-<version>

# View results
cat results/benchmark-summary.json | jq '.metrics'
```

### Track Performance Over Time

Create a dashboard to visualize performance trends:

1. Collect benchmark results from each PR
2. Store in time-series database (Prometheus, InfluxDB)
3. Visualize with Grafana
4. Set up alerts for degradation trends

## FAQ

**Q: Why does the workflow take so long?**
A: Setting up a kind cluster, building the operator, and running comprehensive benchmarks takes 10-15 minutes. This ensures accurate performance measurement.

**Q: Can I skip performance tests for small PRs?**
A: The workflow only runs when relevant files change (src/, Cargo.toml, benchmarks/). Documentation-only PRs won't trigger it.

**Q: What if my PR intentionally changes performance?**
A: Document the change in the PR description and update the baseline after merge.

**Q: How do I update the baseline?**
A: Baselines are automatically updated on release tags. For manual updates, use the compare_benchmarks.py script.

**Q: Can I run performance tests locally?**
A: Yes! Follow the "Local Testing" section above.

## References

- [k6 Documentation](https://k6.io/docs/)
- [kind Documentation](https://kind.sigs.k8s.io/)
- [Kubernetes Performance Testing](https://kubernetes.io/docs/concepts/cluster-administration/system-metrics/)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
