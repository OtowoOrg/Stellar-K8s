# Byzantine Monitoring — Quick Start

Deploy multi-vantage-point consensus monitoring in 5 minutes.

## What Problem Does This Solve?

A Stellar validator might believe it's in consensus while actually being network-partitioned. Monitoring from a single location can't detect this — you need multiple geographic vantage points.

## Quick Deploy (3 Watchers)

### 1. Create `byzantine-values.yaml`

```yaml
byzantineWatcher:
  enabled: true
  regions:
    - cloud: aws
      region: us-east-1
      nodeEndpoint: http://stellar-core.stellar-system.svc.cluster.local:11626
    - cloud: gcp
      region: eu-west-1
      nodeEndpoint: http://stellar-core-eu.stellar-system.svc.cluster.local:11626
    - cloud: azure
      region: ap-south-1
      nodeEndpoint: http://stellar-core-ap.stellar-system.svc.cluster.local:11626
```

### 2. Deploy

```bash
helm upgrade stellar-operator charts/stellar-operator \
  --namespace stellar-system \
  --values byzantine-values.yaml
```

### 3. Apply Alerts

```bash
kubectl apply -f monitoring/byzantine-alerts.yaml -n monitoring
```

### 4. Import Dashboard

Grafana → Import → Upload `monitoring/byzantine-dashboard.json`

## What You Get

- **Real-time divergence detection**: Alert fires when >20% of watchers see different ledger hashes
- **Geographic coverage**: Detect partitions that single-cluster monitoring misses
- **Grafana dashboard**: Visualize consensus state across all vantage points
- **PagerDuty/Slack integration**: Get notified immediately on Byzantine partitions

## Next Steps

- Read the [full documentation](byzantine-monitoring.md) for multi-cloud deployment
- Configure AlertManager routing for Byzantine alerts
- Deploy 5+ watchers for production (reduces false positives)

## Metrics Preview

```promql
# Divergence ratio (0.0 = all agree, 1.0 = complete disagreement)
stellar:watcher:divergence_ratio

# Active watcher count
stellar:watcher:active_count

# Per-watcher ledger sequence
stellar_watcher_ledger_sequence
```

## Alert Preview

```
🚨 StellarByzantinePartitionDetected
40% of watchers on mainnet see a different ledger hash than the majority.
This may indicate a network partition, eclipse attack, or consensus failure.

Divergence ratio: 40%
Threshold: 20%
```
