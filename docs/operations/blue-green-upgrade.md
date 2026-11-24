# Blue-Green Deployment for Stellar Core Validators

**Issue:** #1387 — Implement blue-green deployment strategy for Stellar Core  
**Audience:** SREs, cluster operators

---

## Overview

Stellar-K8s implements blue-green deployments for Validator (Stellar Core)
StatefulSets to achieve **zero-downtime upgrades** with **instant rollback**.

The strategy runs two StatefulSets ("blue" and "green") on independent
PersistentVolumeClaims. Traffic flows through a single `stellar-validator`
Service whose `selector` is atomically switched between colors by the operator.

---

## Deployment Phases

```
BlueActive
    │  (version bump annotation or spec.version change)
    ▼
PreparingGreen  ── green StatefulSet scaled to 1, NODE_IS_VALIDATOR=false
    │
    ▼
WaitingForGreen ── polling until: Ready + Synced + lag ≤ maxLedgerLag
    │
    ▼
CuttingOver     ── blue scaled to 0, Service selector → green, green validator=true
    │
    ▼
GreenActive     ── post-cutover health checks; blue retained for rollback window
    │
    ▼ (rollback window expires or operator confirms)
[Blue PVC deleted, green is permanent]
```

### Automatic rollback path

```
CuttingOver / GreenActive
    │  (green fails consecutiveFailureThreshold checks)
    ▼
RollingBack     ── blue scaled to 1, Service selector → blue
    │
    ▼
BlueActive      ── original state restored; green scaled to 0
```

---

## Health-Gate Thresholds

| Parameter | Default | Description |
|-----------|---------|-------------|
| `maxLedgerLag` | 5 | Max ledger lag before cutover proceeds |
| `readyTimeoutSeconds` | 3600 | Max seconds waiting for green to sync |
| `postCutoverSuccessThreshold` | 3 | Consecutive healthy checks to finalise |
| `consecutiveFailureThreshold` | 3 | Failures post-cutover before rollback |
| `rollbackWindowSeconds` | 3600 | Seconds to retain blue after cutover |

---

## Triggering an Upgrade

### Via StellarNode spec (recommended)

```yaml
# stellar-node.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: my-validator
  namespace: stellar-system
spec:
  nodeType: Validator
  version: "v22.0.0"    # ← bump version
  strategy:
    type: blueGreen
    blueGreen:
      maxLedgerLag: 5
      readyTimeoutSeconds: 3600
      requireVolumeSnapshot: true
```

```bash
kubectl apply -f stellar-node.yaml
```

### Via annotation (emergency / CI)

```bash
kubectl annotate stellarnode my-validator \
  stellar.org/bg-target-version=v22.0.0 \
  stellar.org/bg-retry=true \
  -n stellar-system
```

---

## Monitoring the Rollout

### Check current phase

```bash
kubectl get stellarnode my-validator \
  -n stellar-system \
  -o jsonpath='{.status.blueGreenPhase}'
```

### Watch live events

```bash
kubectl get events \
  --field-selector involvedObject.name=my-validator \
  -n stellar-system \
  --watch
```

### Check active color and target version

```bash
kubectl get stellarnode my-validator -n stellar-system \
  -o custom-columns=\
PHASE:.status.blueGreenPhase,\
COLOR:.status.blueGreenActiveColor,\
TARGET:.status.blueGreenTargetVersion,\
MESSAGE:.status.blueGreenMessage
```

### Verify Service selector

```bash
kubectl get service stellar-validator -n stellar-system \
  -o jsonpath='{.spec.selector}'
```

---

## Manual Rollback

If automatic rollback does not fire, force a manual rollback:

```bash
# Scale green to 0
kubectl scale statefulset stellar-validator-green \
  --replicas=0 -n stellar-system

# Switch Service back to blue
kubectl patch service stellar-validator \
  -n stellar-system \
  --type=merge \
  -p '{"spec":{"selector":{"stellar.org/deployment-color":"blue"}}}'

# Scale blue back to 1
kubectl scale statefulset stellar-validator-blue \
  --replicas=1 -n stellar-system

# Clear the failed annotation to allow retry later
kubectl annotate stellarnode my-validator \
  stellar.org/bg-retry=true \
  -n stellar-system
```

---

## Verifying Zero Downtime

The upgrade should produce **no validator downtime** (no missed ledger closes).
Verify with:

```bash
# Watch ledger sequence advancing continuously
watch -n 2 "kubectl get stellarnode my-validator \
  -n stellar-system \
  -o jsonpath='{.status.ledgerSequence}'"

# Check no alerts fired during the upgrade
kubectl get prometheusrule stellar-blue-green-alerts -n stellar-system
```

---

## Manifests Reference

| File | Purpose |
|------|---------|
| `config/blue-green/blue-green-deployment.yaml` | Blue/green StatefulSets, Services, PDB |
| `config/blue-green/health-check-traffic-switch.yaml` | Health gate ConfigMap + Prometheus alerts |

---

## Limitations

- Blue-green is supported for `nodeType: Validator` only. Horizon/SorobanRpc
  use the Deployment-based canary strategy.
- The active-slot PVC is **never deleted automatically** — manual pruning is
  required after the rollback window expires.
- Requires CSI snapshot support when `requireVolumeSnapshot: true` (default).
