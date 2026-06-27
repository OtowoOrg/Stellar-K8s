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
