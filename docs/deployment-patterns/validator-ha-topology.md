# High-Availability Validator Topology

> This architecture enforces strict single-active signing behavior while preserving survivability across node/zone failures.

---

## 1) Anti-Double-Signing Model

| Control | Implementation | Why it matters |
|---|---|---|
| Single-active lock | Lease object + operator leader lock | Prevents concurrent signing processes |
| Stateful identity | StatefulSet ordinal + persistent identity | Stable peer configuration and key mapping |
| Hard anti-affinity | One validator pod per node | Reduces correlated host failures |
| Readiness gates | Active lock required for Ready | Prevents accidental traffic routing to standby signer |

### Example Pod Anti-Affinity + Topology Spread

```yaml
affinity:
  podAntiAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      - labelSelector:
          matchLabels:
            app.kubernetes.io/component: validator
        topologyKey: kubernetes.io/hostname
topologySpreadConstraints:
  - maxSkew: 1
    topologyKey: topology.kubernetes.io/zone
    whenUnsatisfiable: DoNotSchedule
    labelSelector:
      matchLabels:
        app.kubernetes.io/component: validator
```

### Lease Guard Example (conceptual)

```yaml
apiVersion: coordination.k8s.io/v1
kind: Lease
metadata:
  name: validator-signer-lock
  namespace: stellar-validator
spec:
  holderIdentity: validator-0
  leaseDurationSeconds: 15
```

---

## 2) StatefulSet Profile

| Parameter | Recommended |
|---|---|
| `podManagementPolicy` | `OrderedReady` |
| `updateStrategy` | `RollingUpdate` with partition gates |
| `terminationGracePeriodSeconds` | `>= 120` |
| PVC | Dedicated high-IOPS class per validator |

### Single-Active Readiness Gate Pattern

- Active signer writes lock ownership state to shared control endpoint.
- Readiness probe checks lock ownership before marking ready.
- Standby validators remain unready until lock transfer is complete.

## 3) Quorum Slice Alignment

- Ensure quorum sets do not overconcentrate in a single zone/region.
- Keep inter-validator RTT low and consistent within primary slices.
- Run periodic byzantine and partition simulations before production changes.

## 4) Failure Drills

1. Simulate node crash of active signer; validate standby promotion time.
2. Simulate zone outage; verify no dual-active condition.
3. Simulate stale lease and network partition; verify conservative fail-safe behavior.
