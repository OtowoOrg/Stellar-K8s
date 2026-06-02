# Stellar-K8s Disaster Recovery Failover Guide

## Overview

This document describes the **automated cross-region state synchronization** system
and the **manual failover procedure** for Stellar-K8s multi-region deployments.

**Scope**: Active-passive failover from Primary (e.g., `us-east-1`) to Secondary
(e.g., `eu-west-1`) with zero-RPO via continuous ledger state streaming.

**Architecture**:
```
Primary Region (us-east-1)                 Standby Region (eu-west-1)
┌──────────────────────────────┐           ┌──────────────────────────────┐
│  StellarNode Pod             │           │  StellarNode Pod             │
│  ┌──────────┐  ┌──────────┐ │  bridge   │  ┌──────────┐  ┌──────────┐ │
│  │  core    │→ │state-sync│ │──────────▶│  │state-sync│→ │  core    │ │
│  │ :11626   │  │ sidecar  │ │           │  │ sidecar  │  │ standby  │ │
│  └──────────┘  └────┬─────┘ │           │  └────┬─────┘  └──────────┘ │
│                     │       │           │       │                      │
│              ConfigMap       │           │  reads ConfigMap via bridge  │
│         <node>-ledger-state  │           │  computes lag + hash check   │
└──────────────────────────────┘           └──────────────────────────────┘
```

**RTO**: ~5-10 min (automated failover) / ~15-30 min (manual).
**RPO**: Zero (streaming ledger sync, lag ≤ 10 ledgers = ~50 seconds).

---

## Part 1: Automated State Synchronization

### How It Works

The operator injects a `state-sync` sidecar into every StellarNode pod when
`spec.dr_config.sync_strategy: streamingledger` is set. The sidecar:

1. Polls the local Stellar Core HTTP API (`/info`) every second.
2. Extracts the latest closed ledger sequence and SHA-256 hash.
3. Publishes a `LedgerStateSnapshot` to a Kubernetes ConfigMap
   (`<node-name>-ledger-state`) in the same namespace.
4. Standby-region operators read that ConfigMap via the cross-cluster bridge
   and compute sync lag + hash-chain consistency.

### Enabling Cross-Region Sync

**Step 1: Enable the feature flags in Helm values**

```yaml
# values-production.yaml
featureFlags:
  enableDr: "true"

crossRegion:
  enabled: true
  peerClusters:
    - clusterId: "us-east-1"
      endpoint: "k8s-api.us-east-1.example.com"
      region: "us-east-1"
      port: 11625
      enabled: true
    - clusterId: "eu-west-1"
      endpoint: "k8s-api.eu-west-1.example.com"
      region: "eu-west-1"
      port: 11625
      enabled: true
```

```bash
helm upgrade stellar-operator charts/stellar-operator \
  -f values-production.yaml \
  --namespace stellar-system
```

**Step 2: Configure StellarNode resources**

Primary region:
```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-primary
  namespace: stellar-system
spec:
  nodeType: Validator
  network: Mainnet
  version: "v21.0.0"
  drConfig:
    enabled: true
    role: primary
    peerClusterId: "stellar-system/validator-standby"
    syncStrategy: streamingledger
    healthCheckInterval: 30
    failoverDns:
      hostname: "horizon.stellar.example.com"
      provider: "route53"
```

Standby region:
```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-standby
  namespace: stellar-system
spec:
  nodeType: Validator
  network: Mainnet
  version: "v21.0.0"
  drConfig:
    enabled: true
    role: standby
    peerClusterId: "stellar-system/validator-primary"
    syncStrategy: streamingledger
    healthCheckInterval: 30
    failoverDns:
      hostname: "horizon.stellar.example.com"
      provider: "route53"
```

**Step 3: Verify sync is active**

```bash
# Check the ledger-state ConfigMap on the primary
kubectl get configmap validator-primary-ledger-state \
  -n stellar-system -o jsonpath='{.data.ledger-state}' | jq .

# Expected output:
# {
#   "ledgerSequence": 50123456,
#   "ledgerHash": "a1b2c3d4...",
#   "networkPassphrase": "Public Global Stellar Network ; September 2015",
#   "capturedAt": "2026-05-31T12:00:00Z",
#   "coreVersion": "v21.0.0",
#   "inSync": true
# }

# Check sync lag on the standby
kubectl get stellarnode validator-standby \
  -n stellar-system -o jsonpath='{.status.drStatus}' | jq .
```

### Sync Lag Thresholds

| Lag (ledgers) | Status | Action |
|---------------|--------|--------|
| 0–10 | ✅ In sync | Normal operation |
| 11–50 | ⚠️ Degraded | Alert; investigate network |
| 51–500 | 🔴 Critical | Consider manual intervention |
| > 500 | 🚨 Out of sync | Trigger failover or re-bootstrap |

### Hash-Chain Consistency

The sidecar verifies that when the standby reaches the same ledger sequence as
the primary, their ledger hashes match. A mismatch indicates a **fork** — the
two nodes have diverged and the standby must be re-bootstrapped from a snapshot.

```bash
# Check for fork detection
kubectl get stellarnode validator-standby \
  -n stellar-system -o jsonpath='{.status.drStatus.forkDetected}'
# Should return: false
```

---

## Part 2: Cross-Cluster Network Bridges

The Helm chart creates `ExternalName` Services for each peer cluster, enabling
DNS-based routing without a full service mesh:

```bash
# List bridge services
kubectl get services -n stellar-system -l stellar.org/component=cross-region-bridge

# NAME                          TYPE           CLUSTER-IP   EXTERNAL-IP
# stellar-bridge-us-east-1      ExternalName   <none>       k8s-api.us-east-1.example.com
# stellar-bridge-eu-west-1      ExternalName   <none>       k8s-api.eu-west-1.example.com
```

For production deployments with strict network isolation, use **Submariner** or
**Cilium Cluster Mesh** instead of ExternalName services:

```yaml
# StellarNode with Submariner cross-cluster
spec:
  crossCluster:
    enabled: true
    mode: ServiceMesh
    serviceMesh:
      meshType: Submariner
      clusterSetId: "stellar-global"
    peerClusters:
      - clusterId: "eu-west-1"
        endpoint: "validator-primary.stellar-system.svc.clusterset.local"
        enabled: true
```

---

## Part 3: Failover Procedure

### Prerequisites

1. **Verify standby sync status**:
   ```bash
   kubectl get stellarnode validator-standby \
     -n stellar-system --context=standby-kubeconfig \
     -o jsonpath='{.status.drStatus}' | jq .
   # Confirm: lagLedgers < 10, withinThreshold: true, forkDetected: false
   ```

2. **Confirm quorum capability**:
   ```bash
   kubectl get stellarnode --all-namespaces --context=standby-kubeconfig
   # All Ready=True
   ```

3. **Backups available**:
   ```bash
   velero snapshot get --selector=backup=stellar-daily
   ```

4. **DNS TTL ≤ 300s** on the Horizon endpoint.

### Automated Failover (~5 min)

The operator automatically promotes the standby when the primary is unreachable
for more than `healthCheckInterval` seconds (default: 30s):

```bash
# Monitor failover progress
kubectl get stellarnode validator-standby \
  -n stellar-system --context=standby-kubeconfig \
  -w -o jsonpath='{.status.drStatus.failoverActive}'

# Once true, verify the standby is now primary:
kubectl get stellarnode validator-standby \
  -n stellar-system --context=standby-kubeconfig \
  -o jsonpath='{.status.drStatus.currentRole}'
# Expected: "primary"
```

### Manual Failover (~15 min)

Use this procedure when automated failover has not triggered or you need
controlled maintenance failover.

#### Step 1: Quiesce Primary (~2 min)

```bash
# Graceful scale-down of primary validators
kubectl patch stellarnode validator-primary \
  -n stellar-system \
  --context=primary-kubeconfig \
  -p '{"spec":{"replicas":0}}'

# Wait for pods to terminate
kubectl wait pod \
  -l app.kubernetes.io/instance=validator-primary \
  -n stellar-system \
  --context=primary-kubeconfig \
  --for=delete --timeout=300s
```

#### Step 2: Verify Final Sync State (~1 min)

```bash
# Confirm standby has the latest ledger
kubectl get configmap validator-primary-ledger-state \
  -n stellar-system --context=primary-kubeconfig \
  -o jsonpath='{.data.ledger-state}' | jq .ledgerSequence

kubectl get stellarnode validator-standby \
  -n stellar-system --context=standby-kubeconfig \
  -o jsonpath='{.status.ledgerSequence}'

# Both values should match (or standby within 10 ledgers)
```

#### Step 3: Promote Standby (~2 min)

```bash
# Update the standby's DR role to primary
kubectl patch stellarnode validator-standby \
  -n stellar-system \
  --context=standby-kubeconfig \
  -p '{"spec":{"drConfig":{"role":"primary"}}}'

# Scale up to full replica count
kubectl patch stellarnode validator-standby \
  -n stellar-system \
  --context=standby-kubeconfig \
  -p '{"spec":{"replicas":5}}'

# Force peer-discovery refresh
kubectl annotate stellarnode validator-standby \
  -n stellar-system \
  --context=standby-kubeconfig \
  peer-discovery.stellar.org/reload=true --overwrite
```

#### Step 4: Failover Horizon DNS (~2 min)

```bash
# Update Route53 (replace with your DNS provider)
aws route53 change-resource-record-sets \
  --hosted-zone-id Z123 \
  --change-batch '{
    "Changes": [{
      "Action": "UPSERT",
      "ResourceRecordSet": {
        "Name": "horizon.stellar.example.com",
        "Type": "A",
        "TTL": 60,
        "ResourceRecords": [{"Value": "STANDBY_REGION_IP"}]
      }
    }]
  }'
```

#### Step 5: Validate (~5 min)

```bash
# 1. Check quorum
kubectl logs -l app=stellar-core \
  -n stellar-system --context=standby-kubeconfig \
  | grep "Quorum set"

# 2. Check Horizon health
curl -f https://horizon.stellar.example.com/health

# 3. Verify ledger advancement (should increase every ~5s)
watch -n 5 'kubectl get stellarnode validator-standby \
  -n stellar-system --context=standby-kubeconfig \
  -o jsonpath="{.status.ledgerSequence}"'

# 4. Submit a test transaction
# stellar-sdk submit-tx --network mainnet --source-account GA...
```

**Success Criteria**:
- ✅ Standby quorum active (5+ validators in quorum slice)
- ✅ Horizon `/health` returns 200
- ✅ Ledger sequence advancing
- ✅ New transactions processing

---

## Part 4: Rollback Procedure

If validation fails within 15 minutes of failover:

```bash
# 1. Revert DNS
aws route53 change-resource-record-sets \
  --hosted-zone-id Z123 \
  --change-batch file://horizon-rollback.json

# 2. Restore primary
kubectl patch stellarnode validator-primary \
  -n stellar-system \
  --context=primary-kubeconfig \
  -p '{"spec":{"replicas":5}}'

# 3. Demote standby back
kubectl patch stellarnode validator-standby \
  -n stellar-system \
  --context=standby-kubeconfig \
  -p '{"spec":{"drConfig":{"role":"standby"}}}'

# 4. Wait for primary to re-sync
kubectl wait stellarnode validator-primary \
  -n stellar-system \
  --context=primary-kubeconfig \
  --for=condition=Ready --timeout=600s
```

---

## Part 5: Consistency Verification Under Load

The `state_sync` module includes tests that prove consistency under high load.
Run them with:

```bash
cargo test state_sync -- --nocapture
```

Key test scenarios:
- **Rapid ledger advancement**: 1000 ledger advances with up to 3-ledger jitter
  — all within the 10-ledger threshold.
- **Sustained lag detection**: 20,000-ledger lag correctly flagged as out-of-sync.
- **Fork detection**: Hash mismatch at the same ledger sequence triggers alert.
- **Serialization roundtrip**: `LedgerStateSnapshot` survives JSON encode/decode.

---

## Troubleshooting

| Symptom | Diagnosis | Fix |
|---------|-----------|-----|
| `lagLedgers` > 10 | Network congestion or bridge down | Check ExternalName service DNS; verify peer cluster reachability |
| `forkDetected: true` | Hash-chain divergence | Re-bootstrap standby from OCI snapshot: `spec.oci_snapshot` |
| ConfigMap not found | Sidecar not injected | Verify `sync_strategy: streamingledger` and `enableDr: "true"` |
| Sidecar CrashLoopBackOff | Core not reachable | Check `STELLAR_CORE_HTTP_URL` env var; verify Core pod is running |
| Automated failover not triggering | `healthCheckInterval` too high | Lower to 15s; check `peer_health` in DR status |
| DNS not updating | Route53 permissions | Verify IAM role has `route53:ChangeResourceRecordSets` |

---

## References

- [Cross-Region Architecture](../src/controller/state_sync.rs) — Rust implementation
- [Cross-Cluster Bridges](../src/controller/cross_cluster.rs) — ExternalName + Submariner
- [DR Controller](../src/controller/dr.rs) — Automated failover logic
- [Peer Discovery](peer-discovery.md)
- [Volume Snapshots](volume-snapshots.md)
- [OCI Snapshot Sync](../src/controller/oci_snapshot.rs) — Cross-region bootstrapping
- Stellar Core docs: https://developers.stellar.org/docs/validators/admin-guide/quorum

**Last Updated**: 2026-05-31
