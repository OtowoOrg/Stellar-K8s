# Multi-Cluster Federation & Automated Failover

> Issue #1409 — multi-cluster federation for high availability.

This document describes how Stellar-K8s federates two (or more) clusters into a
single logical Stellar deployment, synchronises secrets/configuration across
cluster boundaries, and fails over automatically when the active region becomes
unhealthy.

The federation state is declared with a `StellarFederation` CR (see
`config/samples/stellar-federation.yaml`), reconciled by the federation
controllers (`src/controller/cross_cluster.rs`, `cross_region_sync.rs`,
`cross_cloud_failover.rs`). The overall DR design (state streaming, RPO/RTO)
is documented in [dr-failover.md](dr-failover.md).

## 1. Cross-Cluster Secret & Configuration Synchronization

Every federated cluster carries a `stellar-federation-<region>` Secret in
`stellar-system` holding that cluster's kubeconfig; the controller uses these
to reach the peers when computing replication lag and during failover.

To keep those Secrets (or validator seed Secrets) in sync at bootstrap and
whenever they rotate:

```bash
./scripts/sync-federation-secrets.sh \
  --source-cluster us-east-1 \
  --target-cluster eu-west-1 \
  --name stellar-federation-us-east-1 \
  --namespace stellar-system

# dry-run first, then apply
./scripts/sync-federation-secrets.sh --source-cluster us-east-1 \
  --target-cluster eu-west-1 --name validator-seed --dry-run
```

The script strips server-managed fields and applies idempotently on the target
context, so it is safe to run from cron/CI (e.g. as a `CronJob` seeded from a
federation sync Schedule).

## 2. Automated Failover with Health-Check Based Routing

The `StellarFederation` spec configures failover behaviour:

| Field | Purpose |
|-------|---------|
| `spec.trafficRouting.strategy` | `Geographic` / `RoundRobin` / `LeastConnections` — how client traffic is routed across regions |
| `spec.trafficRouting.healthCheckTimeoutSecs` | Per-probe timeout for the active region health endpoint |
| `spec.healthCheckIntervalSecs` | How often probes run (default 30s) |
| `spec.failoverDetectionSecs` | Consecutive-miss window that triggers failover (default 30s) |
| `spec.replication.mode` / `.replicationLagThresholdSecs` | Async sync with a lag budget; failover is blocked while the standby lags beyond the threshold |

Sequence on primary failure:

1. Health checks observe the active region missing `healthCheckIntervalSecs`
   probes for `failoverDetectionSecs`.
2. The controller re-weights `trafficRouting` to the healthy standby (or
   switches a Geographic DNS mapping in front of the API endpoint).
3. Replication lag is verified `< replicationLagThresholdSecs` before the
   standby is promoted, preserving the zero-RPO guarantee.
4. `status.phase` / `status.activeRegions` / `status.failedRegions` reflect the
   transition; `lastSyncTime` and `replicationLagMs` are exported for alerts.

## 3. Failover Procedures

**Automated failover (expected path)**
1. Confirm `kubectl get stellarfederation stellar-global -n stellar-system` shows
   `status.failedRegions: [<primary>]` and `activeRegions: [<standby>]`.
2. Verify ledger height on the standby is at/above the primary's last synced
   height (`stellar-logs` / the `replicationLagMs` metric).
3. Re-point any external DNS/horizon endpoints to the standby region.
4. Open a post-incident review; do NOT fall back automatically before the
   primary region is re-verified.

**Manual failover (drill / deliberate maintenance)**
1. Put primary region in maintenance (`kubectl drain`), which triggers the
   same health-based promotion.
2. Confirm standby authenticity (validator seed Secret synced, archive intact).
3. Update the federation weights (`spec.clusters[].weight`) as needed.

## 4. RTO / RPO Targets

| Metric | Target |
|--------|--------|
| **RTO** (Automated failover) | 5–10 minutes (probe detection 30s + promotion) |
| **RTO** (Manual) | 15–30 minutes |
| **RPO** | Zero — asynchronous ledger streaming with lag ≤ 60s (threshold `spec.replication.replicationLagThresholdSecs`) |

These targets are exercised monthly by the disaster-recovery drills
(`docs/chaos-drills.md`, `.github/workflows/dr-drill.yml`).

## 5. Related

- [DR Failover Guide](./dr-failover.md)
- [Cross-Cloud Failover](./cross-cloud-failover.md)
- [Multi-Region DR](./deployment-patterns/multi-region-dr.md)