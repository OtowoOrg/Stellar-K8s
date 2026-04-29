# Cross-Cloud Failover for Stellar Horizon Clusters

This document describes the multi-cloud failover architecture for Stellar Horizon API clusters, enabling seamless traffic shifting between cloud providers (AWS, GCP, Azure) during major provider outages.

## Overview

Cloud-level outages happen. To achieve 99.99% availability for the Horizon RPC layer, the operator supports automatic cross-cloud failover:

1. **Continuous health monitoring** of Horizon endpoints across all configured clouds
2. **Database synchronization** to keep standby clusters current
3. **Automatic GLB/DNS update** to shift traffic when the primary cloud fails
4. **Kubernetes Events** for full audit trail

## Architecture

```
                    ┌─────────────────────────────────────┐
                    │   Global Load Balancer               │
                    │   (Cloudflare / F5 / AWS GA)         │
                    │   horizon.stellar.example.com        │
                    └──────────┬──────────────┬────────────┘
                               │              │
                    ┌──────────▼──┐      ┌────▼──────────┐
                    │  AWS        │      │  GCP           │
                    │  us-east-1  │      │  us-central1   │
                    │  Horizon    │      │  Horizon       │
                    │  (Primary)  │      │  (Secondary)   │
                    └──────────┬──┘      └────┬───────────┘
                               │              │
                    ┌──────────▼──────────────▼────────────┐
                    │   PostgreSQL Logical Replication      │
                    │   (or CNPG Cross-Cluster Replica)     │
                    └──────────────────────────────────────┘
```

## Configuration

Add `crossCloudFailover` to your Horizon `StellarNode`:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: horizon-primary
  namespace: stellar
spec:
  nodeType: Horizon
  network: Mainnet
  version: "v2.31.0"
  horizonConfig:
    databaseSecretRef: horizon-db-secret
    enableIngest: true
    stellarCoreUrl: http://stellar-core:11626

  crossCloudFailover:
    enabled: true
    role: primary
    primaryCloudProvider: aws

    clouds:
      - cloudProvider: aws
        region: us-east-1
        endpoint: horizon-aws.stellar.example.com
        priority: 100
        enabled: true
      - cloudProvider: gcp
        region: us-central1
        endpoint: horizon-gcp.stellar.example.com
        priority: 90
        enabled: true
      - cloudProvider: azure
        region: eastus
        endpoint: horizon-azure.stellar.example.com
        priority: 80
        enabled: true

    globalLoadBalancer:
      provider: cloudflare
      hostname: horizon.stellar.example.com
      healthCheckPath: /health
      ttlSeconds: 60
      credentialsSecretRef: cloudflare-credentials

    databaseSync:
      method: logicalReplication
      replicationSlot: horizon_standby
      maxLagSeconds: 30
      standbyCredentialsSecretRef: horizon-standby-db-secret

    failureThreshold: 3
    healthCheckTimeoutSeconds: 5
    autoFailback: false
```

## GLB Provider Configuration

### Cloudflare

The operator uses the [external-dns](https://github.com/kubernetes-sigs/external-dns) pattern. Create a Kubernetes Secret with your Cloudflare credentials:

```bash
kubectl create secret generic cloudflare-credentials \
  --from-literal=CF_API_TOKEN=<your-api-token> \
  --from-literal=CF_ZONE_ID=<your-zone-id> \
  -n stellar
```

Configure external-dns with the Cloudflare provider and point it at your cluster. The operator creates/updates `DNSEndpoint` resources that external-dns syncs to Cloudflare.

### F5 BIG-IP

For F5 GTM/LTM, use [F5 CIS (Container Ingress Services)](https://clouddocs.f5.com/containers/latest/). The operator creates `VirtualServer` CRDs that CIS syncs to BIG-IP.

```bash
kubectl create secret generic f5-credentials \
  --from-literal=F5_USERNAME=admin \
  --from-literal=F5_PASSWORD=<password> \
  -n stellar
```

### AWS Global Accelerator

Use the [AWS Load Balancer Controller](https://kubernetes-sigs.github.io/aws-load-balancer-controller/) with `TargetGroupBinding`. The operator updates endpoint group weights via annotations.

## Database Synchronization

### PostgreSQL Logical Replication (Recommended)

Lowest RPO (~seconds). Requires `pg_logical` extension.

**On the primary database:**
```sql
-- Create replication slot
SELECT pg_create_logical_replication_slot('horizon_standby', 'pgoutput');

-- Create publication
CREATE PUBLICATION horizon_pub FOR ALL TABLES;
```

**On the standby database:**
```sql
-- Create subscription
CREATE SUBSCRIPTION horizon_sub
  CONNECTION 'host=primary-db.aws.example.com dbname=horizon user=replicator password=...'
  PUBLICATION horizon_pub
  WITH (slot_name = 'horizon_standby');
```

Monitor replication lag:
```sql
SELECT slot_name, confirmed_flush_lsn, pg_current_wal_lsn(),
       (pg_current_wal_lsn() - confirmed_flush_lsn) AS lag_bytes
FROM pg_replication_slots
WHERE slot_name = 'horizon_standby';
```

### CloudNativePG Cross-Cluster (CNPG)

If using CNPG for managed databases, configure a replica cluster:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: horizon-db-gcp
spec:
  instances: 2
  externalClusters:
    - name: horizon-db-aws
      connectionParameters:
        host: horizon-db-aws.stellar.svc.cluster.local
        user: streaming_replica
        dbname: horizon
      password:
        name: horizon-db-aws-replication-secret
        key: password
  bootstrap:
    recovery:
      source: horizon-db-aws
```

### Snapshot Restore

For higher RPO tolerance, use periodic VolumeSnapshot backups. The operator verifies snapshot freshness before allowing failover.

## Health Checks

The operator performs multi-probe health checks against each cloud's Horizon `/health` endpoint:

```json
{
  "status": "healthy",
  "core_latest_ledger": 50000000,
  "history_latest_ledger": 49999998,
  "core_synced": true
}
```

A cloud is considered **unhealthy** when `failureThreshold` consecutive checks fail within `healthCheckTimeoutSeconds`.

## Failover Sequence

```
1. Health check fails for primary cloud (AWS)
   └─ Consecutive failures reach failureThreshold (3)

2. Operator evaluates secondary clouds
   └─ Finds GCP healthy (priority 90)

3. Database sync verification
   └─ Checks replication lag < maxLagSeconds (30s)
   └─ If lag too high: abort failover, log warning

4. GLB/DNS update
   └─ Creates/updates DNSEndpoint resource
   └─ external-dns syncs to Cloudflare (TTL: 60s)

5. Status update
   └─ StellarNode.status.crossCloudFailoverStatus.failoverActive = true
   └─ StellarNode.status.crossCloudFailoverStatus.activeCloud = "gcp"

6. Kubernetes Event emitted
   └─ Reason: CrossCloudFailoverActivated
   └─ Message: "Cross-cloud failover activated. Traffic routed to: gcp"
```

## Failback

Failback is **manual by default** (`autoFailback: false`). This prevents flapping during intermittent outages.

To manually initiate failback after the primary cloud recovers:

```bash
# Verify primary cloud is healthy
curl https://horizon-aws.stellar.example.com/health

# Patch the StellarNode to trigger failback
kubectl patch stellarnode horizon-primary -n stellar \
  --type=merge \
  -p '{"spec":{"crossCloudFailover":{"primaryCloudProvider":"aws"}}}'
```

To enable automatic failback:
```yaml
crossCloudFailover:
  autoFailback: true
```

## Monitoring

The operator emits the following Kubernetes Events:

| Reason | Type | Description |
|--------|------|-------------|
| `CrossCloudFailoverActivated` | Normal | Failover triggered, traffic shifted |
| `CrossCloudFailbackCompleted` | Normal | Traffic restored to primary cloud |

Check failover status:
```bash
kubectl get stellarnode horizon-primary -n stellar \
  -o jsonpath='{.status.crossCloudFailoverStatus}' | jq .
```

Example output:
```json
{
  "currentRole": "primary",
  "failoverActive": true,
  "activeCloud": "gcp",
  "lastFailoverTime": "2026-04-24T14:32:00Z",
  "lastFailoverReason": "Primary cloud aws unhealthy",
  "cloudHealth": [
    {"cloudProvider": "aws", "healthy": false, "errorMessage": "3 consecutive failures"},
    {"cloudProvider": "gcp", "healthy": true, "latencyMs": 45},
    {"cloudProvider": "azure", "healthy": true, "latencyMs": 62}
  ]
}
```

## Multi-Cloud Recovery Plan

### RTO / RPO Targets

| Metric | Target | Notes |
|--------|--------|-------|
| RTO (Recovery Time Objective) | < 2 minutes | DNS TTL 60s + propagation |
| RPO (Recovery Point Objective) | < 30 seconds | Logical replication lag threshold |
| Availability Target | 99.99% | ~52 min/year downtime |

### Runbook: Cloud Provider Outage

**Detection** (automated, ~30s):
- Operator health checks detect primary cloud failure
- `failureThreshold` consecutive failures trigger evaluation

**Failover** (automated, ~60s):
1. Operator verifies secondary cloud health
2. Operator verifies DB replication lag < 30s
3. Operator updates GLB/DNS
4. DNS TTL expires (60s), traffic shifts

**Verification** (manual):
```bash
# Check Horizon is responding on secondary cloud
curl https://horizon-gcp.stellar.example.com/health

# Verify ledger is current
curl https://horizon.stellar.example.com/ledgers?order=desc&limit=1 | jq '.._embedded.records[0].sequence'

# Check operator events
kubectl get events -n stellar --field-selector reason=CrossCloudFailoverActivated
```

**Failback** (manual, after primary recovery):
```bash
# 1. Verify primary cloud is fully recovered
curl https://horizon-aws.stellar.example.com/health

# 2. Verify DB replication is caught up
# (check pg_replication_slots lag on primary)

# 3. Trigger failback via operator
kubectl patch stellarnode horizon-primary -n stellar \
  --type=json \
  -p '[{"op":"replace","path":"/spec/crossCloudFailover/primaryCloudProvider","value":"aws"}]'

# 4. Monitor events
kubectl get events -n stellar -w
```

### Runbook: Database Sync Failure

If failover is blocked due to replication lag:

```bash
# Check replication lag
kubectl exec -n stellar deploy/horizon-primary -- \
  psql $DATABASE_URL -c "
    SELECT slot_name, confirmed_flush_lsn,
           pg_current_wal_lsn() - confirmed_flush_lsn AS lag_bytes
    FROM pg_replication_slots WHERE slot_name = 'horizon_standby';"

# If lag is acceptable, temporarily increase threshold
kubectl patch stellarnode horizon-primary -n stellar \
  --type=merge \
  -p '{"spec":{"crossCloudFailover":{"databaseSync":{"maxLagSeconds":120}}}}'
```

### Runbook: GLB/DNS Update Failure

If the DNS update fails:

```bash
# Check external-dns logs
kubectl logs -n external-dns deploy/external-dns --tail=50

# Manually create DNSEndpoint
kubectl apply -f - <<EOF
apiVersion: externaldns.k8s.io/v1alpha1
kind: DNSEndpoint
metadata:
  name: horizon-failover
  namespace: stellar
spec:
  endpoints:
  - dnsName: horizon.stellar.example.com
    recordTTL: 60
    recordType: CNAME
    targets:
    - horizon-gcp.stellar.example.com
EOF
```

## Prerequisites

- Kubernetes 1.25+
- [external-dns](https://github.com/kubernetes-sigs/external-dns) deployed and configured for your DNS provider
- PostgreSQL 14+ with `pg_logical` extension (for logical replication)
- Network connectivity between cloud clusters (VPN, VPC peering, or public endpoints)
- Cloudflare/F5/AWS credentials stored in Kubernetes Secrets

## References

- [Cloudflare Load Balancing](https://developers.cloudflare.com/load-balancing/)
- [F5 Container Ingress Services](https://clouddocs.f5.com/containers/latest/)
- [AWS Global Accelerator](https://aws.amazon.com/global-accelerator/)
- [external-dns](https://github.com/kubernetes-sigs/external-dns)
- [CloudNativePG Cross-Cluster](https://cloudnative-pg.io/documentation/current/replica_cluster/)
- [PostgreSQL Logical Replication](https://www.postgresql.org/docs/current/logical-replication.html)
