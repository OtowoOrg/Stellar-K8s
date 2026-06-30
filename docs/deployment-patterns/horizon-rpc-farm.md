# Horizon RPC Farm Architecture

> Scalable API architecture for Horizon/RPC workloads with independent lifecycle from validator consensus paths.

---

## 1) Reference Topology

| Layer | Component | Notes |
|---|---|---|
| Ingress | NLB/ALB/MetalLB | Multi-AZ endpoint for user traffic |
| API tier | Horizon deployments | Horizontal autoscaling, stateless pods |
| Data tier | PostgreSQL replicas | Read-heavy scaling and failover strategy |
| Cache tier | Redis (optional) | Query acceleration and hot-path offload |

## 2) Horizontal Scaling Baseline

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: horizon-hpa
  namespace: stellar-rpc
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: horizon
  minReplicas: 3
  maxReplicas: 30
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 65
    - type: Pods
      pods:
        metric:
          name: http_requests_per_second
        target:
          type: AverageValue
          averageValue: "250"
```

## 3) Traffic Segmentation

- Keep validator and RPC namespaces physically and logically separate.
- Use distinct node pools and taints to avoid noisy-neighbor effects.
- Apply independent disruption budgets and autoscaling policies.

## 4) SLO-Oriented Capacity Hints

| Daily Active Clients | Recommended Horizon Replicas | DB Read Replicas |
|---|---|---|
| < 10k | 3-5 | 1 |
| 10k-100k | 6-12 | 2-3 |
| 100k-1M | 15-30 | 4-8 |

## 5) Validation Checks

```bash
kubectl -n stellar-rpc get deploy,hpa,pdb
kubectl -n stellar-rpc top pods
kubectl -n stellar-rpc logs deploy/horizon --tail=100
```

Target outcomes:

- p95 request latency remains within API SLO during step-load tests.
- No error-rate spikes during rolling updates or DB replica failover.
