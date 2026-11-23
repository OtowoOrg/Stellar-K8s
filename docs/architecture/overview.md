# Architecture Overview

The Stellar-K8s architecture is centered on a Kubernetes-native operator that manages Stellar Core clusters through `StellarNode` custom resources.

## Core Components

- `stellar-operator` deployment: control plane, reconciliation loop, REST API, metrics, and health probes.
- `StellarNode` CRD: defines cluster topology, network settings, storage, and runtimes.
- Admission webhook: validates and mutates incoming `StellarNode` resources.
- Managed workloads: Stellar Core pods, optional sidecars, and service mesh proxies.
- Monitoring stack: Prometheus scrapes operator, webhook, and workload metrics.
- Schema registry: versioned store for all internal protobuf/JSON service schemas, with a compatibility CLI that gates merges by diffing each change against every registered consumer.
- Orphan reconciler: namespace-scoped controller that detects and reclaims orphaned Job/CronJob artifacts (completed pods, failed jobs, deleted-owner remnants) under a configurable retention policy CR.
- Failover plan controller: orchestrates multi-region failover and failback through a controller-owned plan CR, shifting traffic via weighted DNS records gated on health evidence.
- In-cluster feature-flag evaluator: evaluates flags in-process from a signed, cached bundle refreshed asynchronously, with a full audit trail of user-affecting decisions and a kill-switch path.

## Integration Points

- Kubernetes API Server: stores CRDs and orchestrates pod lifecycle.
- External systems: CI/CD, dashboards, automation tools, and observability platforms.
- Optional service mesh: provides mTLS and traffic policy enforcement for Stellar workloads.
- Schema registry and compatibility CLI: integrated into the build pipeline and client codegen; CI blocks breaking schema changes without an explicit override and emits a consumer impact report per change.
- DNS weighted records and health checking: consumed by the failover plan controller to perform declarative, auditable traffic shifts.
- Flag management platform and bundle signer: source of the signed flag bundle consumed by the in-cluster evaluator.

## Design Principles

- Declarative infrastructure through CRDs.
- Strong validation with admission webhooks.
- Observability through Prometheus-compatible metrics.
- Stable upgrades through versioned API and documentation.
- Separation of control plane (`stellar-operator`) and data plane (managed Stellar workloads).
- Schema compatibility enforced at build and deploy time, not just against the previous version.
- Retention and reconciliation policy expressed as CRs rather than hardcoded TTLs.
- Traffic shifts and failback driven by declarative, health-gated plans with operator-readable status.
- Feature-flag evaluation kept off the network hot path, with every user-affecting decision auditable.

## Architecture Goals

- Make Stellar Core easier to run on Kubernetes.
- Provide clear, version-controlled architecture documentation.
- Enable operations teams with runbooks, monitoring guidance, and incident response procedures.
- Guarantee zero unapproved breaking schema changes and 100% internal schema registration.
- Eliminate orphaned Job/CronJob artifacts without premature deletion of active work.
- Meet declared RTO/RPO targets during regional failover drills with zero user-visible errors.
- Deliver sub-millisecond feature-flag evaluation with a complete targeting audit trail.