# Multi-Cluster and Edge Routing

> This guide defines deterministic BGP edge connectivity for Stellar-K8s clusters, including ToR peering, MetalLB service announcement modes, and AWS managed load balancer equivalents.

---

## 1) ToR Mapping Strategy

### Rack and ASN Layout

| Rack | Node Label | Node ASN | ToR Peer ASN | Peer IP |
|---|---|---|---|---|
| Rack A | `rack=a` | `64520` | `64512` | `172.20.0.1` |
| Rack B | `rack=b` | `64521` | `64512` | `172.21.0.1` |
| Rack C | `rack=c` | `64522` | `64512` | `172.22.0.1` |

### Node Labeling

```bash
kubectl label nodes worker-a01 rack=a stellar.io/edge=true
kubectl label nodes worker-b01 rack=b stellar.io/edge=true
kubectl label nodes worker-c01 rack=c stellar.io/edge=true
```

### BGPAdvertisement (MetalLB BGP mode)

```yaml
apiVersion: metallb.io/v1beta1
kind: IPAddressPool
metadata:
  name: stellar-rpc-pool
  namespace: metallb-system
spec:
  addresses:
    - 10.30.10.100-10.30.10.150
---
apiVersion: metallb.io/v1beta1
kind: BGPPeer
metadata:
  name: tor-a
  namespace: metallb-system
spec:
  myASN: 64520
  peerASN: 64512
  peerAddress: 172.20.0.1
  holdTime: 9s
  bfdProfile: fast-failover
---
apiVersion: metallb.io/v1beta1
kind: BGPAdvertisement
metadata:
  name: stellar-rpc-adv
  namespace: metallb-system
spec:
  ipAddressPools:
    - stellar-rpc-pool
  communities:
    - 64512:100
  localPref: 150
```

---

## 2) MetalLB Mode Selection

| Mode | Use Case | Benefits | Risks |
|---|---|---|---|
| Layer2 | Small single-L2 domains, lab/dev | Simple setup, no BGP dependencies | ARP/NDP failover convergence less deterministic |
| BGP | Production, multi-rack, route-policy control | Fast failover, ToR policy integration, ECMP support | Requires network team ASN/policy coordination |

### Layer2 Example

```yaml
apiVersion: metallb.io/v1beta1
kind: L2Advertisement
metadata:
  name: l2-adv
  namespace: metallb-system
spec:
  ipAddressPools:
    - stellar-rpc-pool
```

---

## 3) Cloud Equivalent: AWS Load Balancer Controller

### NLB Service for Horizon/RPC

```yaml
apiVersion: v1
kind: Service
metadata:
  name: horizon-public
  namespace: stellar-rpc
  annotations:
    service.beta.kubernetes.io/aws-load-balancer-type: "nlb"
    service.beta.kubernetes.io/aws-load-balancer-scheme: "internet-facing"
    service.beta.kubernetes.io/aws-load-balancer-target-type: "ip"
    service.beta.kubernetes.io/aws-load-balancer-healthcheck-protocol: "HTTP"
    service.beta.kubernetes.io/aws-load-balancer-healthcheck-path: "/metrics"
    service.beta.kubernetes.io/aws-load-balancer-cross-zone-load-balancing-enabled: "true"
spec:
  type: LoadBalancer
  ports:
    - name: http
      port: 80
      targetPort: 8000
  selector:
    app.kubernetes.io/name: horizon
```

### ALB Ingress for API routing

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: rpc-ingress
  namespace: stellar-rpc
  annotations:
    kubernetes.io/ingress.class: alb
    alb.ingress.kubernetes.io/scheme: internet-facing
    alb.ingress.kubernetes.io/target-type: ip
    alb.ingress.kubernetes.io/listen-ports: '[{"HTTP":80},{"HTTPS":443}]'
    alb.ingress.kubernetes.io/ssl-redirect: "443"
spec:
  rules:
    - host: rpc.example.org
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: horizon-public
                port:
                  number: 80
```

---

## 4) Multi-Cluster Edge Pattern

### Recommended Pattern

- Regional cluster-local validator pools maintain local quorum peers first.
- Inter-region peers are constrained to designated edge nodes.
- Public RPC traffic is globally distributed via DNS + health checks.

### Edge BGP Policy Suggestions

- Advertise only service VIP ranges from edge nodes.
- Keep Pod CIDR advertisements internal unless routing architecture requires exposure.
- Use BGP communities to control upstream preference and blackhole signaling for incident response.

## 5) Verification Checklist

```bash
kubectl -n metallb-system get bgppeers,bgpadvertisements,ipaddresspools
kubectl get svc -A | grep LoadBalancer
ip route get 10.30.10.101
```

- Verify service VIPs are present in ToR route table.
- Verify failover convergence under node drain (`kubectl drain`).
- Verify RPC SLO during failover remains within error budget.
