# Canary Deployments for Horizon

The operator supports canary releases for Horizon (and SorobanRpc) nodes, routing a configurable percentage of live traffic to a new version before promoting it to stable. If the canary's 4xx/5xx error rate spikes, the operator automatically rolls back.

## How It Works

```
                    ┌─────────────────────────────────────┐
                    │   Ingress / Istio VirtualService     │
                    │   horizon.stellar.example.com        │
                    └──────────┬──────────────┬────────────┘
                               │ 90%          │ 10%
                    ┌──────────▼──┐      ┌────▼──────────┐
                    │  Stable     │      │  Canary        │
                    │  Deployment │      │  Deployment    │
                    │  v2.30.0    │      │  v2.31.0       │
                    └─────────────┘      └───────────────┘
```

1. Operator detects a version change in `spec.version`
2. Creates a `<name>-canary` Deployment and Service running the new version
3. Applies Nginx canary annotations (or Istio VirtualService) to split traffic
4. Every `checkIntervalSeconds`, probes the canary's `/health` endpoint and measures the 4xx/5xx error rate
5. If healthy: optionally steps up the weight and increments the consecutive-healthy counter
6. Once `successThreshold` consecutive healthy checks pass at `maxWeight`: promotes (replaces stable deployment)
7. If error rate exceeds `maxErrorRate` at any point: rolls back immediately

## Configuration

Add a `strategy` block to your Horizon `StellarNode`:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: horizon-mainnet
  namespace: stellar
spec:
  nodeType: Horizon
  network: Mainnet
  version: "v2.31.0"          # ← bump this to trigger a canary

  strategy:
    type: Canary
    canary:
      weight: 10               # initial % of traffic to canary
      checkIntervalSeconds: 300 # evaluate every 5 minutes
      maxErrorRate: 0.05       # rollback if >5% of requests are 4xx/5xx
      stepWeight: 10           # increase weight by 10% each healthy check
      maxWeight: 50            # promote once weight reaches 50% and successThreshold met
      successThreshold: 2      # require 2 consecutive healthy checks before promoting

  ingress:
    className: nginx           # or "istio"
    hosts:
      - host: horizon.stellar.example.com
        paths:
          - path: /
```

### Field Reference

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `weight` | int | `10` | Initial traffic percentage sent to the canary |
| `checkIntervalSeconds` | int | `300` | Seconds between health evaluations |
| `maxErrorRate` | float | `0.05` | Max 4xx/5xx rate (0.0–1.0) before rollback |
| `stepWeight` | int | `0` | Weight increment per healthy check (0 = no stepping) |
| `maxWeight` | int | `50` | Maximum weight before promotion is considered |
| `successThreshold` | int | `1` | Consecutive healthy checks required to promote |

## Traffic Splitting

### Nginx Ingress

When `ingress.className` is `nginx` (or any non-Istio class), the operator applies standard Nginx canary annotations:

```yaml
nginx.ingress.kubernetes.io/canary: "true"
nginx.ingress.kubernetes.io/canary-weight: "10"
```

A second Ingress resource (`<name>-ingress-canary`) is created pointing to the `<name>-canary` Service.

### Istio

When `ingress.className` is `istio`, the operator creates an Istio `VirtualService` that splits traffic using weighted routes:

```yaml
apiVersion: networking.istio.io/v1beta1
kind: VirtualService
spec:
  hosts:
    - horizon.stellar.example.com
  http:
    - route:
        - destination:
            host: horizon-mainnet        # stable
            port: { number: 8000 }
          weight: 90
        - destination:
            host: horizon-mainnet-canary # canary
            port: { number: 8000 }
          weight: 10
```

The VirtualService is updated on every reconcile to reflect the current live weight from `status.canaryWeight`.

## Progressive Rollout Example

With `stepWeight: 10`, `maxWeight: 50`, `successThreshold: 2`:

```
Interval 1: weight=10%, healthy → weight steps to 20%, consecutiveHealthy=1
Interval 2: weight=20%, healthy → weight steps to 30%, consecutiveHealthy=2
Interval 3: weight=30%, healthy → weight steps to 40%, consecutiveHealthy=3
Interval 4: weight=40%, healthy → weight steps to 50%, consecutiveHealthy=4
Interval 5: weight=50%, consecutiveHealthy≥2 AND weight≥maxWeight → PROMOTE
```

## Automatic Rollback

The operator rolls back if:
- The canary pod is not ready (no Ready pods found)
- The measured 4xx/5xx error rate exceeds `maxErrorRate`

On rollback:
- The `<name>-canary` Deployment, Service, and Ingress/VirtualService are deleted
- The stable Deployment is left unchanged (no traffic disruption)
- A `CanaryRolledBack` Kubernetes Event is emitted with the reason
- `status.phase` is set to `Failed` with the rollback message

```bash
# Watch for canary events
kubectl get events -n stellar --field-selector reason=CanaryRolledBack
kubectl get events -n stellar --field-selector reason=CanaryPromoted
```

## Observing Canary Status

```bash
kubectl get stellarnode horizon-mainnet -n stellar -o jsonpath='{.status}' | jq '{
  canaryVersion: .canaryVersion,
  canaryWeight: .canaryWeight,
  canaryErrorRate: .canaryErrorRate,
  canaryConsecutiveHealthy: .canaryConsecutiveHealthy,
  phase: .phase
}'
```

Example output during a progressive rollout:
```json
{
  "canaryVersion": "v2.31.0",
  "canaryWeight": 30,
  "canaryErrorRate": 0.0,
  "canaryConsecutiveHealthy": 2,
  "phase": "Canary"
}
```

## Manual Rollback

To force an immediate rollback, remove the canary version from the spec:

```bash
kubectl patch stellarnode horizon-mainnet -n stellar \
  --type=merge \
  -p '{"spec":{"version":"v2.30.0"}}'
```

Or patch the status directly to clear canary state (operator will clean up resources on next reconcile):

```bash
kubectl patch stellarnode horizon-mainnet -n stellar \
  --type=merge \
  --subresource=status \
  -p '{"status":{"canaryVersion":null,"canaryStartTime":null}}'
```

## Limitations

- Canary is only supported for `Horizon` and `SorobanRpc` node types (not `Validator`)
- Requires an Ingress controller (Nginx, Traefik, or Istio) to be installed
- The error rate measurement uses a 5-sample probe of the canary pod's `/health` endpoint — for production use, consider integrating with Prometheus metrics for more accurate measurement
- `stepWeight: 0` (default) keeps the weight fixed at the initial `weight` value throughout the rollout
