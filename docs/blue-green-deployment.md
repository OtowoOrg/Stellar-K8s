# Blue-Green Deployment Strategy for Stellar Core

*Addresses issue #1417.*

Blue-green deployment is the recommended upgrade strategy for **Horizon** and
**Soroban RPC** nodes running in production.  It provides zero-downtime
upgrades with instant, automated rollback when health checks fail.

---

## How It Works

```
Step 1: Blue (v21.0.0) is live, serving all traffic
┌─────────────────────────────────────────┐
│  Service (selector: app=horizon)        │
└──────────────┬──────────────────────────┘
               │ 100% traffic
       ┌───────▼────────┐
       │ Blue Deployment│  (current version)
       │  v21.0.0       │
       └────────────────┘

Step 2: Operator creates Green (v21.1.0), waits for Ready
┌─────────────────────────────────────────┐
│  Service (selector: app=horizon)        │
└──────────────┬──────────────────────────┘
               │ 100% traffic
       ┌───────▼────────┐      ┌───────────────────┐
       │ Blue Deployment│      │ Green Deployment   │
       │  v21.0.0       │      │  v21.1.0 (ready)  │
       └────────────────┘      └───────────────────┘

Step 3: Smoke tests pass → Service selector flipped to Green (atomic)
┌───────────────────────────────────────────────────────────┐
│  Service (selector: deployment-color=green)               │
└────────────────────────────────────────┬──────────────────┘
                                         │ 100% traffic
       ┌────────────────┐      ┌─────────▼─────────┐
       │ Blue Deployment│      │ Green Deployment   │
       │  v21.0.0       │      │  v21.1.0           │
       └────────────────┘      └───────────────────┘
              (pending cleanup)

Step 4: Health monitor passes observation window → Blue deleted
┌───────────────────────────────────────────────────────────┐
│  Service (standard selector)                              │
└────────────────────────────────────────┬──────────────────┘
                                         │ 100% traffic
                               ┌─────────▼─────────┐
                               │ Deployment         │
                               │  v21.1.0           │
                               └───────────────────┘
```

The traffic switch happens **atomically** at the Kubernetes Service level —
there is no period where some pods run v21.0.0 and others run v21.1.0.

---

## Acceptance Criteria

| Criterion | Implementation |
|---|---|
| Blue-green deployment manifests | `examples/blue-green-deployment.yaml` |
| Switching logic | `src/controller/blue_green.rs` |
| Health-check based traffic switching | `switch_traffic_to_green` + `run_smoke_tests` |
| Automated rollback on health check failure | `monitor_and_auto_rollback` + `rollback_to_blue` |

---

## Configuration

Add a `blueGreen` block to your `StellarNode` spec:

```yaml
spec:
  nodeType: Horizon
  network: mainnet
  version: "v21.1.0"             # Bump to trigger upgrade
  deploymentStrategy: BlueGreen
  blueGreen:
    readyTimeoutSeconds: 300      # Wait up to 5 min for green replicas
    switchTimeoutSeconds: 60      # Switch must complete within 1 min
    enableSmokeTests: true        # Run /health checks before switching
    healthCheckEndpoint: /health
    autoRollback:
      enabled: true
      failureThreshold: 3         # Roll back after 3 consecutive failures
      observationWindowSeconds: 120  # Monitor for 2 min post-switch
```

### Field Reference

| Field | Type | Default | Description |
|---|---|---|---|
| `readyTimeoutSeconds` | int | 300 | Max seconds to wait for the green deployment to reach Ready state |
| `switchTimeoutSeconds` | int | 60 | Max seconds for the Service selector patch to complete |
| `enableSmokeTests` | bool | true | Run HTTP health checks against green before switching |
| `healthCheckEndpoint` | string | `/health` | Path to probe for smoke tests |
| `autoTrafficSwitch` | bool | true | Set to `false` for manual traffic management |
| `autoRollback.enabled` | bool | true | Enable automated rollback on health check failure |
| `autoRollback.failureThreshold` | int | 3 | Consecutive failures before rollback |
| `autoRollback.observationWindowSeconds` | int | 120 | Duration of the post-switch health monitor |

---

## Automated Rollback

The `monitor_and_auto_rollback` function runs after a successful traffic switch
and periodically checks:

1. **Kubernetes readiness** — all desired replicas in the green Deployment are
   `Ready`.
2. **HTTP health endpoint** — `GET <health_check_endpoint>` returns 2xx.

If either check fails `failure_threshold` consecutive times within
`observation_window_secs` seconds, the operator:

1. Re-runs `rollback_to_blue` — restores the Service selector to the standard
   (non-color) labels so traffic goes back to the previous deployment.
2. Deletes the failed green deployment.
3. Emits a Kubernetes warning event on the `StellarNode` resource.

The rollback is designed to be **idempotent** — if the operator restarts mid-
rollback it will re-enter the same code path and converge to the correct state.

---

## Manual Traffic Management

Set `autoTrafficSwitch: false` to pause after the green deployment is ready,
giving operators time to inspect it before committing.

```bash
# Check blue-green status
kubectl stellar blue-green status --name horizon-mainnet --namespace stellar

# Manually switch traffic to green
kubectl stellar blue-green switch --name horizon-mainnet --namespace stellar

# Manually rollback to blue
kubectl stellar blue-green rollback --name horizon-mainnet --namespace stellar
```

---

## Zero-Downtime Guarantee

The Service selector switch is a single atomic API call to the Kubernetes API
server.  Kubernetes routes new connections to the new pods immediately.
Existing long-lived connections (HTTP/2, WebSocket) drain naturally as the old
pods are removed after the blue cleanup.

If the API server is unreachable, the switch is retried with back-off.  Traffic
is never split between versions.

---

## Observability

The operator emits the following metrics for blue-green deployments:

| Metric | Type | Labels |
|---|---|---|
| `stellar_blue_green_switch_total` | Counter | `namespace`, `name`, `network`, `result` |
| `stellar_blue_green_rollback_total` | Counter | `namespace`, `name`, `network`, `reason` |
| `stellar_horizon_migration_duration_seconds` | Histogram | `namespace`, `name`, `network`, `result` |

Example Prometheus alert for unexpected rollbacks:

```yaml
- alert: BlueGreenRollbackDetected
  expr: increase(stellar_blue_green_rollback_total[10m]) > 0
  for: 1m
  labels:
    severity: warning
  annotations:
    summary: "Blue-green rollback triggered for {{ $labels.name }}"
    description: "An automated rollback was triggered — inspect the pod logs and events."
```

---

## Examples

Ready-to-apply manifests: [`examples/blue-green-deployment.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/blue-green-deployment.yaml)

- **Horizon mainnet** — automatic traffic switch + 2-minute rollback window
- **Soroban RPC mainnet** — longer ready timeout for sync
- **Manual mode** — `autoTrafficSwitch: false` for operator-controlled switch
