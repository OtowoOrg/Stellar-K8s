# Deployment Patterns: Reference Architectures

> Production topology patterns for Stellar Core validator and RPC operations on Kubernetes, with explicit guidance for anti-double-signing controls, DR, and GitOps rollout safety.
>
> Tracking: Closes #1000

---

## Contents

| Pattern | File | Focus |
|---|---|---|
| HA validator architecture | [validator-ha-topology.md](validator-ha-topology.md) | Single-active enforcement and quorum safety |
| Horizon/RPC farm architecture | [horizon-rpc-farm.md](horizon-rpc-farm.md) | Horizontal scale and API resilience |
| Multi-region and DR | [multi-region-dr.md](multi-region-dr.md) | Quorum latency and restore playbooks |
| GitOps, capacity, upgrades | [gitops-scaling-upgrades.md](gitops-scaling-upgrades.md) | Terraform/Helm templates and release strategy |

## Architecture Principles

1. Validators prioritize deterministic consensus over elastic scaling.
2. RPC tiers absorb user load and isolate public traffic from validator planes.
3. Region failover should preserve quorum assumptions and recovery point targets.
4. All rollouts are policy-gated and metric-validated.
