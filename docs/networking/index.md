# Networking: Stellar Core on Kubernetes

> Production-grade networking guidance for running Stellar Core Validator and RPC workloads with deterministic peer connectivity, low-latency consensus traffic, and explicit east-west isolation.
>
> Tracking: Closes #998

---

## Scope

This module covers:

- Calico and Cilium CNI integration for Stellar Core traffic patterns.
- Multi-cluster and edge routing using ToR-aware BGP design.
- MetalLB and AWS load balancer implementation options.
- Zero-trust network segmentation and mTLS service mesh patterns.
- Troubleshooting and eBPF/kernel tuning for high-throughput ledger sync.

## Document Map

| Topic | File | Primary Audience |
|---|---|---|
| CNI topology and dataplane selection | [topology-cni.md](topology-cni.md) | Platform engineers |
| ToR BGP, multi-cluster edge, load balancers | [bgp-edge-routing.md](bgp-edge-routing.md) | Network/SRE teams |
| Isolation policies and service mesh mTLS | [service-mesh-isolation.md](service-mesh-isolation.md) | Security engineers |
| Command-level troubleshooting and tuning | [troubleshooting-performance.md](troubleshooting-performance.md) | On-call responders |

## Traffic Classes for Stellar-K8s

| Traffic Class | Typical Ports | Latency Sensitivity | Recommended Handling |
|---|---|---|---|
| SCP peer traffic (validator quorum) | `11625/tcp` | Very high | Minimize hops, BGP/direct routing, strict allow-listing |
| Public HTTP API (Horizon/RPC) | `80/443` | Medium | Front with L4/L7 LB, autoscale, WAF where applicable |
| Overlay/operator control plane | `443`, `6443`, `10250` | Medium | Dedicated policy boundaries and audit visibility |
| Metrics/log shipping | `9100`, `9090`, `4317` | Low-Medium | Isolated observability namespace and egress controls |

## Design Goals

1. Keep validator quorum paths predictable and low-jitter.
2. Isolate validator ingress/egress from public RPC traffic.
3. Preserve deterministic failover behavior under node/zone loss.
4. Enforce defense-in-depth with policy + mesh identity + audit.

## Cross-References

- Existing baseline docs: [../network-topology-management.md](../network-topology-management.md), [../metallb-bgp-anycast.md](../metallb-bgp-anycast.md), [../service-mesh.md](../service-mesh.md), [../mtls-guide.md](../mtls-guide.md)
- Security hardening tie-in: [../security/incident-response-playbook.md](../security/incident-response-playbook.md)
- DR tie-in: [../deployment-patterns/multi-region-dr.md](../deployment-patterns/multi-region-dr.md)
# Network Configuration & CNI/BGP Integration Guide

This guide describes the network architecture, container network interface (CNI) configuration, border gateway protocol (BGP) routing, and service mesh integration for deploying **Stellar-K8s** in high-performance environments.

---

## 1. Network Architecture & Topology

Stellar Core nodes communicate using a custom peer-to-peer (P2P) protocol over TCP port `11625`. The Rest API (Horizon) and Soroban RPC nodes communicate over HTTP/HTTPS (ports `8000` and `8080`).

### 1.1 Cluster Traffic Topology
```text
               ┌──────────────────────────────────────────┐
               │              Internet                    │
               └──────────────────┬───────────────────────┘
                                  │
                                  │ (Port 11625 TCP P2P)
                                  ▼
                    ┌────────────────────────────┐
                    │      MetalLB / Load        │
                    │         Balancer           │
                    └─────────────┬──────────────┘
                                  │
         ┌────────────────────────┼────────────────────────┐
         │ (Port 11625 TCP)       │ (Port 8000/8080 HTTP)  │ (Prometheus Scraping)
         ▼                        ▼                        ▼
┌──────────────────┐     ┌──────────────────┐     ┌──────────────────┐
│   Stellar Core   │     │   Horizon API    │     │    Prometheus    │
│  (Validator Pod) │     │    (RPC Pod)     │     │      Server      │
└──────────────────┘     └──────────────────┘     └──────────────────┘
```

---

## 2. CNI Plugin Integration

Stellar-K8s supports advanced networking through industry-standard CNIs.

### 2.1 Calico CNI
Calico provides high-performance networking and rich network security policies using standard Linux iptables or IPVS.
- **Configuring Multi-Cluster Networking**: Enable Calico's IPpool encapsulation (VXLAN or IP-in-IP) for cross-subnet overlay routing.
- **GlobalNetworkPolicy**: Define global policies to allow Stellar P2P ports across all namespaces:
  ```yaml
  apiVersion: projectcalico.org/v3
  kind: GlobalNetworkPolicy
  metadata:
    name: allow-stellar-p2p
  spec:
    selector: app.kubernetes.io/name == 'stellar-node'
    types:
      - Ingress
      - Egress
    ingress:
      - action: Allow
        protocol: TCP
        destination:
          ports: [11625]
    egress:
      - action: Allow
        protocol: TCP
        destination:
          ports: [11625]
  ```

### 2.2 Cilium CNI
Cilium uses eBPF (Extended Berkeley Packet Filter) to route and secure network packets directly in the Linux kernel without iptables overhead.
- **eBPF-based Host Routing**: Enables lower latency and higher throughput, crucial for Validator synchronization.
- **CiliumNetworkPolicy**: Standard policy limiting ingress to authorized endpoints:
  ```yaml
  apiVersion: "cilium.io/v2"
  kind: CiliumNetworkPolicy
  metadata:
    name: secure-validator-p2p
    namespace: stellar
  spec:
    endpointSelector:
      matchLabels:
        app.kubernetes.io/component: stellar-validator
    ingress:
    - fromEndpoints:
      - matchLabels:
          app.kubernetes.io/component: stellar-validator
      toPorts:
      - ports:
        - port: "11625"
          protocol: TCP
  ```

---

## 3. BGP Configuration for Multi-Cluster Networking

BGP (Border Gateway Protocol) allows the Kubernetes cluster nodes to advertise Pod and Service IP blocks directly to the physical network routers.

### 3.1 Calico BGP Configuration
To configure BGP peering with external top-of-rack (ToR) switches:
```yaml
apiVersion: projectcalico.org/v3
kind: BGPPeer
metadata:
  name: tor-switch-peer
spec:
  peerIP: 192.168.1.1
  asNumber: 65001
```

### 3.2 MetalLB BGP Mode Configuration
MetalLB implements load balancers in bare-metal clusters. In BGP mode, MetalLB establishes a BGP session with the router to advertise the LoadBalancer IP.
```yaml
apiVersion: metallb.io/v1beta2
kind: BGPPeer
metadata:
  name: core-router
  namespace: metallb-system
spec:
  peerAddress: 10.0.0.1
  peerASN: 64512
  myASN: 64513
---
apiVersion: metallb.io/v1beta1
kind: IPAddressPool
metadata:
  name: stellar-ips
  namespace: metallb-system
spec:
  addresses:
    - 192.168.10.100-192.168.10.120
---
apiVersion: metallb.io/v1beta1
kind: BGPAdvertisement
metadata:
  name: advertise-stellar-ips
  namespace: metallb-system
spec:
  ipAddressPools:
    - stellar-ips
```

---

## 4. Load Balancer Integration

### 4.1 MetalLB (Bare-Metal)
- Set up MetalLB in either Layer 2 mode (ARP-based) or BGP mode as shown above.
- In Layer 2 mode, ensure that `kube-proxy` config has `strictARP: true` enabled.

### 4.2 Cloud Provider Load Balancers
For AWS deployments, use the AWS Load Balancer Controller to provision Network Load Balancers (NLBs) for low-latency TCP routing:
```yaml
apiVersion: v1
kind: Service
metadata:
  name: validator-p2p
  namespace: stellar
  annotations:
    service.beta.kubernetes.io/aws-load-balancer-type: "external"
    service.beta.kubernetes.io/aws-load-balancer-nlb-target-type: "ip"
    service.beta.kubernetes.io/aws-load-balancer-scheme: "internet-facing"
spec:
  type: LoadBalancer
  selector:
    app.kubernetes.io/name: stellar-node
  ports:
    - port: 11625
      targetPort: 11625
      protocol: TCP
```

---

## 5. Mutual TLS (mTLS) and Service Mesh

Integrating a Service Mesh secures inter-pod communication through mutual TLS (mTLS).

### 5.1 Istio Service Mesh
1. **Enable Sidecar Injection**: Label the namespace to inject Envoy proxies automatically:
   ```bash
   kubectl label namespace stellar istio-injection=enabled
   ```
2. **Enforce Strict mTLS**:
   ```yaml
   apiVersion: security.istio.io/v1beta1
   kind: PeerAuthentication
   metadata:
     name: default
     namespace: stellar
   spec:
     mtls:
       mode: STRICT
   ```

### 5.2 Linkerd Service Mesh
1. Inject the Linkerd proxy by adding the annotation to your `StellarNode` spec metadata:
   ```yaml
   spec:
     metadata:
       annotations:
         linkerd.io/inject: enabled
   ```

---

## 6. Network Performance Tuning

To optimize network throughput and reduce latency for high-speed blockchain state sync:
1. **TCP Socket Buffers**: Increase sysctl socket memory allocation limits on the host nodes:
   ```bash
   sysctl -w net.core.rmem_max=16777216
   sysctl -w net.core.wmem_max=16777216
   ```
2. **Cilium eBPF Host Routing**: Skip standard iptables connection tracking overhead using Cilium's direct routing model:
   ```bash
   helm upgrade cilium cilium/cilium --set bpf.masquerade=true --set hostServices.enabled=true
   ```

---

## 7. Troubleshooting and Common Issues

Refer to the [Networking Troubleshooting Guide](../troubleshooting/networking.md) for step-by-step diagnostic actions for:
- `Connection Refused`
- `No Route to Host`
- DNS Resolution Failures
- CNI Status checks
- mTLS handshake errors

---

## 8. Network Monitoring & Metrics

### 8.1 Prometheus ServiceMonitor
Create a ServiceMonitor to collect network performance metrics from the nodes:
```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: stellar-node-monitor
  namespace: stellar
spec:
  selector:
    matchLabels:
      app.kubernetes.io/name: stellar-node
  endpoints:
  - port: metrics
    interval: 10s
```

### 8.2 Recommended Grafana Panels
- **Active Connections**: Track the total number of connected P2P peers.
- **Network I/O Bytes**: Inbound and outbound bandwidth utilization.
- **Packet Retransmission Rate**: High values indicate packet loss and potential CNI/network congestion.
