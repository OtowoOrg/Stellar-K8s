# Multi-Cluster Deployment Guide

Deploy and manage Stellar nodes across multiple Kubernetes clusters for high availability, geographic distribution, and fault isolation.

---

## Architecture Patterns

### Active-Active Federation

All clusters serve live traffic simultaneously. Each cluster runs a full set of Stellar nodes (Validator, Horizon, Soroban RPC). SCP quorum is configured to span clusters so that no single cluster failure can halt consensus.

```
┌─────────────────────┐     SCP / Peer     ┌─────────────────────┐
│   Cluster: us-east  │◄──────────────────►│   Cluster: eu-west  │
│  Validator (active) │                    │  Validator (active) │
│  Horizon (active)   │                    │  Horizon (active)   │
└─────────────────────┘                    └─────────────────────┘
          ▲                                          ▲
          │              Global LB                  │
          └──────────────────┬───────────────────────┘
                             │
                        Client Traffic
```

**When to use:** Maximum throughput, zero-downtime maintenance, geographic latency reduction.

### Active-Passive (Hot Standby)

One cluster is primary; the standby cluster is fully provisioned but receives no external traffic. Failover is automated via the operator's DR controller.

**When to use:** Cost-sensitive deployments that still need fast recovery (RTO < 5 min).

### Hub-and-Spoke

A central hub cluster runs the operator and shared services (Prometheus, Grafana, Kafka). Spoke clusters run only Stellar nodes and report metrics back to the hub.

**When to use:** Centralized observability with distributed node placement.

---

## Prerequisites

- Two or more Kubernetes clusters (1.28+), each with:
  - The Stellar-K8s operator installed (same version on all clusters)
  - A `StorageClass` with `allowVolumeExpansion: true`
  - Prometheus Operator or compatible metrics stack
- Network connectivity between clusters (see [Cross-Cluster Networking](#cross-cluster-networking))
- `kubectl` contexts configured for each cluster

---

## Cluster Federation Setup

### 1. Install the Operator on Each Cluster

```bash
# Install on cluster A
kubectl config use-context cluster-a
helm install stellar-operator stellar-k8s/stellar-operator \
  --namespace stellar-system \
  --create-namespace \
  --set clusterName=us-east

# Install on cluster B
kubectl config use-context cluster-b
helm install stellar-operator stellar-k8s/stellar-operator \
  --namespace stellar-system \
  --create-namespace \
  --set clusterName=eu-west
```

### 2. Create a Shared Seed Secret

Each cluster needs the same validator seed so that the validator identity is consistent across clusters. Store the seed in a Kubernetes Secret on each cluster:

```bash
# Create the secret on both clusters
for ctx in cluster-a cluster-b; do
  kubectl --context "$ctx" create secret generic my-validator-seed \
    --namespace stellar \
    --from-literal=seed="$STELLAR_VALIDATOR_SEED"
done
```

### 3. Deploy StellarNode Resources

Apply the same `StellarNode` manifest to each cluster. The `clusterRole` annotation tells the operator whether this instance is primary or standby:

```yaml
# validator-cluster-a.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-primary
  namespace: stellar
  annotations:
    stellar.org/cluster-role: "primary"
    stellar.org/cluster-name: "us-east"
spec:
  nodeType: Validator
  network: Mainnet
  version: "v21.0.0"
  storage:
    storageClass: "gp3"
    size: "500Gi"
    retentionPolicy: Retain
  validatorConfig:
    seedSecretRef: "my-validator-seed"
    enableHistoryArchive: true
  peers:
    # Explicitly list peers in the other cluster
    - host: "validator.eu-west.example.com"
      port: 11625
```

```bash
kubectl --context cluster-a apply -f validator-cluster-a.yaml
kubectl --context cluster-b apply -f validator-cluster-b.yaml
```

### 4. Configure Cross-Cluster Quorum

Edit `stellar-core.cfg` (managed via the operator's ConfigMap) to include validators from all clusters in the quorum set:

```toml
# Quorum spanning both clusters
[[VALIDATORS]]
NAME="us-east-validator"
HOME_DOMAIN="us-east.example.com"
PUBLIC_KEY="GABC..."
ADDRESS="validator.us-east.example.com:11625"
QUALITY="HIGH"

[[VALIDATORS]]
NAME="eu-west-validator"
HOME_DOMAIN="eu-west.example.com"
PUBLIC_KEY="GDEF..."
ADDRESS="validator.eu-west.example.com:11625"
QUALITY="HIGH"

[QUORUM_SET]
THRESHOLD_PERCENT=67
VALIDATORS=["$us-east-validator", "$eu-west-validator"]
```

---

## Cross-Cluster Networking

Choose one of the following approaches based on your infrastructure:

### Option A: External DNS + LoadBalancer Services

The simplest approach. Each cluster exposes the Stellar peer port (11625) via a `LoadBalancer` Service. ExternalDNS registers the IP in your DNS zone.

```yaml
# Peer service with ExternalDNS annotation
apiVersion: v1
kind: Service
metadata:
  name: validator-peer
  namespace: stellar
  annotations:
    external-dns.alpha.kubernetes.io/hostname: "validator.us-east.example.com"
spec:
  type: LoadBalancer
  selector:
    app: stellar-validator
  ports:
    - name: peer
      port: 11625
      targetPort: 11625
```

See [`examples/cross-cluster-external-dns.yaml`](../examples/cross-cluster-external-dns.yaml) for a complete example.

### Option B: Istio Service Mesh (mTLS)

For encrypted, authenticated cross-cluster traffic. Requires Istio installed on both clusters with a shared root CA.

```yaml
# ServiceEntry to reach the remote cluster's validator
apiVersion: networking.istio.io/v1beta1
kind: ServiceEntry
metadata:
  name: remote-validator
  namespace: stellar
spec:
  hosts:
    - validator.eu-west.svc.cluster.local
  location: MESH_EXTERNAL
  ports:
    - number: 11625
      name: tcp-peer
      protocol: TCP
  endpoints:
    - address: "10.20.0.5"  # Remote cluster node IP
```

See [`examples/cross-cluster-istio.yaml`](../examples/cross-cluster-istio.yaml) and [`docs/service-mesh.md`](service-mesh.md) for full setup.

### Option C: Submariner (Direct Pod-to-Pod)

Submariner creates an encrypted tunnel between cluster pod CIDRs, enabling direct pod IP routing across clusters.

```bash
# Install Submariner broker on cluster A
subctl deploy-broker --context cluster-a

# Join both clusters
subctl join --context cluster-a broker-info.subm --clusterid us-east
subctl join --context cluster-b broker-info.subm --clusterid eu-west
```

See [`examples/cross-cluster-submariner.yaml`](../examples/cross-cluster-submariner.yaml) for the full configuration.

---

## Load Balancing Configuration

### Global Load Balancer (Horizon API)

Use a global load balancer (AWS Global Accelerator, GCP Cloud Load Balancing, or Cloudflare) to distribute Horizon API traffic across clusters:

```yaml
# Horizon service on each cluster
apiVersion: v1
kind: Service
metadata:
  name: horizon-api
  namespace: stellar
  annotations:
    external-dns.alpha.kubernetes.io/hostname: "horizon.example.com"
    # AWS: register with Global Accelerator
    service.beta.kubernetes.io/aws-load-balancer-type: "nlb"
spec:
  type: LoadBalancer
  selector:
    app: stellar-horizon
  ports:
    - port: 8000
      targetPort: 8000
```

### Health-Based Routing

Configure your load balancer to route only to healthy, synced clusters. The operator exposes a `/healthz` endpoint that returns `200` only when the node is fully synced:

```
GET http://horizon.us-east.example.com:8000/healthz
→ 200 OK  (synced, serving traffic)

GET http://horizon.eu-west.example.com:8000/healthz
→ 503 Service Unavailable  (syncing, removed from rotation)
```

### Weighted Routing for Canary Upgrades

During upgrades, shift traffic gradually between clusters:

```yaml
# Example: Route 90% to cluster-a, 10% to cluster-b during upgrade
# (AWS Route 53 weighted records)
- Name: horizon.example.com
  Type: A
  AliasTarget: horizon.us-east.example.com
  Weight: 90

- Name: horizon.example.com
  Type: A
  AliasTarget: horizon.eu-west.example.com
  Weight: 10
```

---

## Failover Procedures

### Automatic Failover

The operator's DR controller monitors cross-cluster health and triggers failover automatically when the primary cluster becomes unhealthy. Enable it via the feature flag:

```yaml
# stellar-operator-config ConfigMap
data:
  enable_dr: "true"
```

Configure the DR policy:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-primary
  namespace: stellar
spec:
  drPolicy:
    enabled: true
    failoverThresholdSeconds: 120   # Declare failure after 2 min of no heartbeat
    targetCluster: "eu-west"
    autoFailover: true
```

### Manual Failover

To manually promote the standby cluster to primary:

```bash
# 1. Verify standby is healthy and synced
kubectl --context cluster-b stellar status

# 2. Scale down primary (prevents split-brain)
kubectl --context cluster-a scale statefulset validator-primary --replicas=0 -n stellar

# 3. Promote standby
kubectl --context cluster-b annotate stellarnode validator-primary \
  stellar.org/cluster-role=primary --overwrite

# 4. Update DNS / load balancer to point to cluster-b
# (update your DNS records or load balancer configuration)

# 5. Verify
kubectl --context cluster-b stellar status
```

### Failback Procedure

After the primary cluster is restored:

```bash
# 1. Ensure primary cluster is healthy
kubectl --context cluster-a stellar status

# 2. Let it sync to the current ledger (monitor with kubectl stellar status)

# 3. Demote current primary (cluster-b) to standby
kubectl --context cluster-b annotate stellarnode validator-primary \
  stellar.org/cluster-role=standby --overwrite

# 4. Promote cluster-a back to primary
kubectl --context cluster-a annotate stellarnode validator-primary \
  stellar.org/cluster-role=primary --overwrite

# 5. Restore DNS / load balancer
```

---

## Multi-Cluster Monitoring

### Federated Prometheus

Use Prometheus federation to aggregate metrics from all clusters into a central Prometheus instance:

```yaml
# Central Prometheus scrape config
scrape_configs:
  - job_name: 'federate-us-east'
    honor_labels: true
    metrics_path: '/federate'
    params:
      match[]:
        - '{job="stellar-operator"}'
        - '{job="stellar-node"}'
    static_configs:
      - targets: ['prometheus.us-east.example.com:9090']
        labels:
          cluster: 'us-east'

  - job_name: 'federate-eu-west'
    honor_labels: true
    metrics_path: '/federate'
    params:
      match[]:
        - '{job="stellar-operator"}'
        - '{job="stellar-node"}'
    static_configs:
      - targets: ['prometheus.eu-west.example.com:9090']
        labels:
          cluster: 'eu-west'
```

### Cross-Cluster Grafana Dashboard

Import the multi-cluster dashboard from `monitoring/grafana-dashboard.json`. Key panels:

| Panel | Description |
|-------|-------------|
| Cluster Health Overview | Per-cluster node availability and sync status |
| Cross-Cluster Ledger Lag | Ledger sequence difference between clusters |
| Peer Connectivity Matrix | Which validators are connected to which |
| Failover Events | Timeline of automatic and manual failovers |

### Alerting Rules

```yaml
groups:
  - name: multi_cluster
    rules:
      - alert: ClusterLedgerDivergence
        expr: |
          abs(
            stellar_node_ledger_sequence{cluster="us-east"}
            - stellar_node_ledger_sequence{cluster="eu-west"}
          ) > 100
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Clusters have diverged by more than 100 ledgers"

      - alert: ClusterUnreachable
        expr: up{job="stellar-operator"} == 0
        for: 2m
        labels:
          severity: critical
        annotations:
          summary: "Stellar operator in cluster {{ $labels.cluster }} is down"

      - alert: CrossClusterPeerLost
        expr: stellar_node_peer_count < 2
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Validator {{ $labels.name }} has fewer than 2 peers"
```

---

## Example Configurations

### Minimal Two-Cluster Setup

```yaml
# cluster-a: primary validator
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator
  namespace: stellar
  annotations:
    stellar.org/cluster-role: "primary"
spec:
  nodeType: Validator
  network: Mainnet
  version: "v21.0.0"
  storage:
    storageClass: "gp3"
    size: "500Gi"
  validatorConfig:
    seedSecretRef: "validator-seed"
    enableHistoryArchive: true
  peers:
    - host: "validator.eu-west.example.com"
      port: 11625
```

```yaml
# cluster-b: standby validator (identical spec, different annotation)
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator
  namespace: stellar
  annotations:
    stellar.org/cluster-role: "standby"
spec:
  nodeType: Validator
  network: Mainnet
  version: "v21.0.0"
  storage:
    storageClass: "gp3"
    size: "500Gi"
  validatorConfig:
    seedSecretRef: "validator-seed"
    enableHistoryArchive: true
  peers:
    - host: "validator.us-east.example.com"
      port: 11625
```

See [`examples/dr-setup.yaml`](../examples/dr-setup.yaml) for a complete disaster recovery configuration.

---

## Troubleshooting

### Validators Cannot Peer Across Clusters

1. Verify network connectivity:
   ```bash
   kubectl --context cluster-a exec -n stellar deploy/stellar-operator -- \
     nc -zv validator.eu-west.example.com 11625
   ```
2. Check firewall rules allow TCP 11625 between cluster egress IPs.
3. Confirm DNS resolves correctly from within the pod:
   ```bash
   kubectl --context cluster-a exec -n stellar <validator-pod> -- \
     nslookup validator.eu-west.example.com
   ```
4. Review Stellar Core logs for `PEER_CONNECT_FAILED` errors:
   ```bash
   kubectl --context cluster-a stellar logs validator -f | grep PEER
   ```

### Ledger Divergence Between Clusters

1. Check the ledger sequence on each cluster:
   ```bash
   kubectl --context cluster-a stellar status
   kubectl --context cluster-b stellar status
   ```
2. If one cluster is far behind, it may be catching up from history archives. Monitor with:
   ```bash
   kubectl --context cluster-b logs -n stellar <validator-pod> | grep "LedgerManager"
   ```
3. Ensure history archive URLs are accessible from both clusters.

### Failover Did Not Trigger Automatically

1. Verify `enable_dr: "true"` is set in the operator ConfigMap on the standby cluster.
2. Check the operator logs for DR controller activity:
   ```bash
   kubectl --context cluster-b logs -n stellar-system deploy/stellar-operator | grep "dr_"
   ```
3. Confirm `failoverThresholdSeconds` is not set too high.
4. Check that the standby cluster can reach the primary cluster's health endpoint.

### Split-Brain After Failover

If both clusters believe they are primary:

1. Immediately scale down one cluster's validators:
   ```bash
   kubectl --context cluster-a scale statefulset validator --replicas=0 -n stellar
   ```
2. Determine which cluster has the higher ledger sequence (it is the authoritative one).
3. Demote the lower-sequence cluster to standby and let it re-sync.
4. Review DR policy configuration to prevent recurrence.

---

## See Also

- [Disaster Recovery Failover](dr-failover.md)
- [Cross-Cloud Failover](cross-cloud-failover.md)
- [Service Mesh Setup](service-mesh.md)
- [mTLS Guide](mtls-guide.md)
- [Monitoring & Observability](../README.md#-monitoring--observability)
