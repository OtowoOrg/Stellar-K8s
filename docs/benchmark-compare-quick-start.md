# Benchmark Compare Quick Start

Get started with multi-cluster performance comparison in 5 minutes.

## Prerequisites

- Two Kubernetes clusters with Stellar nodes deployed
- Prometheus installed on both clusters
- `kubectl` configured with contexts for both clusters
- `stellar-operator` CLI installed

## Quick Start

### 1. Verify Contexts

List available Kubernetes contexts:

```bash
kubectl config get-contexts
```

Example output:
```
CURRENT   NAME              CLUSTER           AUTHINFO
*         prod-us-east      prod-us-east      admin
          prod-us-west      prod-us-west      admin
          staging           staging-cluster   admin
```

### 2. Run Basic Comparison

Compare two clusters for 60 seconds:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod-us-east \
  --cluster-b-context prod-us-west
```

### 3. View Results

The tool will display a comparison table:

```
╔═══════════════════════════╦═══════════════╦═══════════════╦════════════╦═══════════╗
║ Metric                    ║ Cluster A     ║ Cluster B     ║ Difference ║ Winner    ║
╠═══════════════════════════╬═══════════════╬═══════════════╬════════════╬═══════════╣
║ TPS (mean)                ║ 1250.45       ║ 1180.32       ║ 5.9%       ║ Cluster A ║
║ TPS (p95)                 ║ 1320.12       ║ 1245.67       ║            ║           ║
║ Ledger Time (mean)        ║ 5.23s         ║ 5.67s         ║ 8.4%       ║ Cluster A ║
║ Consensus Latency (mean)  ║ 2.15s         ║ 2.45s         ║ 13.9%      ║ Cluster A ║
╚═══════════════════════════╩═══════════════╩═══════════════╩════════════╩═══════════╝
```

## Common Scenarios

### Scenario 1: A/B Test Configuration Change

Test a new configuration:

```bash
# Deploy new configuration to staging
kubectl apply -f new-config.yaml --context staging

# Wait for rollout
kubectl rollout status deployment/stellar-validator --context staging

# Compare performance
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --cluster-a-label "Current (Prod)" \
  --cluster-b-label "New Config (Staging)" \
  --duration 300 \
  --output ab-test.html
```

### Scenario 2: Cloud Provider Comparison

Compare AWS vs GCP:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context aws-cluster \
  --cluster-b-context gcp-cluster \
  --cluster-a-label "AWS (m5.2xlarge)" \
  --cluster-b-label "GCP (n2-standard-8)" \
  --duration 600 \
  --output cloud-comparison.html
```

### Scenario 3: Hardware Upgrade Validation

Validate hardware upgrade:

```bash
stellar-operator benchmark-compare \
  --cluster-a-context old-hardware \
  --cluster-b-context new-hardware \
  --cluster-a-label "Old (m5.large)" \
  --cluster-b-label "New (m5.xlarge)" \
  --duration 300
```

## Using Prometheus URLs

If you prefer to use Prometheus URLs directly:

```bash
# Port-forward Prometheus services
kubectl port-forward -n monitoring svc/prometheus 9090:9090 --context prod &
kubectl port-forward -n monitoring svc/prometheus 9091:9090 --context staging &

# Run comparison
stellar-operator benchmark-compare \
  --cluster-a-prometheus http://localhost:9090 \
  --cluster-b-prometheus http://localhost:9091 \
  --cluster-a-label "Production" \
  --cluster-b-label "Staging"
```

## Export Options

### HTML Report

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --output report.html \
  --format html

# Open in browser
open report.html  # macOS
xdg-open report.html  # Linux
```

### JSON Export

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --output data.json \
  --format json

# Analyze with jq
cat data.json | jq '.cluster_a_summary.tps.mean'
```

## Troubleshooting

### Issue: "Cannot discover Prometheus"

**Solution**: Provide Prometheus URL explicitly:

```bash
stellar-operator benchmark-compare \
  --cluster-a-prometheus http://prometheus.monitoring:9090 \
  --cluster-b-prometheus http://prometheus.monitoring:9090
```

### Issue: "No metrics returned"

**Solution**: Verify Prometheus is accessible:

```bash
# Check Prometheus service
kubectl get svc -n monitoring

# Test query
kubectl port-forward -n monitoring svc/prometheus 9090:9090
curl http://localhost:9090/api/v1/query?query=stellar_horizon_tps
```

### Issue: "Context not found"

**Solution**: Check available contexts:

```bash
kubectl config get-contexts
kubectl config use-context <context-name>
```

## Next Steps

1. **Longer Duration**: Run for 5-10 minutes for more accurate results
2. **Multiple Runs**: Run comparison multiple times to account for variability
3. **Peak Hours**: Test during peak load for realistic comparison
4. **Export Reports**: Save HTML reports for historical comparison
5. **Automate**: Integrate into CI/CD pipeline

## Advanced Options

### Custom Duration and Interval

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --duration 600 \
  --interval 10
```

### Specific Metrics

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --metrics tps,consensus_latency
```

### Disable Live Updates

```bash
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --live=false
```

## Example Workflow

Complete workflow for testing a configuration change:

```bash
#!/bin/bash

# 1. Baseline measurement
echo "Collecting baseline..."
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context prod \
  --cluster-a-label "Baseline Run 1" \
  --cluster-b-label "Baseline Run 2" \
  --duration 300 \
  --output baseline.html

# 2. Apply change to staging
echo "Applying configuration change..."
kubectl apply -f new-config.yaml --context staging
kubectl rollout status deployment/stellar-validator --context staging

# 3. Compare prod vs staging
echo "Comparing prod vs staging..."
stellar-operator benchmark-compare \
  --cluster-a-context prod \
  --cluster-b-context staging \
  --cluster-a-label "Production (Current)" \
  --cluster-b-label "Staging (New Config)" \
  --duration 600 \
  --output comparison.html

# 4. Analyze results
echo "Results saved to comparison.html"
open comparison.html
```

## Related Documentation

- [Full Benchmark Compare Documentation](./benchmark-compare.md)
- [Prometheus Setup](./prometheus-setup.md)
- [Performance Tuning](./performance-tuning.md)
- [Monitoring Guide](./monitoring.md)
