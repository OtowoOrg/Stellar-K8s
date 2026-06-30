# Multi-Region and Disaster Recovery Patterns

> Runbook and architecture guidance for multi-region Stellar-K8s with deterministic quorum behavior and tested restoration workflows.

---

## 1) Multi-Region Pattern

| Region | Role | Workloads |
|---|---|---|
| Region A | Primary | Active validator quorum majority + RPC |
| Region B | Secondary | Standby validators + active RPC |
| Region C | Tertiary | Witness validators/observer + cold failover |

### Quorum Latency Optimization

- Prioritize quorum slices with low intra-slice RTT.
- Cap cross-region validator peer count to avoid quorum instability under WAN jitter.
- Continuously track p95/p99 RTT and packet loss across regions.

---

## 2) Velero Backup/Restore Procedure

### Backup Schedule

```yaml
apiVersion: velero.io/v1
kind: Schedule
metadata:
  name: stellar-nightly
  namespace: velero
spec:
  schedule: "0 2 * * *"
  template:
    includedNamespaces:
      - stellar-validator
      - stellar-rpc
    ttl: 720h
    snapshotVolumes: true
```

### Restore Command

```bash
velero restore create --from-backup stellar-nightly-2026-06-27
velero restore describe stellar-nightly-2026-06-27 --details
```

---

## 3) etcd Snapshot Automation

```bash
#!/usr/bin/env bash
set -euo pipefail
TS="$(date +%Y%m%d-%H%M%S)"
ETCDCTL_API=3 etcdctl \
  --endpoints=https://127.0.0.1:2379 \
  --cacert=/etc/kubernetes/pki/etcd/ca.crt \
  --cert=/etc/kubernetes/pki/etcd/server.crt \
  --key=/etc/kubernetes/pki/etcd/server.key \
  snapshot save /var/backups/etcd/etcd-${TS}.db
ETCDCTL_API=3 etcdctl snapshot status /var/backups/etcd/etcd-${TS}.db -w table
```

## 4) DR Objectives

| Objective | Target |
|---|---|
| RPO (validator config + secrets metadata) | <= 15 minutes |
| RTO (regional control-plane recovery) | <= 60 minutes |
| RPC partial service restoration | <= 20 minutes |

## 5) Recovery Validation

1. Restore manifests and secrets metadata from Velero backup.
2. Restore persistent volume snapshots where required.
3. Verify validator lock and quorum declarations before rejoining peers.
4. Run synthetic transaction and ledger catch-up checks.
