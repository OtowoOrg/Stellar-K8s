# Stellar-K8s Disaster Recovery Failover Guide

## Overview

This document provides a **manual step-by-step procedure** for performing a disaster recovery (DR) failover between regions in a Stellar-K8s multi-region deployment.

**Scope**: Active-passive failover from Primary (e.g., `us-east-1`) to Secondary (e.g., `eu-west-1`) region.

**Assumptions**:
- 10 validators total: 5 Primary (quorum-capable), 5 Secondary.
- Cross-region peering: Submariner/BGP (docs/peer-discovery.md).
- Storage: CSI snapshots backed by Velero (docs/volume-snapshots.md).
- Horizon failover via DNS.
- Same Stellar network passphrase across regions.

**RTO**: ~15-30 min (quorum shift + DNS TTL).
**RPO**: Near-zero (async replication + snapshots).

**⚠️ WARNING**: Test in staging first! Use DR drills (controller/dr.rs).

## Prerequisites

1. **Cluster Health**:
   ```bash
   kubectl get stellarnode --all-namespaces -o wide
   # All Ready=True
   ```

2. **Quorum Status** (Primary holds quorum):
   ```bash
   kubectl get stellarnodes stellar.org/v1alpha1 default -o json | jq '.items[0].status.quorum'
   ```

3. **Backups**:
   - Velero snapshots: `velero snapshot get --selector=backup=stellar-daily`
   - Forensic snapshots: `kubectl get forensicsnapshots -A`

4. **DNS Ready**: Secondary Horizon endpoints registered, TTL ≤300s.

5. **Access**: Admin kubeconfig for both regions.

## Preparation (~5 min)

1. **Scale Secondary Validators** (pre-warm):
   ```yaml
   # secondary-region.yaml
   apiVersion: stellar.org/v1alpha1
   kind: StellarNode
   metadata:
     name: validators-secondary
     namespace: stellar-system
   spec:
     replicas: 7  # +2 buffer
   ```
   `kubectl apply -f secondary-region.yaml --context=secondary-kubeconfig`

2. **Sync Data** (if needed):
   ```bash
   # Promote latest snapshot if lag > RPO
   velero restore create --from-backup=stellar-daily-latest --namespace=stellar-system
   ```

3. **Notify Stakeholders**:
   - Announce 30min maintenance.
   - Monitor: Grafana (monitoring/grafana-dashboard.json).

## Failover Procedure (~10 min)

### Step 1: Quiesce Primary (~2 min)
```bash
# Graceful scale-down Primary validators
kubectl patch stellarnode validators-primary -p='{\"spec\":{\"replicas\":1}}' --context=primary-kubeconfig

# Wait for drain
kubectl wait --for=delete stellarpod --all --timeout=300s -n stellar-system --context=primary
```

### Step 2: Promote Secondary Quorum (~3 min)
```bash
# Adjust validator weights/peers (network.toml or CR)
kubectl patch stellarnode validators-secondary -p='{\"spec\":{\"quorumWeight\":100}}' --context=secondary

# Force peer-discovery refresh
kubectl annotate stellarnode validators-secondary peer-discovery.stellar.org/reload=true --context=secondary
```

### Step 3: Failover Horizon (~2 min)
```bash
# Update Route53/Global DNS
aws route53 change-resource-record-sets --hosted-zone-id Z123 --change-batch file://horizon-failover.json
```
`horizon-failover.json`:
```json
[
  {
    \"Action\": \"UPSERT\",
    \"ResourceRecordSet\": {
      \"Name\": \"horizon.stellar.example.com\",
      \"Type\": \"A\",
      \"SetIdentifier\": \"secondary\",
      \"Failover\": \"PRIMARY\",
      \"ResourceRecords\": [{\"Value\": \"203.0.113.10\"}]
    }
  }
]
```

### Step 4: Verify Network Connectivity
```bash
# From Secondary: Check quorum
kubectl logs -l app=stellar-core -n stellar-system --context=secondary | grep \"Quorum set\"

# Cross-region ping (Submariner)
kubectl exec -it debug -- ping peer-primary-ip --context=secondary
```

## Validation (~5-10 min)

1. **Health Checks**:
   ```
   kubectl get stellarnode --context=secondary  # Ready=True, quorum slice full
   curl -f https://horizon.stellar.example.com/health
   ```

2. **Workload Test**:
   ```bash
   # Submit tx, verify inclusion
   sdk-submit-tx --network testnet --source-account GA...  # From QUICK_START_HEALTH_CHECKS.md
   ```

3. **Observability**:
   - Grafana: Ledger lag <5, CPU <80%.
   - Events: `kubectl get events --sort-by=.lastTimestamp`.

**Success Criteria**:
- ✅ Quorum in Secondary.
- ✅ Horizon responsive.
- ✅ New tx processing.

## Rollback (~10 min)

If validation fails:

1. **Revert DNS**:
   ```bash
   aws route53 change-resource-record-sets --change-batch file://horizon-rollback.json
   ```

2. **Restore Primary**:
   ```bash
   kubectl patch stellarnode validators-primary -p='{\"spec\":{\"replicas\":5}}' --context=primary
   sleep 60; kubectl rollout status deployment/stellar-operator -n stellar-system --context=primary
   ```

3. **Demote Secondary**:
   ```bash
   kubectl patch stellarnode validators-secondary -p='{\"spec\":{\"quorumWeight\":0}}' --context=secondary
   ```

## Post-Failover

- Update SOPs with timings.
- Schedule failback.
- Run DR drill.

## Troubleshooting

| Issue | Command |
|-------|---------|
| Quorum stuck | `kubectl delete pod -l app=stellar-core --context=secondary` |
| Peer discovery fail | Check docs/peer-discovery.md BGP status |
| Snapshot restore | `velero restore describe` |

## References
- [Peer Discovery](peer-discovery.md)
- [Volume Snapshots](volume-snapshots.md)
- [QUICK_START_HEALTH_CHECKS](QUICK_START_HEALTH_CHECKS.md)
- Stellar Core: https://developers.stellar.org/docs/validators/admin-guide/quorum

**Last Updated**: $(date)
