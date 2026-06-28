# Grafana Dashboard Guide

This guide explains how to use the comprehensive Grafana dashboards provided for monitoring Stellar nodes in Kubernetes.

## Available Dashboards

The stellar-k8s operator includes four specialized Grafana dashboards:

1. **Validator Dashboard** (`grafana-validator-dashboard.json`) - SCP consensus and validator metrics
2. **Horizon Dashboard** (`grafana-horizon-dashboard.json`) - API performance and ingestion metrics
3. **Soroban RPC Dashboard** (`grafana-soroban-rpc-dashboard.json`) - Smart contract execution metrics
4. **Operator Health Dashboard** (`grafana-operator-health-dashboard.json`) - Operator performance metrics

## Installation

### Prerequisites

- Grafana 9.0+ installed in your Kubernetes cluster
- Prometheus datasource configured in Grafana
- stellar-k8s operator deployed and exporting metrics

### Import Dashboards

#### Method 1: Grafana UI

1. Open Grafana web interface
2. Navigate to **Dashboards** → **Import**
3. Click **Upload JSON file**
4. Select one of the dashboard JSON files from `monitoring/` directory
5. Select your Prometheus datasource
6. Click **Import**

#### Method 2: ConfigMap (GitOps)

```bash
# Create ConfigMap for each dashboard
kubectl create configmap grafana-stellar-validator-dashboard \
  --from-file=monitoring/grafana-validator-dashboard.json \
  -n monitoring

# Add label for Grafana sidecar auto-discovery
kubectl label configmap grafana-stellar-validator-dashboard \
  grafana_dashboard=1 \
  -n monitoring
```

#### Method 3: Helm Values (if using Grafana Helm chart)

```yaml
# values.yaml
dashboardProviders:
  dashboardproviders.yaml:
    apiVersion: 1
    providers:
    - name: 'stellar'
      orgId: 1
      folder: 'Stellar'
      type: file
      disableDeletion: false
      editable: true
      options:
        path: /var/lib/grafana/dashboards/stellar

dashboards:
  stellar:
    validator:
      url: https://raw.githubusercontent.com/stellar/stellar-k8s/main/monitoring/grafana-validator-dashboard.json
    horizon:
      url: https://raw.githubusercontent.com/stellar/stellar-k8s/main/monitoring/grafana-horizon-dashboard.json
    soroban:
      url: https://raw.githubusercontent.com/stellar/stellar-k8s/main/monitoring/grafana-soroban-rpc-dashboard.json
    operator:
      url: https://raw.githubusercontent.com/stellar/stellar-k8s/main/monitoring/grafana-operator-health-dashboard.json
```

## Dashboard Overview

### 1. Validator Dashboard

**Purpose**: Monitor Stellar validator nodes and SCP consensus performance

**Key Panels**:
- **Validator Status**: Real-time health status (Up/Down)
- **Ledger Close Time**: p50, p95, p99 percentiles for ledger close timing
- **Transaction Throughput**: Current TPS (transactions per second)
- **SCP Consensus Phase Timing**: Breakdown of nomination, prepare, commit, and externalize phases
- **Peer Connections**: Authenticated and pending peer counts
- **Database Size**: Current database size and growth trends
- **Archive Health Status**: History archive health indicators
- **Quorum Failures**: Critical alert for quorum intersection failures
- **Transaction Success Rate**: Percentage of successful transactions
- **Peer Message Latency**: Network latency to peers (p50, p95, p99)

**Variables**:
- `$datasource`: Prometheus datasource selector
- `$node`: Multi-select validator node filter

**Recommended Alerts**:
- Ledger close time p99 > 10s
- Quorum intersection failures > 0
- Transaction success rate < 95%
- Peer connection count < 3

### 2. Horizon Dashboard

**Purpose**: Monitor Horizon API servers and ingestion performance

**Key Panels**:
- **API Request Rate**: Requests per second to Horizon API
- **API Latency**: Request duration percentiles (p50, p95, p99)
- **Ingestion Lag**: Seconds behind the network
- **DB Replication Lag**: Database replication delay
- **Database Connection Pool**: Active vs idle connections
- **Transaction Throughput**: TPS processed by Horizon
- **Database Size**: Storage usage and growth

**Variables**:
- `$datasource`: Prometheus datasource selector
- `$node`: Multi-select Horizon node filter

**Recommended Alerts**:
- Ingestion lag > 60s
- API latency p95 > 1s
- DB replication lag > 10s
- Connection pool exhaustion

### 3. Soroban RPC Dashboard

**Purpose**: Monitor Soroban RPC nodes and smart contract execution

**Key Panels**:
- **Contract Invocation Rate**: Smart contract calls per second
- **Contract Execution Time**: Execution duration percentiles (p50, p95, p99)
- **WASM Cache Hit Rate**: Percentage of WASM cache hits
- **Host Function Call Rate**: Rate of host function invocations
- **WASM Cache Performance**: Hits vs misses over time
- **Database Size**: Storage usage for Soroban state
- **Transaction Throughput**: TPS for Soroban transactions

**Variables**:
- `$datasource`: Prometheus datasource selector
- `$node`: Multi-select Soroban RPC node filter

**Recommended Alerts**:
- Contract execution time p99 > 500ms
- WASM cache hit rate < 70%
- Database size approaching limits

### 4. Operator Health Dashboard

**Purpose**: Monitor the stellar-k8s operator itself

**Key Panels**:
- **Reconciliation Rate**: Controller reconciliation frequency
- **Reconciliation Duration**: Time spent in reconciliation loops (p50, p95, p99)
- **Error Rate by Type**: Errors categorized by controller and error kind
- **Managed Nodes by Type**: Count of validators, Horizon, and Soroban nodes
- **Operator CPU Usage**: CPU consumption
- **Operator Memory Usage**: Memory consumption

**Variables**:
- `$datasource`: Prometheus datasource selector

**Recommended Alerts**:
- Reconciliation duration p99 > 5s
- Error rate > 0.1 errors/sec
- CPU usage > 80%
- Memory usage approaching limits

## Dashboard Customization

### Adding Custom Panels

1. Click **Add panel** in dashboard edit mode
2. Select **Add a new panel**
3. Choose visualization type
4. Configure query using metrics from [STELLAR_METRICS_GUIDE.md](../metrics/STELLAR_METRICS_GUIDE.md)
5. Set thresholds and alerts as needed
6. Save panel

### Creating Custom Variables

```json
{
  "name": "network",
  "type": "query",
  "datasource": "${datasource}",
  "query": "label_values(stellar_transaction_throughput_tps, network)",
  "multi": true,
  "includeAll": true
}
```

### Example Custom Queries

**Average TPS across all validators in a network**:
```promql
avg(stellar_transaction_throughput_tps{network="$network",node_type="validator"})
```

**Database growth rate (GB per day)**:
```promql
stellar_database_growth_rate_bytes_per_hour * 24 / 1024 / 1024 / 1024
```

**Peer connection stability (uptime > 1 hour)**:
```promql
count(stellar_peer_connection_uptime_seconds > 3600) by (node_name)
```

## Alert Configuration

### Prometheus Alert Rules

Create alert rules in Prometheus to trigger notifications:

```yaml
# monitoring/stellar-alerts.yaml
groups:
  - name: stellar_validator_alerts
    interval: 30s
    rules:
      - alert: HighLedgerCloseTime
        expr: stellar_ledger_close_time_p99 > 10
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High ledger close time on {{ $labels.node_name }}"
          description: "Ledger close time p99 is {{ $value }}s (threshold: 10s)"
      
      - alert: QuorumIntersectionFailure
        expr: rate(stellar_scp_quorum_intersection_failures[5m]) > 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Quorum intersection failure on {{ $labels.node_name }}"
          description: "Critical consensus issue detected"
      
      - alert: HighHorizonIngestionLag
        expr: stellar_horizon_ingestion_lag_seconds > 60
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Horizon ingestion lag on {{ $labels.node_name }}"
          description: "Ingestion is {{ $value }}s behind the network"
      
      - alert: LowWASMCacheHitRate
        expr: |
          rate(stellar_soroban_wasm_cache_hits[5m]) / 
          (rate(stellar_soroban_wasm_cache_hits[5m]) + rate(stellar_soroban_wasm_cache_misses[5m])) < 0.7
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "Low WASM cache hit rate on {{ $labels.node_name }}"
          description: "Cache hit rate is {{ $value | humanizePercentage }}"
```

### Grafana Alerts

Configure alerts directly in Grafana panels:

1. Edit panel
2. Go to **Alert** tab
3. Click **Create alert rule from this panel**
4. Set conditions and thresholds
5. Configure notification channels
6. Save alert

## Best Practices

### Dashboard Organization

1. **Use Folders**: Organize dashboards by environment (prod, staging, dev)
2. **Naming Convention**: Use consistent naming: `Stellar - [Node Type] - [Environment]`
3. **Tags**: Add tags for easy discovery: `stellar`, `validator`, `horizon`, `soroban`
4. **Permissions**: Set appropriate view/edit permissions per team

### Performance Optimization

1. **Time Range**: Use appropriate time ranges (1h for real-time, 24h for trends)
2. **Refresh Rate**: Set to 10s-30s for live monitoring, 1m+ for historical analysis
3. **Query Optimization**: Use recording rules for complex queries
4. **Panel Limits**: Limit number of series per panel to avoid browser slowdown

### Monitoring Strategy

1. **Start with Overview**: Use Operator Health dashboard to see overall system health
2. **Drill Down**: Navigate to specific node type dashboards for detailed analysis
3. **Correlate Metrics**: Compare metrics across dashboards to identify root causes
4. **Set Baselines**: Establish normal operating ranges for your environment
5. **Regular Review**: Review dashboards weekly to identify trends

## Troubleshooting

### No Data Displayed

**Problem**: Panels show "No data"

**Solutions**:
1. Verify Prometheus datasource is configured correctly
2. Check that stellar-k8s operator is running and exporting metrics
3. Verify Prometheus is scraping the operator metrics endpoint
4. Check time range and refresh settings
5. Verify label selectors match your deployment

```bash
# Check if metrics are being exported
kubectl port-forward -n stellar-system svc/stellar-operator 9090:9090
curl http://localhost:9090/metrics | grep stellar_
```

### High Cardinality Issues

**Problem**: Grafana/Prometheus performance degradation

**Solutions**:
1. Limit peer_id label cardinality by aggregating peer metrics
2. Use recording rules for frequently queried metrics
3. Adjust Prometheus retention and scrape intervals
4. Filter dashboards by namespace or network

### Dashboard Import Errors

**Problem**: Dashboard fails to import

**Solutions**:
1. Verify Grafana version compatibility (9.0+)
2. Check JSON syntax validity
3. Ensure datasource UID matches your Prometheus datasource
4. Update dashboard version if using older Grafana

## Integration with Alertmanager

Configure Alertmanager to route alerts to appropriate channels:

```yaml
# alertmanager.yaml
route:
  group_by: ['alertname', 'cluster', 'service']
  group_wait: 10s
  group_interval: 10s
  repeat_interval: 12h
  receiver: 'stellar-team'
  routes:
    - match:
        severity: critical
      receiver: 'pagerduty'
    - match:
        severity: warning
      receiver: 'slack'

receivers:
  - name: 'stellar-team'
    email_configs:
      - to: 'stellar-ops@example.com'
  
  - name: 'pagerduty'
    pagerduty_configs:
      - service_key: '<pagerduty-key>'
  
  - name: 'slack'
    slack_configs:
      - api_url: '<slack-webhook-url>'
        channel: '#stellar-alerts'
```

## Related Documentation

- [Stellar Metrics Guide](../metrics/STELLAR_METRICS_GUIDE.md)
- Prometheus Alert Rules
- Operator Configuration

## Support

For issues or questions:
- GitHub Issues: https://github.com/OtowoOrg/Stellar-K8s/issues
- Documentation: https://github.com/OtowoOrg/Stellar-K8s/tree/main/docs
