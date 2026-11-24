# Stellar Core (Validator) Blue/Green Deployment Runbook

This runbook describes **Validator / Stellar Core** blue-green rollouts for issue #1331.

It is **not** the Horizon/Soroban RPC blue-green migration path (`src/controller/blue_green.rs`).

## What blue-green means for Stellar Core

| Color | Meaning |
|-------|---------|
| **Blue** | Publishing StatefulSet `{name}` with PVC `{name}-data` |
| **Green** | Warm standby StatefulSet `{name}-green` with **independent** PVC `{name}-green-data` |

Green warms with `NODE_IS_VALIDATOR=false` so it does **not** publish while blue is active.

Cutover is **serialized** to avoid dual Core identity:

1. Green catches up (standby)
2. Blue is scaled to 0 and pods are confirmed down
3. Green ConfigMap sets `NODE_IS_VALIDATOR=true` and the green StatefulSet is force-rolled
4. Operator waits for green Ready + Synced + ledger lag gate
5. **Only then** the primary Service selector switches to green

## What "zero downtime" and "instant rollback" mean here

- **Not** dual-active publishing of the same validator identity (unsupported / unsafe).
- Warm green catch-up reduces the cutover gap; there is still a window while blue is down before green is publishing and selected.
- **Instant rollback** means the **routing decision** can move the Service selector quickly once the target color is Ready + Synced. It does **not** mean a cold Core process is instantly operational. Rollback always waits for blue Ready + Synced before switching the Service.

## Enable

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-1
spec:
  nodeType: Validator
  version: "21.2.0"
  strategy:
    type: blueGreen
    blueGreen:
      maxLedgerLag: 5
      readyTimeoutSeconds: 3600
      rollbackWindowSeconds: 3600
      requireVolumeSnapshot: true
  storage:
    storageClass: standard-rwo
    size: 500Gi
  validatorConfig:
    seedSecretRef: validator-seed
```

Canary remains rejected for Validators.

## Storage isolation

- Blue PVC: `{name}-data`
- Green PVC: `{name}-green-data`
- Never share a live Core data PVC between colors
- Prefer CSI VolumeSnapshot of blue -> green PVC `dataSource` when `requireVolumeSnapshot: true`
- **Never delete** rollback-protected PVCs automatically (blue retained at replicas=0 after cutover)

## Failed rollouts

If green does not become eligible before `readyTimeoutSeconds`, phase becomes `Failed` and blue stays active.

Destructive preparation (new snapshots / green rebuild) does **not** repeat every reconcile.

Retry explicitly:

```bash
kubectl annotate stellarnode validator-1 stellar.org/bg-retry=true --overwrite
```

## Repeated upgrades after green is active

Further blue/green version bumps while green is already active are **deferred** (`UpgradeDeferred`). There is no automatic flip-flop that deletes the blue PVC. Consolidate manually before another automated blue/green cycle.

## Observability

| Field / annotation | Purpose |
|--------------------|---------|
| `status.blueGreenPhase` | `BlueActive`, `PreparingGreen`, `WaitingForGreen`, `CuttingOver`, `GreenActive`, `RollingBack`, `Failed`, `UpgradeDeferred` |
| `status.blueGreenActiveColor` | `blue` or `green` |
| `stellar.org/bg-cutover-step` | Serialized cutover sub-step |
| `stellar.org/bg-rollback-step` | Serialized rollback sub-step |
| `stellar.org/bg-publish-rollout` | Token used to force green STS restart after publish config change |

## Prerequisites

- CSI VolumeSnapshot support when `requireVolumeSnapshot: true`
- History archives reachable for residual catch-up

## Relation to Horizon blue-green

| | Validator (#1331) | Horizon |
|--|-------------------|---------|
| Module | `blue_green_core.rs` | `blue_green.rs` |
| Workload | StatefulSet + isolated PVCs | Deployment |
| Health | Core sync + ledger lag | Deployment ready + HTTP `/health` |
| Switch | Service after publish+health | Service `deployment-color` |
