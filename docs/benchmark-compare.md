# Multi-Cluster Performance Comparison

Compare performance metrics (TPS, Ledger Time, Consensus Latency) between two different Kubernetes clusters or Prometheus instances in real-time.

## Overview

The `benchmark-compare` subcommand enables operators to perform A/B testing of optimizations, compare different configurations, and validate that one cluster performs better than another. This is essential when testing infrastructure changes, evaluating different cloud providers, or comparing hardware configurations.

## Features

- **Dual-Cluster Support**: Connect to two different Kubernetes contexts or Prometheus instances
- **Real-time Metrics**: Collect and compare metrics in real-time
- **Statistical Analysis**: Calculate mean, median, p95, p99, and standard deviation
- **Multiple Output Formats**: Terminal table, HTML report, JSON export, PDF (planned)
- **Side-by-Side Comparison**: Clear visualization of performance differences
- **Configurable Duration**: Collect metrics for any duration (default: 60 seconds)
- **Flexible Sampling**: Adjust sampling interval (default: 5 seconds)

## Usage

### Basic Comparison

Compare two Kubernetes contexts:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod-us-east \
  --cluster-b-context prod-us-west
```

### Compare Prometheus Instances

Directly compare two Prometheus instances:

```bash
stellar-operator benchmark-compare \
  --cluster-a-prometheus http://prom-a:9090 \
  --cluster-b-prometheus http://prom-b:9090
```

### Custom Labels

Use custom labels for better readability:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --cluster-a-label "Production (AWS)" \
  --cluster-b-label "Staging (GCP)"
```

### Extended Duration

Collect metrics for 5 minutes with 10-second intervals:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --duration 300 \
  --interval 10
```

### Export to HTML

Generate an HTML report:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --output report.html \
  --format html
```

### Export to JSON

Export raw data for further analysis:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --output data.json \
  --format json
```

## Command-Line Options

| Option | Description | Default |
|--------|-------------|---------|
| `--cluster-a-context` | Kubernetes context for Cluster A | - |
| `--cluster-b-context` | Kubernetes context for Cluster B | - |
| `--cluster-a-prometheus` | Prometheus URL for Cluster A | - |
| `--cluster-b-prometheus` | Prometheus URL for Cluster B | - |
| `--cluster-a-label` | Display label for Cluster A | "Cluster A" |
| `--cluster-b-label` | Display label for Cluster B | "Cluster B" |
| `--namespace` | Kubernetes namespace to query | "stellar-system" |
| `--duration` | Duration to collect metrics (seconds) | 60 |
| `--interval` | Sampling interval (seconds) | 5 |
| `--output` | Output file path | - (stdout) |
| `--format` | Output format (table, html, json, pdf) | table |
| `--live` | Show real-time updates in terminal | true |
| `--metrics` | Metrics to compare (comma-separated) | tps,ledger_time,consensus_latency,sync_status |

## Metrics Collected

### Transactions Per Second (TPS)
- **Query**: `rate(stellar_horizon_tps[1m])`
- **Description**: Number of transactions processed per second
- **Higher is Better**: Yes

### Ledger Close Time
- **Query**: `stellar_node_ledger_close_time_seconds`
- **Description**: Time taken to close a ledger
- **Lower is Better**: Yes

### Consensus Latency
- **Query**: `stellar_consensus_latency_seconds`
- **Description**: Time to reach consensus
- **Lower is Better**: Yes

### Sync Status
- **Query**: `stellar_node_sync_status`
- **Description**: Node synchronization status
- **Values**: 0=Pending, 1=Creating, 2=Running, 3=Syncing, 4=Ready

### Active Validators
- **Query**: `count(stellar_node_up == 1)`
- **Description**: Number of active validators

### Ledger Sequence
- **Query**: `stellar_node_ledger_sequence`
- **Description**: Current ledger sequence number

## Output Formats

### Terminal Table

Default output format with color-coded results:

```
╔═══════════════════════════╦═══════════════╦═══════════════╦════════════╦═══════════╗
║ Metric                    ║ Cluster A     ║ Cluster B     ║ Difference ║ Winner    ║
╠═══════════════════════════╬═══════════════╬═══════════════╬════════════╬═══════════╣
║ TPS (mean)                ║ 1250.45       ║ 1180.32       ║ 5.9%       ║ Cluster A ║
║ TPS (p95)                 ║ 1320.12       ║ 1245.67       ║            ║           ║
║ Ledger Time (mean)        ║ 5.23s         ║ 5.67s         ║ 8.4%       ║ Cluster A ║
║ Consensus Latency (mean)  ║ 2.15s         ║ 2.45s         ║ 13.9%      ║ Cluster A ║
║ Sample Count              ║ 12            ║ 12            ║            ║           ║
║ Duration                  ║ 60s           ║ 60s           ║            ║           ║
╚═══════════════════════════╩═══════════════╩═══════════════╩════════════╩═══════════╝
```

### HTML Report

Professional HTML report with:
- Summary statistics
- Side-by-side comparison table
- Winner highlighting
- Timestamp and metadata

### JSON Export

Raw data export for custom analysis:

```json
{
  "cluster_a_label": "Production",
  "cluster_b_label": "Staging",
  "cluster_a_metrics": [...],
  "cluster_b_metrics": [...],
  "cluster_a_summary": {
    "tps": {
      "mean": 1250.45,
      "median": 1248.32,
      "min": 1180.12,
      "max": 1320.45,
      "p95": 1310.23,
      "p99": 1318.67,
      "stddev": 35.42
    }
  },
  "cluster_b_summary": {...},
  "duration_secs": 60,
  "sample_count": 12
}
```

## Use Cases

### 1. A/B Testing Infrastructure Changes

Test whether a new configuration improves performance:

```bash
# Before change
stellar-operator benchmark-compare \
  --cluster-a-context prod-current \
  --cluster-b-context prod-new-config \
  --cluster-a-label "Current Config" \
  --cluster-b-label "New Config" \
  --duration 300 \
  --output ab-test.html
```

### 2. Cloud Provider Comparison

Compare performance across different cloud providers:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context aws-us-east-1 \
  --cluster-b-context gcp-us-central1 \
  --cluster-a-label "AWS" \
  --cluster-b-label "GCP" \
  --duration 600
```

### 3. Hardware Configuration Testing

Compare different instance types:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context m5-large \
  --cluster-b-context c5-xlarge \
  --cluster-a-label "m5.large (General)" \
  --cluster-b-label "c5.xlarge (Compute)" \
  --duration 300
```

### 4. Network Optimization Validation

Verify network optimizations:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context before-optimization \
  --cluster-b-context after-optimization \
  --cluster-a-label "Before" \
  --cluster-b-label "After" \
  --metrics tps,consensus_latency \
  --duration 180
```

### 5. Multi-Region Performance

Compare performance across regions:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context us-east \
  --cluster-b-context eu-west \
  --cluster-a-label "US East" \
  --cluster-b-label "EU West" \
  --duration 300
```

## Statistical Analysis

The tool calculates the following statistics for each metric:

- **Mean**: Average value across all samples
- **Median**: Middle value when sorted
- **Min**: Minimum value observed
- **Max**: Maximum value observed
- **P95**: 95th percentile (95% of values are below this)
- **P99**: 99th percentile (99% of values are below this)
- **StdDev**: Standard deviation (measure of variability)

## Prometheus Discovery

When using Kubernetes contexts, the tool automatically discovers Prometheus:

1. Checks for common Prometheus service names:
   - `prometheus`
   - `prometheus-server`
   - `kube-prometheus-stack-prometheus`
   - `prometheus-operated`

2. Queries the service to get the endpoint

3. Falls back to manual URL if discovery fails

## Troubleshooting

### Prometheus Not Found

**Problem**: Cannot discover Prometheus service

**Solution**: Provide Prometheus URL explicitly:
```bash
stellar-operator benchmark-compare \
  --cluster-a-prometheus http://prometheus.monitoring:9090 \
  --cluster-b-prometheus http://prometheus.monitoring:9090
```

### No Metrics Returned

**Problem**: Queries return no data

**Solution**: Verify metrics exist:
```bash
# Check if metrics are available
kubectl port-forward -n monitoring svc/prometheus 9090:9090
curl http://localhost:9090/api/v1/query?query=stellar_horizon_tps
```

### Context Not Found

**Problem**: Kubernetes context doesn't exist

**Solution**: List available contexts:
```bash
kubectl config get-contexts
```

### Permission Denied

**Problem**: Cannot access Prometheus

**Solution**: Check RBAC permissions:
```bash
kubectl auth can-i get services -n monitoring
```

## Best Practices

1. **Sufficient Duration**: Collect metrics for at least 60 seconds to get meaningful statistics
2. **Consistent Load**: Ensure both clusters have similar load during comparison
3. **Multiple Runs**: Run comparison multiple times to account for variability
4. **Peak Hours**: Test during peak hours for realistic comparison
5. **Baseline First**: Establish baseline before making changes
6. **Document Changes**: Keep track of what changed between comparisons
7. **Export Results**: Save results for historical comparison

## Integration with CI/CD

### GitHub Actions

```yaml
name: Performance Comparison

on:
  pull_request:
    branches: [main]

jobs:
  compare:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Setup kubectl
        uses: azure/setup-kubectl@v3
      
      - name: Compare Performance
        run: |
          stellar-operator benchmark-compare \
            --cluster-a-context prod \
            --cluster-b-context staging \
            --duration 300 \
            --output comparison.html \
            --format html
      
      - name: Upload Report
        uses: actions/upload-artifact@v3
        with:
          name: performance-report
          path: comparison.html
```

### GitLab CI

```yaml
performance_comparison:
  stage: test
  script:
    - stellar-operator benchmark-compare
        --cluster-a-context prod
        --cluster-b-context staging
        --duration 300
        --output comparison.html
        --format html
  artifacts:
    paths:
      - comparison.html
    expire_in: 1 week
```

## Advanced Usage

### Custom Metrics

Query custom Prometheus metrics:

```bash
# Modify the tool to support custom queries
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --metrics "custom_metric_1,custom_metric_2"
```

### Continuous Monitoring

Run comparison continuously:

```bash
while true; do
  stellar-operator benchmark-compare \
    --cluster-a-context prod \
    --cluster-b-context staging \
    --duration 60 \
    --output "report-$(date +%Y%m%d-%H%M%S).html"
  sleep 300
done
```

### Alerting on Degradation

Alert if performance degrades:

```bash
#!/bin/bash
result=$(stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --format json \
  --output /tmp/comparison.json)

# Parse JSON and check if staging is worse
tps_diff=$(jq '.cluster_a_summary.tps.mean - .cluster_b_summary.tps.mean' /tmp/comparison.json)

if (( $(echo "$tps_diff > 100" | bc -l) )); then
  echo "WARNING: Staging TPS is significantly lower than production"
  # Send alert
fi
```

## Related Documentation

- [Benchmarking Guide](./benchmarking.md)
- [Prometheus Metrics](./metrics.md)
- [Performance Tuning](./performance-tuning.md)
- [Monitoring Setup](./monitoring-setup.md)
