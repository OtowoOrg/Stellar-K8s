# Byzantine Monitoring Setup

## Overview

A Stellar validator might believe it is in consensus while actually being network-partitioned — isolated from the rest of the network. Monitoring from a single vantage point (the cluster itself) cannot distinguish between "the network is fine" and "we are cut off from the network."

**Byzantine Monitoring** solves this by deploying lightweight `stellar-watcher` sidecars in multiple geographically dispersed cloud regions. Each watcher independently observes the Stellar network and reports the latest externalized ledger hash. A central Prometheus instance aggregates all watcher observations and fires an alert when more than **20%** of watchers disagree on the current ledger hash — a strong signal of a Byzantine partition.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Stellar Network                              │
│                                                                     │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐           │
│  │  Validator A │   │  Validator B │   │  Validator C │           │
│  └──────┬───────┘   └──────┬───────┘   └──────┬───────┘           │
└─────────┼───────────────────┼───────────────────┼───────────────────┘
          │                   │                   │
          │  HTTP :11626/info  │                   │
          ▼                   ▼                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                  stellar-watcher Sidecars                           │
│                                                                     │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐  │
│  │ watcher          │  │ watcher          │  │ watcher          │  │
│  │ aws/us-east-1    │  │ gcp/eu-west-1    │  │ azure/ap-south-1 │  │
│  │ :9101/metrics    │  │ :9101/metrics    │  │ :9101/metrics    │  │
│  └──────┬───────────┘  └──────┬───────────┘  └──────┬───────────┘  │
└─────────┼───────────────────────┼───────────────────┼───────────────┘
          │  Prometheus scrape    │                   │
          ▼                       ▼                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│              Central Prometheus + AlertManager                      │
│                                                                     │
│  stellar:watcher:divergence_ratio > 0.20                           │
│  → PagerDuty / Slack: "Byzantine partition detected on mainnet"    │
└─────────────────────────────────────────────────────────────────────┘
```

### How It Works

1. Each `stellar-watcher` pod polls `GET /info` on its configured Stellar Core endpoint every 10 seconds (configurable).
2. It extracts the latest externalized ledger sequence and hash from the response.
3. It exports these as Prometheus metrics on `:9101/metrics`.
4. Prometheus scrapes all watchers and evaluates recording rules that compute the **divergence ratio**: the fraction of watchers that see a different ledger hash than the majority.
5. If the divergence ratio exceeds **20%** for more than 1 minute, the `StellarByzantinePartitionDetected` alert fires.

---

## Metrics Reference

Each `stellar-watcher` instance exports the following metrics:

| Metric | Type | Description |
|--------|------|-------------|
| `stellar_watcher_ledger_sequence` | Gauge | Latest externalized ledger sequence |
| `stellar_watcher_ledger_hash` | Gauge | Always 1; `hash` label carries the hex-encoded ledger close hash |
| `stellar_watcher_consensus_view` | Gauge | 1 = EXTERNALIZED (in consensus), 0 = not synced |
| `stellar_watcher_poll_errors_total` | Counter | Cumulative failed polls |
| `stellar_watcher_last_poll_timestamp_seconds` | Gauge | Unix timestamp of last successful poll |
| `stellar_watcher_info` | Gauge | Always 1; carries `watcher_id`, `cloud`, `region`, `network`, `node_endpoint` labels |

All metrics carry these labels: `watcher_id`, `cloud`, `region`, `network`, `node_endpoint`.

### Aggregated Recording Rules

The `monitoring/byzantine-alerts.yaml` PrometheusRule defines these recording rules:

| Rule | Description |
|------|-------------|
| `stellar:watcher:active_count` | Number of active watchers per network |
| `stellar:watcher:hash_votes` | Watcher count per (network, hash) |
| `stellar:watcher:majority_hash_votes` | Vote count for the majority hash |
| `stellar:watcher:diverging_count` | Number of watchers on a non-majority hash |
| `stellar:watcher:divergence_ratio` | Fraction of diverging watchers (0.0–1.0) |

---

## Alerts

| Alert | Severity | Condition | Description |
|-------|----------|-----------|-------------|
| `StellarByzantinePartitionDetected` | **critical** | `divergence_ratio > 0.20` for 1m | >20% of watchers see a different ledger hash |
| `StellarByzantinePartitionWarning` | warning | `divergence_ratio > 0.10` for 2m | Early warning — approaching threshold |
| `StellarByzantineWatcherCoverageInsufficient` | warning | `active_count < 3` for 5m | Too few watchers for reliable detection |
| `StellarByzantineWatcherStale` | warning | Last poll > 120s ago | A watcher has stopped polling |
| `StellarByzantineWatcherPollErrors` | warning | Error rate > 0.1/s for 5m | A watcher is failing to reach its node |

---

## Deployment

### Prerequisites

- Kubernetes cluster with Helm 3.x
- Prometheus Operator (kube-prometheus-stack or standalone)
- At least 3 geographically dispersed Stellar Core nodes (or access to public endpoints)
- The `stellar-operator` Helm chart installed

### Step 1: Configure Watcher Regions

Edit your `values.yaml` (or create a `byzantine-values.yaml` override):

```yaml
byzantineWatcher:
  enabled: true
  defaultNetwork: "mainnet"
  pollIntervalSecs: 10
  requestTimeoutSecs: 5
  metricsPort: 9101
  logLevel: "info"

  resources:
    limits:
      cpu: 100m
      memory: 64Mi
    requests:
      cpu: 25m
      memory: 32Mi

  regions:
    # AWS — US East
    - cloud: aws
      region: us-east-1
      nodeEndpoint: http://stellar-core.stellar-system.svc.cluster.local:11626

    # AWS — US West
    - cloud: aws
      region: us-west-2
      nodeEndpoint: http://stellar-core-usw2.stellar-system.svc.cluster.local:11626

    # GCP — Europe
    - cloud: gcp
      region: eu-west-1
      nodeEndpoint: http://stellar-core-eu.stellar-system.svc.cluster.local:11626

    # Azure — Asia Pacific
    - cloud: azure
      region: ap-south-1
      nodeEndpoint: http://stellar-core-ap.stellar-system.svc.cluster.local:11626

    # On-premises — Frankfurt DC
    - cloud: on-prem
      region: dc-frankfurt
      nodeEndpoint: http://10.0.1.50:11626
```

### Step 2: Deploy the Watchers

```bash
helm upgrade stellar-operator charts/stellar-operator \
  --namespace stellar-system \
  --values byzantine-values.yaml
```

Verify the watcher pods are running:

```bash
kubectl get pods -n stellar-system -l app=stellar-watcher
# NAME                                                    READY   STATUS    RESTARTS   AGE
# stellar-operator-watcher-us-east-1-7d9f8b6c4-xk2p9    1/1     Running   0          2m
# stellar-operator-watcher-eu-west-1-5c8d7a3b2-mn4q8    1/1     Running   0          2m
# stellar-operator-watcher-ap-south-1-9e6f5c1a0-pj7r6   1/1     Running   0          2m
```

Check metrics are being exported:

```bash
kubectl port-forward -n stellar-system \
  deployment/stellar-operator-watcher-us-east-1 9101:9101

curl http://localhost:9101/metrics | grep stellar_watcher
```

### Step 3: Apply the PrometheusRule

```bash
kubectl apply -f monitoring/byzantine-alerts.yaml -n monitoring
```

Verify the rule is loaded:

```bash
kubectl get prometheusrule -n monitoring stellar-byzantine-monitoring
```

### Step 4: Import the Grafana Dashboard

1. Open Grafana → Dashboards → Import
2. Upload `monitoring/byzantine-dashboard.json`
3. Select your Prometheus data source
4. Click **Import**

The dashboard shows:
- Real-time divergence ratio gauge per network
- Active watcher count
- Byzantine alert status
- Per-watcher consensus view (synced/not synced)
- Ledger sequence timeline per watcher
- Watcher health (poll age, error rate)

### Step 5: Configure AlertManager

Add a receiver for Byzantine alerts in your AlertManager config:

```yaml
# alertmanager.yaml
route:
  group_by: ['alertname', 'network']
  routes:
    - match:
        component: byzantine-monitoring
        severity: critical
      receiver: pagerduty-byzantine
    - match:
        component: byzantine-monitoring
        severity: warning
      receiver: slack-stellar-infra

receivers:
  - name: pagerduty-byzantine
    pagerduty_configs:
      - service_key: <your-pagerduty-key>
        description: '{{ .CommonAnnotations.summary }}'
        details:
          network: '{{ .CommonLabels.network }}'
          divergence_ratio: '{{ .CommonAnnotations.description }}'

  - name: slack-stellar-infra
    slack_configs:
      - api_url: <your-slack-webhook>
        channel: '#stellar-alerts'
        title: '{{ .CommonAnnotations.summary }}'
        text: '{{ .CommonAnnotations.description }}'
```

---

## Multi-Cloud Deployment (Advanced)

For true geographic diversity, deploy watchers as standalone containers outside Kubernetes using the `stellar-watcher` binary directly.

### Docker (any cloud VM)

```bash
docker run -d \
  --name stellar-watcher-eu \
  --restart unless-stopped \
  -p 9101:9101 \
  -e WATCHER_ID=watcher-gcp-eu-west-1 \
  -e WATCHER_CLOUD=gcp \
  -e WATCHER_REGION=eu-west-1 \
  -e WATCHER_NETWORK=mainnet \
  -e WATCHER_NODE_ENDPOINT=http://10.0.2.50:11626 \
  -e WATCHER_POLL_INTERVAL=10 \
  -e RUST_LOG=info \
  ghcr.io/stellar/stellar-k8s:latest \
  /stellar-watcher
```

### AWS ECS Task Definition

```json
{
  "family": "stellar-watcher",
  "containerDefinitions": [
    {
      "name": "watcher",
      "image": "ghcr.io/stellar/stellar-k8s:latest",
      "command": ["/stellar-watcher"],
      "environment": [
        { "name": "WATCHER_ID", "value": "watcher-aws-us-east-1" },
        { "name": "WATCHER_CLOUD", "value": "aws" },
        { "name": "WATCHER_REGION", "value": "us-east-1" },
        { "name": "WATCHER_NETWORK", "value": "mainnet" },
        { "name": "WATCHER_NODE_ENDPOINT", "value": "http://10.0.1.50:11626" },
        { "name": "WATCHER_POLL_INTERVAL", "value": "10" },
        { "name": "RUST_LOG", "value": "info" }
      ],
      "portMappings": [
        { "containerPort": 9101, "protocol": "tcp" }
      ],
      "healthCheck": {
        "command": ["CMD-SHELL", "curl -f http://localhost:9101/healthz || exit 1"],
        "interval": 30,
        "timeout": 5,
        "retries": 3
      },
      "cpu": 64,
      "memory": 64
    }
  ]
}
```

### Prometheus Scrape Config (for standalone watchers)

Add to your Prometheus `scrape_configs`:

```yaml
scrape_configs:
  - job_name: stellar-byzantine-watchers
    static_configs:
      - targets:
          - watcher-us-east-1.example.com:9101
          - watcher-eu-west-1.example.com:9101
          - watcher-ap-south-1.example.com:9101
        labels:
          job: stellar-byzantine-watcher
    relabel_configs:
      - source_labels: [__address__]
        target_label: instance
```

---

## Runbook

### Alert: `StellarByzantinePartitionDetected`

**Severity:** Critical  
**Condition:** >20% of watchers see a different ledger hash for >1 minute

#### Immediate Actions (first 5 minutes)

1. **Check the Grafana dashboard** — identify which watchers are diverging and from which regions.

2. **Compare ledger hashes** across all watchers:
   ```bash
   # Query Prometheus for current hash per watcher
   curl -s 'http://prometheus:9090/api/v1/query' \
     --data-urlencode 'query=stellar_watcher_ledger_hash == 1' \
     | jq '.data.result[] | {watcher: .metric.watcher_id, hash: .metric.hash}'
   ```

3. **Check Stellar Core directly** on each validator:
   ```bash
   # For each validator pod
   kubectl exec -n stellar-system <validator-pod> -- \
     curl -s http://localhost:11626/info | jq '.info.ledger'
   ```

4. **Check network connectivity** between regions:
   ```bash
   # From a diverging watcher pod
   kubectl exec -n stellar-system <watcher-pod> -- \
     curl -v http://<stellar-core-endpoint>:11626/info
   ```

5. **Check Stellar network status**: https://dashboard.stellar.org

#### Investigation (5–30 minutes)

6. **Identify the partition boundary** — which regions see hash A vs hash B?

7. **Check peer connectivity** on affected validators:
   ```bash
   kubectl exec -n stellar-system <validator-pod> -- \
     curl -s http://localhost:11626/peers | jq '.authenticated_peers | length'
   ```

8. **Check SCP state** on affected validators:
   ```bash
   kubectl exec -n stellar-system <validator-pod> -- \
     curl -s 'http://localhost:11626/scp?limit=1' | jq '.[0].phase'
   ```

9. **Review validator logs** for partition indicators:
   ```bash
   kubectl logs -n stellar-system <validator-pod> --since=10m \
     | grep -E "EXTERNALIZE|partition|timeout|disconnect"
   ```

#### Resolution

- If a **network partition** is confirmed: restore connectivity between the isolated segment and the majority.
- If an **eclipse attack** is suspected: rotate peer connections and check firewall rules.
- If a **software bug** is suspected: check for recent upgrades and consider rolling back.
- If a **false positive**: verify the watcher's `nodeEndpoint` is correct and the node is healthy.

---

### Alert: `StellarByzantineWatcherCoverageInsufficient`

**Condition:** Fewer than 3 active watchers

1. Check watcher pod status:
   ```bash
   kubectl get pods -n stellar-system -l app=stellar-watcher
   ```

2. Check watcher logs for errors:
   ```bash
   kubectl logs -n stellar-system -l app=stellar-watcher --tail=50
   ```

3. Deploy additional watchers by adding regions to `byzantineWatcher.regions` in values.yaml.

---

### Alert: `StellarByzantineWatcherStale`

**Condition:** A watcher has not polled successfully in >2 minutes

1. Check the specific watcher pod:
   ```bash
   kubectl logs -n stellar-system \
     -l "stellar.org/watcher-region=<region>" --tail=50
   ```

2. Verify the Stellar Core endpoint is reachable from the watcher pod:
   ```bash
   kubectl exec -n stellar-system <watcher-pod> -- \
     curl -v http://<node-endpoint>:11626/info
   ```

3. Check NetworkPolicy — ensure the watcher can reach port 11626 on the Stellar Core pod.

---

## Security Considerations

- Watchers only make **outbound HTTP GET** requests to Stellar Core's HTTP API (port 11626). They do not require any Kubernetes RBAC permissions.
- The `/info` endpoint on Stellar Core is **read-only** and does not expose sensitive data.
- Watcher pods run as **non-root** (UID 65532) with a read-only root filesystem.
- The metrics endpoint (`:9101/metrics`) should be accessible only within the cluster or via a secured Prometheus scrape path. Do not expose it publicly.
- For cross-cloud deployments, use a VPN or private network to connect watchers to Stellar Core nodes rather than exposing port 11626 to the internet.

---

## Minimum Viable Setup

For a quick start with 3 watchers in a single cluster (useful for testing):

```yaml
byzantineWatcher:
  enabled: true
  regions:
    - cloud: local
      region: zone-a
      nodeEndpoint: http://stellar-core-0.stellar-system.svc.cluster.local:11626
    - cloud: local
      region: zone-b
      nodeEndpoint: http://stellar-core-1.stellar-system.svc.cluster.local:11626
    - cloud: local
      region: zone-c
      nodeEndpoint: http://stellar-core-2.stellar-system.svc.cluster.local:11626
```

> **Note:** Single-cluster watchers cannot detect cluster-level partitions. For production Byzantine monitoring, deploy watchers in at least 3 different cloud providers or data centers.

---

## FAQ

**Q: Why 20% as the alert threshold?**  
A: The Stellar Consensus Protocol (SCP) requires a quorum of validators to agree. A 20% divergence is a strong signal that something is wrong — either a partition, an eclipse attack, or a consensus failure. The threshold is configurable via the PrometheusRule if your network has different requirements.

**Q: Can a single diverging watcher cause a false positive?**  
A: With 5+ watchers, a single diverging watcher represents 20% — which is exactly at the threshold (not above it), so no alert fires. With 4 watchers, 1 diverging = 25% → alert fires. We recommend deploying at least 5 watchers to avoid false positives from a single misbehaving watcher.

**Q: What if the Stellar Core node I'm pointing watchers at is itself partitioned?**  
A: That's exactly what this system detects. If some watchers can reach the node and see hash A, while others see hash B (or can't reach it at all), the divergence ratio will exceed 20% and the alert fires.

**Q: Does this work for Testnet?**  
A: Yes. Set `defaultNetwork: "testnet"` and point `nodeEndpoint` at your Testnet Stellar Core nodes.

**Q: How does this interact with the existing quorum analysis?**  
A: The existing quorum analyzer (`src/controller/quorum/`) analyzes the *internal* quorum topology from within the cluster. Byzantine monitoring is *external* — it observes the network from multiple geographic vantage points. They are complementary: quorum analysis detects fragility in the quorum set configuration, while Byzantine monitoring detects active partitions.
